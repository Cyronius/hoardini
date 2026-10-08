use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

use hoardini_win::{
    DirEntry, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, IO_REPARSE_TAG_MOUNT_POINT,
    IO_REPARSE_TAG_SYMLINK, UsnJournal, UsnRecord, VolumeInfo,
};
use parking_lot::Mutex;

use super::journal::{
    ERROR_JOURNAL_ENTRY_DELETED, ERROR_JOURNAL_NOT_ACTIVE, JournalSource,
    USN_REASON_BASIC_INFO_CHANGE, USN_REASON_CLOSE, USN_REASON_DATA_EXTEND,
    USN_REASON_DATA_OVERWRITE, USN_REASON_DATA_TRUNCATION, USN_REASON_FILE_CREATE,
    USN_REASON_FILE_DELETE, USN_REASON_HARD_LINK_CHANGE, USN_REASON_RENAME_NEW_NAME,
    USN_REASON_RENAME_OLD_NAME, USN_REASON_REPARSE_POINT_CHANGE, USN_REASON_SECURITY_CHANGE,
};
use super::lister::DirLister;
use super::{join_path, normalize_path, split_parent};

const T0: i64 = 1_700_000_000_000;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
const FAKE_CLOUD_TAG: u32 = 0x9000_601A;
const USN_STEP: i64 = 96;
type ListHook = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone, Debug)]
struct Node {
    entry: DirEntry,
    open_alloc: Option<u64>,
    denied: bool,
}

struct Journal {
    id: u64,
    first: i64,
    next: i64,
    records: Vec<UsnRecord>,
    enabled: bool,
    page: usize,
}

struct Inner {
    nodes: BTreeMap<String, Node>,
    next_record: u64,
    listed: Vec<String>,
    opened: Vec<String>,
    volume: Option<VolumeInfo>,
    journal: Journal,
    dir_times: bool,
}

pub struct FakeFs {
    root: String,
    inner: Mutex<Inner>,
    hook: Mutex<Option<ListHook>>,
}

fn key(p: &str) -> String {
    normalize_path(p).to_lowercase()
}

fn ntfs_id(record: u64) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&(record | (1u64 << 48)).to_le_bytes());
    id
}

fn round_alloc(logical: u64) -> u64 {
    logical.div_ceil(4096) * 4096
}

impl Inner {
    fn new_entry(&mut self, name: &str, attrs: u32) -> DirEntry {
        let t = self.tick();
        DirEntry {
            name: name.to_string(),
            attrs,
            reparse_tag: 0,
            alloc: 0,
            logical: 0,
            ctime_ms: t,
            atime_ms: t,
            mtime_ms: t,
            file_id: ntfs_id(self.next_record),
        }
    }

    fn tick(&mut self) -> i64 {
        self.next_record += 1;
        T0 + self.next_record as i64 * 1000
    }

    fn id_of(&self, path: &str) -> [u8; 16] {
        self.nodes
            .get(&key(path))
            .map(|n| n.entry.file_id)
            .unwrap_or_default()
    }

    fn log(&mut self, file_id: [u8; 16], parent_id: [u8; 16], attrs: u32, reason: u32) {
        let usn = self.journal.next;
        self.journal.records.push(UsnRecord {
            usn,
            file_id,
            parent_id,
            reason,
            attrs,
        });
        self.journal.next += USN_STEP;
    }

    // NTFS logs a change, then the same reasons again with CLOSE when the last handle closes.
    fn log_change(&mut self, path: &str, reason: u32) {
        let Some((id, attrs)) = self
            .nodes
            .get(&key(path))
            .map(|n| (n.entry.file_id, n.entry.attrs))
        else {
            return;
        };
        let parent = split_parent(&normalize_path(path))
            .map(|(p, _)| self.id_of(&p))
            .unwrap_or_default();
        self.log(id, parent, attrs, reason);
        self.log(id, parent, attrs, reason | USN_REASON_CLOSE);
    }

