use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::Digest;

use super::archive::{
    Committed, Copier, EntryRow, commit_archive, finish_restore, layout_rel, list_tree, local_date,
    manifest_hash, remove_tmp, with_suffix,
};
use super::handler::{Cancelled, ItemCx, JobCx, Outcome, Runner};
use crate::destinations::Destination;
use crate::destinations::rclone::{
    Endpoint, ListItem, RcClient, RcloneConfig, open_client, parse_config, parse_uri,
    prepare_remote,
};
use crate::policy::{ItemKind, PolicyEnv};
use crate::store::{get_meta, now_ms};
use crate::vault::SecretStore;

const CHUNK: usize = 1 << 20;
const POLL: Duration = Duration::from_millis(150);
const DROPBOX_BLOCK: usize = 4 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteHash {
    Sha256,
    Sha1,
    Md5,
    Dropbox,
}

impl RemoteHash {
    pub fn name(self) -> &'static str {
        match self {
            RemoteHash::Sha256 => "sha256",
            RemoteHash::Sha1 => "sha1",
            RemoteHash::Md5 => "md5",
            RemoteHash::Dropbox => "dropbox",
        }
    }

    // Strongest first; crc32 and vendor-only hashes are left to the download compare.
    pub fn pick(supported: &[String]) -> Option<RemoteHash> {
        [
            RemoteHash::Sha256,
            RemoteHash::Sha1,
            RemoteHash::Md5,
            RemoteHash::Dropbox,
        ]
        .into_iter()
        .find(|h| supported.iter().any(|s| s.eq_ignore_ascii_case(h.name())))
    }
}

enum Extra {
    Sha256(sha2::Sha256),
    Sha1(sha1::Sha1),
    Md5(md5::Md5),
    Dropbox {
        outer: sha2::Sha256,
        block: sha2::Sha256,
        filled: usize,
    },
}