    // Adding, removing, or renaming an entry changes the directory's own write time, as on NTFS.
    fn bump_parent(&mut self, path: &str) {
        if !self.dir_times {
            return;
        }
        if let Some((parent, _)) = split_parent(&normalize_path(path)) {
            let t = self.tick();
            if let Some(n) = self.nodes.get_mut(&key(&parent)) {
                n.entry.mtime_ms = t;
            }
        }
    }

    fn ensure_dir(&mut self, root: &str, path: &str) {
        let path = normalize_path(path);
        if self.nodes.contains_key(&key(&path)) || path.eq_ignore_ascii_case(root) {
            return;
        }
        let Some((parent, name)) = split_parent(&path) else {
            return;
        };
        self.ensure_dir(root, &parent);
        let entry = self.new_entry(name, FILE_ATTRIBUTE_DIRECTORY);
        self.nodes.insert(
            key(&path),
            Node {
                entry,
                open_alloc: None,
                denied: false,
            },
        );
        self.log_change(&path, USN_REASON_FILE_CREATE);
        self.bump_parent(&path);
    }

    fn put(&mut self, root: &str, path: &str, reason: u32, f: impl FnOnce(&mut DirEntry)) {
        let path = normalize_path(path);
        let Some((parent, name)) = split_parent(&path) else {
            return;
        };
        self.ensure_dir(root, &parent);
        self.log_change(&path, USN_REASON_FILE_DELETE);
        let mut entry = self.new_entry(name, 0);
        f(&mut entry);
        self.nodes.insert(
            key(&path),
            Node {
                entry,
                open_alloc: None,
                denied: false,
            },
        );
        self.log_change(&path, reason);
        self.bump_parent(&path);
    }

    fn subtree_keys(&self, path: &str) -> Vec<String> {
        let k = key(path);
        let prefix = if k.ends_with('\\') {
            k.clone()
        } else {
            format!("{k}\\")
        };
        let mut out = vec![k];
        out.extend(
            self.nodes
                .range(prefix.clone()..)
                .take_while(|(kk, _)| kk.starts_with(&prefix))
                .map(|(kk, _)| kk.clone()),
        );
        out
    }

    fn remove_subtree(&mut self, path: &str) -> bool {
        let keys: Vec<String> = self
            .subtree_keys(path)
            .into_iter()
            .filter(|k| self.nodes.contains_key(k))
            .collect();
        for k in keys.iter().rev() {
            self.log_change(k, USN_REASON_FILE_DELETE);
        }
        for k in &keys {
            self.nodes.remove(k);
        }
        !keys.is_empty()
    }

    fn real_path(&self, root: &str, k: &str) -> String {
        let mut names = Vec::new();
        let mut cur = k.to_string();
        while !cur.eq_ignore_ascii_case(root) {
            let Some((parent, _)) = split_parent(&cur) else {
                break;
            };
            names.push(
                self.nodes
                    .get(&cur)
                    .map(|n| n.entry.name.clone())
                    .unwrap_or_default(),
            );
            cur = parent;
        }
        names
            .iter()
            .rev()
            .fold(root.to_string(), |p, n| join_path(&p, n))
    }
}