impl Extra {
    fn new(h: RemoteHash) -> Extra {
        match h {
            RemoteHash::Sha256 => Extra::Sha256(sha2::Sha256::new()),
            RemoteHash::Sha1 => Extra::Sha1(sha1::Sha1::new()),
            RemoteHash::Md5 => Extra::Md5(md5::Md5::new()),
            RemoteHash::Dropbox => Extra::Dropbox {
                outer: sha2::Sha256::new(),
                block: sha2::Sha256::new(),
                filled: 0,
            },
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        match self {
            Extra::Sha256(h) => h.update(data),
            Extra::Sha1(h) => h.update(data),
            Extra::Md5(h) => h.update(data),
            Extra::Dropbox {
                outer,
                block,
                filled,
            } => {
                while !data.is_empty() {
                    let take = (DROPBOX_BLOCK - *filled).min(data.len());
                    block.update(&data[..take]);
                    *filled += take;
                    data = &data[take..];
                    if *filled == DROPBOX_BLOCK {
                        outer.update(block.finalize_reset());
                        *filled = 0;
                    }
                }
            }
        }
    }

    fn finish(self) -> String {
        match self {
            Extra::Sha256(h) => hex(&h.finalize()),
            Extra::Sha1(h) => hex(&h.finalize()),
            Extra::Md5(h) => hex(&h.finalize()),
            Extra::Dropbox {
                mut outer,
                block,
                filled,
            } => {
                if filled > 0 {
                    outer.update(block.finalize());
                }
                hex(&outer.finalize())
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[derive(Debug, Clone)]
pub(crate) struct LocalFile {
    // Windows-style relative path, as archive manifests use.
    pub rel: String,
    pub size: u64,
    pub blake3: String,
    pub extra: Option<String>,
    pub mtime: Option<SystemTime>,
}

fn hash_local(
    path: &Path,
    kind: Option<RemoteHash>,
    cancel: &AtomicBool,
) -> anyhow::Result<(u64, String, Option<String>, Option<SystemTime>)> {
    let mut f = hoardini_win::open_read_preserving_atime(path)?;
    let mut b3 = blake3::Hasher::new();
    let mut extra = kind.map(Extra::new);
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        b3.update(&buf[..n]);
        if let Some(e) = extra.as_mut() {
            e.update(&buf[..n]);
        }
        total += n as u64;
    }
    let mtime = f.metadata().ok().and_then(|m| m.modified().ok());
    Ok((
        total,
        b3.finalize().to_hex().to_string(),
        extra.map(Extra::finish),
        mtime,
    ))
}

pub(crate) fn scan(
    src: &Path,
    kind: ItemKind,
    hash: Option<RemoteHash>,
    cancel: &AtomicBool,
) -> anyhow::Result<(Vec<LocalFile>, usize)> {
    let (rels, links) = match kind {
        ItemKind::File => (vec![String::new()], 0),
        ItemKind::Dir => {
            let t = list_tree(src)?;
            (t.files, t.links)
        }
        ItemKind::Link => anyhow::bail!("a link holds no data; delete it instead"),
    };
    let mut out = Vec::with_capacity(rels.len());
    for rel in rels {
        let (path, name) = if rel.is_empty() {
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (src.to_path_buf(), name)
        } else {
            (src.join(&rel), rel)
        };
        let (size, blake3, extra, mtime) = hash_local(&path, hash, cancel)?;
        out.push(LocalFile {
            rel: name,
            size,
            blake3,
            extra,
            mtime,
        });
    }
    Ok((out, links))
}

pub(crate) fn blake3_of(path: &Path, cancel: &AtomicBool) -> anyhow::Result<(u64, String)> {
    let (size, h, _, _) = hash_local(path, None, cancel)?;
    Ok((size, h))
}

pub(crate) fn manifest_of(files: &[LocalFile]) -> String {
    let entries: Vec<_> = files
        .iter()
        .map(|f| (f.rel.clone(), f.size, f.blake3.clone()))
        .collect();
    manifest_hash(&entries)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Match,
    Mismatch(String),
    // Sizes agree but the remote has no usable hash, so the bytes must be read back.
    NeedDownload,
}

fn slashes(rel: &str) -> String {
    rel.replace('\\', "/")
}

pub(crate) fn compare(
    local: &[LocalFile],
    remote: &[ListItem],
    hash: Option<RemoteHash>,
) -> Verdict {
    let by_path: HashMap<&str, &ListItem> = remote
        .iter()
        .filter(|r| !r.is_dir)
        .map(|r| (r.path.as_str(), r))
        .collect();
    let mut need_download = hash.is_none();
    for f in local {
        let key = slashes(&f.rel);
        let Some(r) = by_path.get(key.as_str()) else {
            return Verdict::Mismatch(format!("{key} is missing at the destination"));
        };
        if r.size != crate::index::to_i64(f.size) {
            return Verdict::Mismatch(format!(
                "{key} is {} bytes at the destination but {} bytes here",
                r.size, f.size
            ));
        }
        let Some(h) = hash else { continue };
        match r.hashes.get(h.name()).filter(|v| !v.is_empty()) {
            None => need_download = true,
            Some(theirs) => {
                let mine = f.extra.as_deref().unwrap_or_default();
                if !theirs.eq_ignore_ascii_case(mine) {
                    return Verdict::Mismatch(format!(
                        "{key}: the destination's {} does not match the source",
                        h.name()
                    ));
                }
            }
        }
    }
    if need_download {
        Verdict::NeedDownload
    } else {
        Verdict::Match
    }
}

pub(crate) fn run_job(
    client: &RcClient,
    method: &str,
    params: Value,
    group: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(i64),
) -> anyhow::Result<Value> {
    let id = client.start(method, params, group)?;
    let mut seen = 0i64;
    let mut report = |client: &RcClient, progress: &mut dyn FnMut(i64)| {
        if let Ok(b) = client.group_bytes(group)
            && b > seen
        {
            progress(b - seen);
            seen = b;
        }
    };
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = client.job_stop(id);
            break Err(Cancelled.into());
        }
        let st = match client.job_status(id) {
            Ok(s) => s,
            Err(e) => break Err(e),
        };
        report(client, progress);
        if st.finished {
            if st.success {
                break Ok(st.output);
            }
            let why = if st.error.is_empty() {
                "unknown error"
            } else {
                &st.error
            };
            break Err(anyhow::anyhow!("rclone {method} failed: {why}"));
        }
        std::thread::sleep(POLL);
    };
    client.stats_reset(group);
    result
}

fn split_parent(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn local_fs(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn copy_params_up(
    cfg: &RcloneConfig,
    src: &Path,
    kind: ItemKind,
    target: &str,
) -> (&'static str, Value) {
    if kind == ItemKind::Dir {
        return (
            "sync/copy",
            json!({"srcFs": local_fs(src), "dstFs": cfg.fs(target), "createEmptySrcDirs": true}),
        );
    }
    let (parent, name) = split_parent(target);
    let src_dir = src.parent().unwrap_or(Path::new(""));
    let src_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    (
        "operations/copyfile",
        json!({"srcFs": local_fs(src_dir), "srcRemote": src_name, "dstFs": cfg.fs(parent), "dstRemote": name}),
    )
}

fn copy_params_down(
    cfg: &RcloneConfig,
    path: &str,
    kind: ItemKind,
    dst: &Path,
) -> (&'static str, Value) {
    if kind == ItemKind::Dir {
        return (
            "sync/copy",
            json!({"srcFs": cfg.fs(path), "dstFs": local_fs(dst), "createEmptySrcDirs": true}),
        );
    }
    let (parent, name) = split_parent(path);
    let dst_dir = dst.parent().unwrap_or(Path::new(""));
    let dst_name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    (
        "operations/copyfile",
        json!({"srcFs": cfg.fs(parent), "srcRemote": name, "dstFs": local_fs(dst_dir), "dstRemote": dst_name}),
    )
}

fn remote_listing(
    client: &RcClient,
    cfg: &RcloneConfig,
    target: &str,
    kind: ItemKind,
    hash: Option<RemoteHash>,
) -> anyhow::Result<Vec<ListItem>> {
    let h = hash.map(RemoteHash::name);
    if kind == ItemKind::Dir {
        return client.list(&cfg.fs(target), true, h);
    }
    let (parent, name) = split_parent(target);
    let all = client.list(&cfg.fs(parent), false, h)?;
    Ok(all.into_iter().filter(|i| i.name == name).collect())
}

fn download_compare(
    client: &RcClient,
    cfg: &RcloneConfig,
    target: &str,
    kind: ItemKind,
    local: &[LocalFile],
    group: &str,
    cancel: &AtomicBool,
) -> anyhow::Result<Verdict> {
    // rclone streams every file back and hashes it, so nothing is stored locally.
    let params = json!({"fs": cfg.fs(target), "hashType": "blake3", "download": true});
    let out = run_job(
        client,
        "operations/hashsum",
        params,
        group,
        cancel,
        &mut |_| {},
    )?;
    let lines: Vec<(&str, &str)> = out["hashsum"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str()?.split_once("  "))
                .collect()
        })
        .unwrap_or_default();
    for f in local {
        let theirs = if kind == ItemKind::File {
            lines.first().map(|(h, _)| *h)
        } else {
            let key = slashes(&f.rel);
            lines.iter().find(|(_, p)| *p == key).map(|(h, _)| *h)
        };
        match theirs {
            None => {
                return Ok(Verdict::Mismatch(format!(
                    "{} did not come back from the destination",
                    f.rel
                )));
            }
            Some(h) if !h.eq_ignore_ascii_case(&f.blake3) => {
                return Ok(Verdict::Mismatch(format!(
                    "{}: the bytes read back from the destination differ",
                    f.rel
                )));
            }
            Some(_) => {}
        }
    }
    Ok(Verdict::Match)
}

struct RcloneRunner {
    dest_id: String,
    cfg: RcloneConfig,
    client: RcClient,
    hash: Option<RemoteHash>,
    day: String,
    stubs: bool,
    env: PolicyEnv,
}

pub(crate) fn begin(job: &JobCx<'_>, dest: &Destination) -> anyhow::Result<Box<dyn Runner>> {
    let cfg = parse_config(&dest.config)?;
    let client = open_client(job.secrets, job.rc.as_ref())?;
    prepare_remote(&client, job.secrets, &cfg)?;
    let need: i64 = job.items.iter().map(|i| i.logical).sum();
    if let Some(free) = client.about(&cfg.root_fs()).ok().and_then(|a| a.free)
        && free < need
    {
        anyhow::bail!(
            "{} has {} free; this archive needs {}",
            dest.name,
            crate::paths::humanize_bytes(crate::index::to_u64(free)),
            crate::paths::humanize_bytes(crate::index::to_u64(need))
        );
    }
    let hash = client
        .hash_types(&cfg.root_fs())
        .ok()
        .and_then(|h| RemoteHash::pick(&h));
    let stubs = get_meta(job.conn, "archive_stubs")?.is_none_or(|v| v.trim() != "false");
    Ok(Box::new(RcloneRunner {
        dest_id: dest.id.clone(),
        cfg,
        client,
        hash,
        day: local_date(now_ms()),
        stubs,
        env: job.env.clone(),
    }))
}

fn unchanged(src: &Path, kind: ItemKind, local: &[LocalFile]) -> anyhow::Result<()> {
    for f in local {
        let p = if kind == ItemKind::File {
            src.to_path_buf()
        } else {
            src.join(&f.rel)
        };
        let md = std::fs::metadata(&p)?;
        if md.len() != f.size || md.modified().ok() != f.mtime {
            anyhow::bail!(
                "{} changed while it was being archived; the source was kept",
                p.display()
            );
        }
    }
    if kind == ItemKind::Dir && list_tree(src)?.files.len() != local.len() {
        anyhow::bail!(
            "files were added to {} while it was being archived; the source was kept",
            src.display()
        );
    }
    Ok(())
}

impl RcloneRunner {
    fn discard_remote(&self, target: &str, kind: ItemKind) {
        let r = if kind == ItemKind::Dir {
            self.client.purge(&self.cfg.fs(target))
        } else {
            let (parent, name) = split_parent(target);
            self.client.delete_file(&self.cfg.fs(parent), name)
        };
        if let Err(e) = r {
            tracing::warn!(error = %e, target, "could not remove the partial remote copy");
        }
    }

    // A listing failure other than "not found" counts as present, so nothing unknown is ever purged.
    fn remote_exists(&self, target: &str) -> bool {
        let (parent, name) = split_parent(target);
        match self.client.list(&self.cfg.fs(parent), false, None) {
            Ok(items) => items.iter().any(|i| i.name == name),
            Err(e) => !e.to_string().to_lowercase().contains("not found"),
        }
    }
}

impl Runner for RcloneRunner {
    fn run(&mut self, cx: &mut ItemCx<'_>) -> anyhow::Result<Outcome> {
        let item = cx.item;
        let src = PathBuf::from(&item.path);
        let rel = slashes(&layout_rel(&item.path));
        let target = self.cfg.join(&format!("{}/{}", self.day, rel));
        let (local, links) = scan(&src, item.kind, self.hash, cx.cancel)?;
        // Never purge a target that was there before this copy started.
        let preexisting = self.remote_exists(&target);
        let copied = (|| -> anyhow::Result<()> {
            let group = format!("hoardini-a{}", cx.action_id);
            let (method, params) = copy_params_up(&self.cfg, &src, item.kind, &target);
            run_job(
                &self.client,
                method,
                params,
                &group,
                cx.cancel,
                &mut *cx.progress,
            )?;
            let verdict = if local.is_empty() {
                Verdict::Match
            } else {
                let listing =
                    remote_listing(&self.client, &self.cfg, &target, item.kind, self.hash)?;
                match compare(&local, &listing, self.hash) {
                    Verdict::NeedDownload => download_compare(
                        &self.client,
                        &self.cfg,
                        &target,
                        item.kind,
                        &local,
                        &format!("{group}-verify"),
                        cx.cancel,
                    )?,
                    v => v,
                }
            };
            if let Verdict::Mismatch(why) = verdict {
                anyhow::bail!("verification failed: {why}; the source was kept");
            }
            cx.check_cancel()?;
            unchanged(&src, item.kind, &local)?;
            Ok(())
        })();
        if let Err(e) = copied {
            if !preexisting {
                self.discard_remote(&target, item.kind);
            }
            return Err(e);
        }
        let size: u64 = local.iter().map(|f| f.size).sum();
        let hash = if item.kind == ItemKind::File {
            local[0].blake3.clone()
        } else {
            manifest_of(&local)
        };
        let dest_uri = self.cfg.uri(&target);
        let to = Committed {
            dest_id: &self.dest_id,
            dest_uri: &dest_uri,
            size,
            hash: &hash,
            links,
        };
        commit_archive(cx, &to, self.stubs, &self.env)
    }
}

pub(crate) fn restore(
    conn: &Connection,
    e: &EntryRow,
    c: &mut Copier<'_>,
    secrets: &dyn SecretStore,
    ep: Option<&Endpoint>,
) -> anyhow::Result<()> {
    let original = PathBuf::from(&e.original_path);
    if std::fs::symlink_metadata(&original).is_ok() {
        anyhow::bail!("something already exists at {}", e.original_path);
    }
    let dest = crate::destinations::get(conn, &e.destination_id)?
        .ok_or_else(|| anyhow::anyhow!("the destination for this archive was removed"))?;
    let cfg = parse_config(&dest.config)?;
    let (remote, path) = parse_uri(&e.dest_uri)
        .ok_or_else(|| anyhow::anyhow!("{} is not an rclone location", e.dest_uri))?;
    if remote != cfg.remote {
        anyhow::bail!(
            "the archive lives on remote {remote}, but the destination now uses {}",
            cfg.remote
        );
    }
    let client = open_client(secrets, ep)?;
    prepare_remote(&client, secrets, &cfg)?;
    let tmp = with_suffix(&original, ".hoardini-restore");
    remove_tmp(&tmp);
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let result = (|| -> anyhow::Result<()> {
        let group = format!("hoardini-r{}-{}", e.id, uuid::Uuid::now_v7().simple());
        let (method, params) = copy_params_down(&cfg, path, e.kind, &tmp);
        run_job(&client, method, params, &group, c.cancel, &mut *c.progress)?;
        let hash = match e.kind {
            ItemKind::Dir => manifest_of(&scan(&tmp, ItemKind::Dir, None, c.cancel)?.0),
            _ => blake3_of(&tmp, c.cancel)?.1,
        };
        if let Some(expected) = &e.blake3
            && !expected.eq_ignore_ascii_case(&hash)
        {
            anyhow::bail!(
                "the archived copy at {} no longer matches the BLAKE3 recorded when it was archived",
                e.dest_uri
            );
        }
        std::fs::rename(&tmp, &original)?;
        Ok(())
    })();
    if result.is_err() {
        remove_tmp(&tmp);
    }
    result?;
    finish_restore(conn, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lf(rel: &str, size: u64, extra: &str) -> LocalFile {
        LocalFile {
            rel: rel.into(),
            size,
            blake3: "b".into(),
            extra: Some(extra.into()),
            mtime: None,
        }
    }

    fn li(path: &str, size: i64, sha: Option<&str>) -> ListItem {
        ListItem {
            path: path.into(),
            name: path.rsplit('/').next().unwrap().into(),
            size,
            is_dir: false,
            hashes: sha
                .map(|h| [("sha256".to_string(), h.to_string())].into())
                .unwrap_or_default(),
        }
    }

    #[test]
    fn hashes_match_known_vectors() {
        let run = |h, data: &[u8]| {
            let mut e = Extra::new(h);
            e.update(data);
            e.finish()
        };
        assert_eq!(
            run(RemoteHash::Md5, b"abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            run(RemoteHash::Sha1, b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            run(RemoteHash::Sha256, b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // A single short block: dropbox hash is sha256(sha256(data)).
        let inner = sha2::Sha256::digest(b"abc");
        assert_eq!(
            run(RemoteHash::Dropbox, b"abc"),
            hex(&sha2::Sha256::digest(inner))
        );
    }

    #[test]
    fn dropbox_hash_is_split_independent() {
        let data: Vec<u8> = (0..(DROPBOX_BLOCK * 2 + 123))
            .map(|i| (i % 251) as u8)
            .collect();
        let mut whole = Extra::new(RemoteHash::Dropbox);
        whole.update(&data);
        let mut parts = Extra::new(RemoteHash::Dropbox);
        for chunk in data.chunks(1_000_003) {
            parts.update(chunk);
        }
        assert_eq!(whole.finish(), parts.finish());
    }

    #[test]
    fn picks_the_strongest_supported_hash() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            RemoteHash::pick(&s(&["crc32", "md5", "SHA1"])),
            Some(RemoteHash::Sha1)
        );
        assert_eq!(
            RemoteHash::pick(&s(&["sha256", "md5"])),
            Some(RemoteHash::Sha256)
        );
        assert_eq!(RemoteHash::pick(&s(&["crc32", "quickxor"])), None);
    }

    #[test]
    fn compare_demands_every_file_size_and_hash() {
        let local = [lf("a.bin", 3, "aa"), lf("sub/b.bin", 5, "bb")];
        let ok = [li("a.bin", 3, Some("AA")), li("sub/b.bin", 5, Some("bb"))];
        assert_eq!(
            compare(&local, &ok, Some(RemoteHash::Sha256)),
            Verdict::Match
        );
        let missing = [li("a.bin", 3, Some("aa"))];
        assert!(matches!(
            compare(&local, &missing, Some(RemoteHash::Sha256)),
            Verdict::Mismatch(_)
        ));
        let short = [li("a.bin", 3, Some("aa")), li("sub/b.bin", 4, Some("bb"))];
        assert!(matches!(
            compare(&local, &short, Some(RemoteHash::Sha256)),
            Verdict::Mismatch(_)
        ));
        let wrong = [li("a.bin", 3, Some("aa")), li("sub/b.bin", 5, Some("cc"))];
        assert!(matches!(
            compare(&local, &wrong, Some(RemoteHash::Sha256)),
            Verdict::Mismatch(_)
        ));
        let blank = [li("a.bin", 3, Some("aa")), li("sub/b.bin", 5, None)];
        assert_eq!(
            compare(&local, &blank, Some(RemoteHash::Sha256)),
            Verdict::NeedDownload
        );
        assert_eq!(compare(&local, &ok, None), Verdict::NeedDownload);
    }
}