impl FakeFs {
    pub fn new(root: &str) -> FakeFs {
        let root = normalize_path(root);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            key(&root),
            Node {
                entry: DirEntry {
                    name: root.clone(),
                    attrs: FILE_ATTRIBUTE_DIRECTORY,
                    reparse_tag: 0,
                    alloc: 0,
                    logical: 0,
                    ctime_ms: T0,
                    atime_ms: T0,
                    mtime_ms: T0,
                    file_id: ntfs_id(5),
                },
                open_alloc: None,
                denied: false,
            },
        );
        let volume = VolumeInfo {
            root: root.clone(),
            serial: 0x1234,
            fs_name: "NTFS".into(),
            label: "Fake".into(),
            total_bytes: 1 << 40,
            free_bytes: 1 << 38,
        };
        FakeFs {
            root,
            inner: Mutex::new(Inner {
                nodes,
                next_record: 100,
                listed: Vec::new(),
                opened: Vec::new(),
                volume: Some(volume),
                dir_times: false,
                journal: Journal {
                    id: 0x01DC_0000_0000_0001,
                    first: 4096,
                    next: 4096,
                    records: Vec::new(),
                    enabled: true,
                    page: 64,
                },
            }),
            hook: Mutex::new(None),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn set_volume(&self, total_bytes: u64, free_bytes: u64) {
        if let Some(v) = self.inner.lock().volume.as_mut() {
            v.total_bytes = total_bytes;
            v.free_bytes = free_bytes;
        }
    }

    pub fn set_fs_name(&self, fs_name: &str) {
        if let Some(v) = self.inner.lock().volume.as_mut() {
            v.fs_name = fs_name.into();
        }
    }

    // Makes volume_info fail with Unsupported, like the portable fallback lister.
    pub fn clear_volume(&self) {
        self.inner.lock().volume = None;
    }

    pub fn dir(&self, path: &str) {
        self.inner.lock().ensure_dir(&self.root, path);
    }

    pub fn file(&self, path: &str, logical: u64) {
        self.file_alloc(path, round_alloc(logical), logical);
    }

    pub fn file_alloc(&self, path: &str, alloc: u64, logical: u64) {
        self.inner
            .lock()
            .put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
                e.alloc = alloc;
                e.logical = logical;
            });
    }

    pub fn file_with_id(&self, path: &str, alloc: u64, logical: u64, file_id: [u8; 16]) {
        self.inner
            .lock()
            .put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
                e.alloc = alloc;
                e.logical = logical;
                e.file_id = file_id;
            });
    }

    pub fn hard_link(&self, existing: &str, link: &str) {
        let mut g = self.inner.lock();
        let Some(src) = g.nodes.get(&key(existing)).map(|n| n.entry.clone()) else {
            return;
        };
        g.put(&self.root, link, USN_REASON_HARD_LINK_CHANGE, |e| {
            let name = std::mem::take(&mut e.name);
            *e = DirEntry { name, ..src };
        });
    }

    pub fn junction(&self, path: &str) {
        self.reparse_dir(path, IO_REPARSE_TAG_MOUNT_POINT);
    }

    pub fn symlink_dir(&self, path: &str) {
        self.reparse_dir(path, IO_REPARSE_TAG_SYMLINK);
    }

    pub fn symlink_file(&self, path: &str) {
        self.inner
            .lock()
            .put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
                e.attrs = FILE_ATTRIBUTE_REPARSE_POINT;
                e.reparse_tag = IO_REPARSE_TAG_SYMLINK;
            });
    }

    pub fn reparse_dir(&self, path: &str, tag: u32) {
        self.inner
            .lock()
            .put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
                e.attrs = FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT;
                e.reparse_tag = tag;
            });
    }

    // Measured on NTFS: a WOF file lists as alloc 0 with plain attributes and no reparse tag.
    pub fn wof_file(&self, path: &str, logical: u64, real_alloc: u64) {
        let mut g = self.inner.lock();
        g.put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
            e.attrs = FILE_ATTRIBUTE_ARCHIVE;
            e.alloc = 0;
            e.logical = logical;
        });
        if let Some(n) = g.nodes.get_mut(&key(path)) {
            n.open_alloc = Some(real_alloc);
        }
    }

    pub fn sparse_file(&self, path: &str, alloc: u64, logical: u64) {
        self.inner
            .lock()
            .put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
                e.attrs = FILE_ATTRIBUTE_ARCHIVE | FILE_ATTRIBUTE_SPARSE_FILE;
                e.alloc = alloc;
                e.logical = logical;
            });
    }

    pub fn placeholder(&self, path: &str, logical: u64) {
        self.inner
            .lock()
            .put(&self.root, path, USN_REASON_FILE_CREATE, |e| {
                e.attrs = FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;
                e.reparse_tag = FAKE_CLOUD_TAG;
                e.alloc = 0;
                e.logical = logical;
            });
    }

    pub fn set_attrs(&self, path: &str, attrs: u32, reparse_tag: u32) {
        let mut g = self.inner.lock();
        let Some(n) = g.nodes.get_mut(&key(path)) else {
            return;
        };
        let tag_changed = n.entry.reparse_tag != reparse_tag;
        n.entry.attrs = attrs;
        n.entry.reparse_tag = reparse_tag;
        let reason = if tag_changed {
            USN_REASON_REPARSE_POINT_CHANGE
        } else {
            USN_REASON_BASIC_INFO_CHANGE
        };
        g.log_change(path, reason);
    }

    pub fn deny(&self, path: &str) {
        let mut g = self.inner.lock();
        g.ensure_dir(&self.root, path);
        if let Some(n) = g.nodes.get_mut(&key(path)) {
            n.denied = true;
        }
        g.log_change(path, USN_REASON_SECURITY_CHANGE);
    }

    pub fn allow(&self, path: &str) {
        let mut g = self.inner.lock();
        if let Some(n) = g.nodes.get_mut(&key(path)) {
            n.denied = false;
        }
        g.log_change(path, USN_REASON_SECURITY_CHANGE);
    }

    pub fn remove(&self, path: &str) {
        let mut g = self.inner.lock();
        if g.remove_subtree(path) {
            g.bump_parent(path);
        }
    }

    // Only the moved entry is journaled; its subtree keeps its ids and gets no records, as on NTFS.
    pub fn rename(&self, from: &str, to: &str) {
        let mut g = self.inner.lock();
        let from_n = normalize_path(from);
        let to_n = normalize_path(to);
        let Some((to_parent, to_name)) = split_parent(&to_n) else {
            return;
        };
        g.ensure_dir(&self.root, &to_parent);
        let from_k = key(&from_n);
        let to_k = key(&to_n);
        let Some((id, attrs)) = g
            .nodes
            .get(&from_k)
            .map(|n| (n.entry.file_id, n.entry.attrs))
        else {
            return;
        };
        if to_k != from_k && g.remove_subtree(&to_n) {
            g.bump_parent(&to_n);
        }
        let old_parent = split_parent(&from_n)
            .map(|(p, _)| g.id_of(&p))
            .unwrap_or_default();
        g.log(id, old_parent, attrs, USN_REASON_RENAME_OLD_NAME);
        for k in g.subtree_keys(&from_n) {
            if let Some(mut n) = g.nodes.remove(&k) {
                if k == from_k {
                    n.entry.name = to_name.to_string();
                }
                g.nodes.insert(format!("{to_k}{}", &k[from_k.len()..]), n);
            }
        }
        let new_parent = g.id_of(&to_parent);
        g.log(id, new_parent, attrs, USN_REASON_RENAME_NEW_NAME);
        g.log(
            id,
            new_parent,
            attrs,
            USN_REASON_RENAME_NEW_NAME | USN_REASON_CLOSE,
        );
        g.bump_parent(&from_n);
        g.bump_parent(&to_n);
    }

    // Changes every hard link of the file, like a real write would; the record names only this path.
    pub fn resize(&self, path: &str, alloc: u64, logical: u64) {
        let mut g = self.inner.lock();
        let Some((id, old)) = g
            .nodes
            .get(&key(path))
            .map(|n| (n.entry.file_id, n.entry.logical))
        else {
            return;
        };
        let k = key(path);
        for (kk, n) in g.nodes.iter_mut() {
            if *kk == k || (id != [0; 16] && n.entry.file_id == id) {
                n.entry.alloc = alloc;
                n.entry.logical = logical;
                n.entry.mtime_ms += 1;
                if n.open_alloc.is_some() {
                    n.open_alloc = Some(alloc);
                    n.entry.alloc = 0;
                }
            }
        }
        let reason = match logical.cmp(&old) {
            std::cmp::Ordering::Greater => USN_REASON_DATA_EXTEND,
            std::cmp::Ordering::Less => USN_REASON_DATA_TRUNCATION,
            std::cmp::Ordering::Equal => USN_REASON_DATA_OVERWRITE,
        };
        g.log_change(path, reason);
    }

    pub fn touch(&self, path: &str, mtime_ms: i64) {
        let mut g = self.inner.lock();
        if let Some(n) = g.nodes.get_mut(&key(path)) {
            n.entry.mtime_ms = mtime_ms;
        }
        g.log_change(path, USN_REASON_BASIC_INFO_CHANGE);
    }

    pub fn file_id(&self, path: &str) -> Option<[u8; 16]> {
        self.inner
            .lock()
            .nodes
            .get(&key(path))
            .map(|n| n.entry.file_id)
    }

    pub fn listed(&self) -> Vec<String> {
        self.inner.lock().listed.clone()
    }

    pub fn opened(&self) -> Vec<String> {
        self.inner.lock().opened.clone()
    }

    pub fn clear_log(&self) {
        let mut g = self.inner.lock();
        g.listed.clear();
        g.opened.clear();
    }

    pub fn on_list(&self, hook: impl Fn(&str) + Send + Sync + 'static) {
        *self.hook.lock() = Some(Arc::new(hook));
    }

    pub fn clear_hook(&self) {
        *self.hook.lock() = None;
    }

    // A recreated journal: new id, no records (`fsutil usn deletejournal` + createjournal).
    pub fn journal_reset(&self) {
        let mut g = self.inner.lock();
        g.journal.id += 1;
        g.journal.records.clear();
        g.journal.first = g.journal.next;
    }

    // The journal overran its size: every record so far is gone, the id stays.
    pub fn journal_wrap(&self) {
        let mut g = self.inner.lock();
        g.journal.records.clear();
        g.journal.first = g.journal.next;
    }

    // Disabled acts like a volume without a journal (FAT, exFAT).
    pub fn set_journal_enabled(&self, enabled: bool) {
        self.inner.lock().journal.enabled = enabled;
    }

    pub fn set_journal_page(&self, records: usize) {
        self.inner.lock().journal.page = records.max(1);
    }

    // Off by default: action patches never re-list a parent, so they cannot see these times change.
    pub fn track_dir_times(&self, on: bool) {
        self.inner.lock().dir_times = on;
    }

    pub fn journal_records(&self) -> usize {
        self.inner.lock().journal.records.len()
    }
}

impl DirLister for FakeFs {
    fn list_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let p = normalize_path(&path.to_string_lossy());
        let hook = self.hook.lock().clone();
        if let Some(h) = hook {
            h(&p);
        }
        let mut g = self.inner.lock();
        g.listed.push(p.clone());
        let k = key(&p);
        match g.nodes.get(&k) {
            None => return Err(io::ErrorKind::NotFound.into()),
            Some(n) if !n.entry.is_dir() => return Err(io::Error::other("not a directory")),
            Some(n) if n.denied => return Err(io::ErrorKind::PermissionDenied.into()),
            Some(_) => {}
        }
        let prefix = if k.ends_with('\\') {
            k
        } else {
            format!("{k}\\")
        };
        Ok(g.nodes
            .range(prefix.clone()..)
            .take_while(|(kk, _)| kk.starts_with(&prefix))
            .filter(|(kk, _)| kk.len() > prefix.len() && !kk[prefix.len()..].contains('\\'))
            .map(|(_, n)| n.entry.clone())
            .collect())
    }

    fn alloc_size_by_open(&self, path: &Path) -> io::Result<u64> {
        let p = normalize_path(&path.to_string_lossy());
        let mut g = self.inner.lock();
        g.opened.push(p.clone());
        match g.nodes.get(&key(&p)) {
            Some(n) => Ok(n.open_alloc.unwrap_or(n.entry.alloc)),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn volume_info(&self, _root: &Path) -> io::Result<VolumeInfo> {
        self.inner
            .lock()
            .volume
            .clone()
            .ok_or_else(|| io::ErrorKind::Unsupported.into())
    }
}

impl JournalSource for FakeFs {
    fn query(&self) -> io::Result<UsnJournal> {
        let g = self.inner.lock();
        if !g.journal.enabled {
            return Err(io::Error::from_raw_os_error(ERROR_JOURNAL_NOT_ACTIVE));
        }
        Ok(UsnJournal {
            journal_id: g.journal.id,
            first_usn: g.journal.first,
            next_usn: g.journal.next,
        })
    }

    fn read(&self, journal_id: u64, from: i64) -> io::Result<(Vec<UsnRecord>, i64)> {
        let g = self.inner.lock();
        let j = &g.journal;
        if !j.enabled {
            return Err(io::Error::from_raw_os_error(ERROR_JOURNAL_NOT_ACTIVE));
        }
        if journal_id != j.id || from < j.first {
            return Err(io::Error::from_raw_os_error(ERROR_JOURNAL_ENTRY_DELETED));
        }
        let start = j.records.partition_point(|r| r.usn < from);
        let page: Vec<UsnRecord> = j.records[start..].iter().take(j.page).cloned().collect();
        let next = j.records.get(start + page.len()).map_or(j.next, |r| r.usn);
        Ok((page, next))
    }

    fn path_by_file_id(&self, id: [u8; 16]) -> io::Result<String> {
        let g = self.inner.lock();
        let k = g
            .nodes
            .iter()
            .find(|(_, n)| id != [0; 16] && n.entry.file_id == id)
            .map(|(k, _)| k.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(g.real_path(&self.root, &k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_direct_children_only() {
        let fs = FakeFs::new(r"C:\");
        fs.file(r"C:\a\x.bin", 10);
        fs.file(r"C:\a\b\y.bin", 10);
        fs.file(r"C:\a b.txt", 10);
        let names: Vec<String> = fs
            .list_dir(Path::new(r"C:\a"))
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["b", "x.bin"]);
        let root: Vec<String> = fs
            .list_dir(Path::new(r"C:\"))
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(root, vec!["a", "a b.txt"]);
    }

    #[test]
    fn renames_subtrees_and_shares_link_sizes() {
        let fs = FakeFs::new(r"C:\");
        fs.file(r"C:\a\b\y.bin", 10);
        fs.hard_link(r"C:\a\b\y.bin", r"C:\z.bin");
        fs.rename(r"C:\a", r"C:\q\a2");
        assert!(fs.list_dir(Path::new(r"C:\a")).is_err());
        let e = fs.list_dir(Path::new(r"C:\q\a2\b")).unwrap();
        assert_eq!(e[0].name, "y.bin");
        fs.resize(r"C:\z.bin", 8192, 8000);
        assert_eq!(
            fs.list_dir(Path::new(r"C:\q\a2\b")).unwrap()[0].logical,
            8000
        );
        assert_eq!(fs.file_id(r"C:\z.bin"), fs.file_id(r"C:\q\a2\b\y.bin"));
    }

    #[test]
    fn journals_changes_with_parent_ids_and_pages() {
        let fs = FakeFs::new(r"C:\");
        fs.file(r"C:\a\Big.bin", 10);
        let start = fs.query().unwrap();
        fs.rename(r"C:\a", r"C:\q\A2");
        fs.remove(r"C:\q\A2\Big.bin");
        let j = fs.query().unwrap();
        assert_eq!(j.journal_id, start.journal_id);
        fs.set_journal_page(2);
        let mut at = start.next_usn;
        let mut recs = Vec::new();
        while at < j.next_usn {
            let (page, next) = fs.read(j.journal_id, at).unwrap();
            assert!(page.len() <= 2);
            recs.extend(page);
            at = next;
        }
        let a2 = fs.file_id(r"C:\q\A2").unwrap();
        let q = fs.file_id(r"C:\q").unwrap();
        assert!(
            recs.iter()
                .any(|r| r.file_id == q && r.reason & USN_REASON_FILE_CREATE != 0)
        );
        let moved: Vec<&UsnRecord> = recs.iter().filter(|r| r.file_id == a2).collect();
        assert_eq!(moved[0].reason, USN_REASON_RENAME_OLD_NAME);
        assert_eq!(moved[0].parent_id, ntfs_id(5));
        assert_eq!(moved[1].parent_id, q);
        assert!(
            recs.iter()
                .any(|r| r.parent_id == a2 && r.reason & USN_REASON_FILE_DELETE != 0)
        );
        assert_eq!(fs.path_by_file_id(a2).unwrap(), r"C:\q\A2");
        fs.journal_wrap();
        assert!(fs.read(j.journal_id, start.next_usn).is_err());
    }
}
