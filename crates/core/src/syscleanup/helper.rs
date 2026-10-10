use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use hoardini_win::{NoFollow, Stat};

use super::SystemOp;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpResult {
    pub ok: bool,
    pub freed: u64,
    pub error: Option<String>,
    pub detail: Option<String>,
}

impl OpResult {
    pub fn failed(msg: impl Into<String>) -> OpResult {
        OpResult {
            ok: false,
            error: Some(msg.into()),
            ..OpResult::default()
        }
    }
}

fn token(op: &SystemOp) -> String {
    match op {
        SystemOp::ComponentStore => "component-store".into(),
        SystemOp::RestorePoints { volume } => format!("restore-points={volume}"),
        SystemOp::Hibernation => "hibernation-reduced".into(),
        SystemOp::CompactVhdx { path } => format!("compact-vhdx={path}"),
        SystemOp::RecycleBin => "recycle-bin".into(),
        SystemOp::Leftovers => "leftovers".into(),
    }
}

fn quote(arg: &str) -> anyhow::Result<String> {
    // A quote inside, or a backslash right before the closing quote, would change how the child splits it.
    if arg.contains('"') || arg.ends_with('\\') || arg.is_empty() {
        bail!("cannot pass {arg:?} to the elevated helper");
    }
    Ok(format!("\"{arg}\""))
}

pub fn helper_params(result: &Path, ops: &[SystemOp]) -> anyhow::Result<String> {
    let mut parts = vec![
        "system-cleanup".to_string(),
        "--result".into(),
        quote(&result.to_string_lossy())?,
    ];
    for op in ops {
        if !op.elevated() {
            bail!("{} does not run in the elevated helper", op.name());
        }
        parts.push(quote(&token(op))?);
    }
    Ok(parts.join(" "))
}

fn volume_ok(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

// Everything the elevated side runs is decided here, from a fixed list; nothing else is accepted.
pub fn parse_helper_args(args: &[String]) -> anyhow::Result<(PathBuf, Vec<SystemOp>)> {
    let mut it = args.iter();
    let (Some(flag), Some(result)) = (it.next(), it.next()) else {
        bail!("usage: system-cleanup --result <file> <operation>...");
    };
    if flag != "--result" || !Path::new(result).is_absolute() {
        bail!("usage: system-cleanup --result <absolute file> <operation>...");
    }
    let mut ops = Vec::new();
    for a in it {
        let op = match a.split_once('=') {
            None if a == "component-store" => SystemOp::ComponentStore,
            None if a == "hibernation-reduced" => SystemOp::Hibernation,
            None if a == "leftovers" => SystemOp::Leftovers,
            Some(("restore-points", v)) if volume_ok(v) => SystemOp::RestorePoints {
                volume: v.to_ascii_uppercase(),
            },
            Some(("compact-vhdx", p))
                if Path::new(p).is_absolute()
                    && !p.starts_with(r"\\")
                    && p.to_ascii_lowercase().ends_with(".vhdx") =>
            {
                SystemOp::CompactVhdx { path: p.into() }
            }
            _ => bail!("unknown or invalid operation {a:?}"),
        };
        ops.push(op);
    }
    Ok((PathBuf::from(result), ops))
}

// Shadow volume device paths are not localized, unlike the rest of vssadmin's output.
pub fn count_shadows(listing: &str) -> usize {
    listing
        .lines()
        .filter(|l| {
            l.to_ascii_lowercase()
                .contains(r"\device\harddiskvolumeshadowcopy")
        })
        .count()
}

fn tool(name: &str) -> anyhow::Result<Command> {
    let mut c = Command::new(hoardini_win::system_tool(name)?);
    c.creation_flags(CREATE_NO_WINDOW).stdin(Stdio::null());
    Ok(c)
}

fn run(mut c: Command, what: &str) -> anyhow::Result<std::process::Output> {
    let out = c
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("could not start {what}"))?;
    Ok(out)
}

fn exit_error(what: &str, out: &std::process::Output) -> anyhow::Error {
    let text = String::from_utf8_lossy(&out.stdout);
    let last = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("");
    anyhow::anyhow!("{what} exited with {}: {last}", out.status)
}

fn component_store() -> anyhow::Result<Option<String>> {
    let mut c = tool("Dism.exe")?;
    c.args([
        "/Online",
        "/Cleanup-Image",
        "/StartComponentCleanup",
        "/Quiet",
        "/NoRestart",
    ]);
    let out = run(c, "DISM")?;
    match out.status.code() {
        Some(0) => Ok(None),
        Some(3010) => Ok(Some("Windows asks for a restart to finish".into())),
        _ => Err(exit_error("DISM", &out)),
    }
}

fn restore_points(volume: &str) -> anyhow::Result<Option<String>> {
    let mut list = tool("vssadmin.exe")?;
    list.args(["list", "shadows", &format!("/for={volume}")]);
    let out = run(list, "vssadmin")?;
    let n = count_shadows(&String::from_utf8_lossy(&out.stdout));
    if n <= 1 {
        let what = if n == 0 {
            "there were no restore points"
        } else {
            "only the newest restore point was there"
        };
        return Ok(Some(what.into()));
    }
    for _ in 1..n {
        let mut del = tool("vssadmin.exe")?;
        del.args([
            "delete",
            "shadows",
            &format!("/for={volume}"),
            "/oldest",
            "/quiet",
        ]);
        let out = run(del, "vssadmin")?;
        if !out.status.success() {
            return Err(exit_error("vssadmin delete shadows", &out));
        }
    }
    Ok(Some(format!("{} older restore points deleted", n - 1)))
}

fn hibernation() -> anyhow::Result<Option<String>> {
    let mut c = tool("powercfg.exe")?;
    c.args(["/h", "/type", "reduced"]);
    let out = run(c, "powercfg")?;
    if !out.status.success() {
        return Err(exit_error("powercfg", &out));
    }
    Ok(None)
}

// Detach runs on its own, so a failed compact never leaves the disk attached.
pub fn diskpart_scripts(path: &str) -> anyhow::Result<(String, String)> {
    if path.chars().any(|c| c == '"' || c.is_control()) {
        bail!("cannot hand {path:?} to diskpart");
    }
    Ok((
        format!("select vdisk file=\"{path}\"\nattach vdisk readonly\ncompact vdisk\n"),
        format!("select vdisk file=\"{path}\"\ndetach vdisk\n"),
    ))
}

// Users may not rename or delete inside Windows\Temp and only admins may write this folder, so no unelevated process can swap a script.
fn private_script_dir() -> anyhow::Result<PathBuf> {
    let dir = hoardini_win::windows_directory()?
        .join("Temp")
        .join(format!(
            "hoardini-diskpart-{}",
            uuid::Uuid::now_v7().simple()
        ));
    hoardini_win::create_private_dir(&dir)
        .with_context(|| format!("could not create {}", dir.display()))?;
    Ok(dir)
}

// diskpart reports a failed command in its exit code only when it runs a script file (/s).
fn diskpart(script: &str, dir: &Path, name: &str) -> anyhow::Result<std::process::Output> {
    let file = dir.join(name);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)?
        .write_all(script.as_bytes())?;
    let mut c = tool("diskpart.exe")?;
    c.arg("/s").arg(&file);
    run(c, "diskpart")
}

fn compact_vhdx(path: &str) -> anyhow::Result<Option<String>> {
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        bail!("{path} is not there anymore");
    }
    let (compact_script, detach_script) = diskpart_scripts(path)?;
    let dir = private_script_dir()?;
    let compact = diskpart(&compact_script, &dir, "compact.txt");
    let _ = diskpart(&detach_script, &dir, "detach.txt");
    let _ = std::fs::remove_dir_all(&dir);
    let out = compact?;
    if !out.status.success() {
        return Err(exit_error(
            "diskpart (is WSL or Docker Desktop still using the disk?)",
            &out,
        ));
    }
    Ok(None)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cleared {
    pub files: u64,
    pub bytes: u64,
    pub in_use: u64,
    pub recent: u64,
    pub failed: u64,
}

impl Cleared {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} file{} deleted ({})",
            self.files,
            if self.files == 1 { "" } else { "s" },
            crate::paths::humanize_bytes(self.bytes)
        );
        let left: Vec<String> = [
            (self.in_use, "in use"),
            (self.recent, "newer than a day"),
            (self.failed, "could not be deleted"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| format!("{n} {what}"))
        .collect();
        if !left.is_empty() {
            s.push_str(&format!("; left: {}", left.join(", ")));
        }
        s
    }
}

// Installers unpack files with an old last write; the creation or change time shows when they arrived.
fn recent(st: &Stat, cutoff_ms: i64) -> bool {
    st.newest_ms() > cutoff_ms
}

fn clear_file(file: &NoFollow, root: &str, cutoff_ms: i64, out: &mut Cleared) {
    let Ok(st) = file.stat() else {
        out.failed += 1;
        return;
    };
    if st.is_reparse() || st.is_dir() {
        return;
    }
    if recent(&st, cutoff_ms) {
        out.recent += 1;
        return;
    }
    if !file.is_inside(root) {
        out.failed += 1;
        return;
    }
    match file.delete() {
        Ok(()) => {
            out.files += 1;
            out.bytes += st.size;
        }
        Err(_) => out.failed += 1,
    }
}

#[cfg(test)]
type ListHook = Option<Box<dyn Fn(&str)>>;

#[cfg(test)]
thread_local! {
    static AFTER_LIST: std::cell::RefCell<ListHook> = const { std::cell::RefCell::new(None) };
}

// Every child is opened through its parent's handle, so a folder swapped for a junction mid-walk is held, not followed.
fn clear_dir(dir: &NoFollow, root: &str, cutoff_ms: i64, out: &mut Cleared) {
    let mut entries = Vec::new();
    if dir.list(|e| entries.push(e)).is_err() {
        out.failed += 1;
        return;
    }
    #[cfg(test)]
    AFTER_LIST.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(&dir.final_path().unwrap_or_default())
        }
    });
    let gone = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    for e in entries {
        if e.is_reparse() {
            continue;
        }
        if e.is_dir() {
            let sub = match dir.child_dir(&e.name) {
                Ok(s) => s,
                Err(err) if gone(&err) => continue,
                Err(_) => {
                    out.failed += 1;
                    continue;
                }
            };
            let Ok(st) = sub.stat() else {
                out.failed += 1;
                continue;
            };
            if st.is_reparse() || !st.is_dir() {
                continue;
            }
            clear_dir(&sub, root, cutoff_ms, out);
            if !recent(&st, cutoff_ms) && sub.is_inside(root) {
                let _ = sub.delete();
            }
            continue;
        }
        // A fresh file someone holds open counts as recent, not in use.
        if e.ctime_ms.max(e.mtime_ms) > cutoff_ms {
            out.recent += 1;
            continue;
        }
        match dir.child_for_delete(&e.name) {
            Ok(Some(f)) => clear_file(&f, root, cutoff_ms, out),
            Ok(None) => out.in_use += 1,
            Err(err) if gone(&err) => {}
            Err(_) => out.failed += 1,
        }
    }
}

// Contents only: the folder itself stays, links inside are never followed or removed.
pub fn clear_contents(target: &Path, cutoff_ms: i64, out: &mut Cleared) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(target)?;
    if md.file_type().is_symlink() {
        return Ok(());
    }
    if md.is_dir() {
        let dir = NoFollow::open_dir(target)?;
        let st = dir.stat()?;
        if st.is_reparse() || !st.is_dir() {
            return Ok(());
        }
        clear_dir(&dir, &dir.final_path()?, cutoff_ms, out);
        return Ok(());
    }
    match NoFollow::open_for_delete(target)? {
        Some(f) => {
            let st = f.stat()?;
            if st.is_reparse() || st.is_dir() {
                return Ok(());
            }
            if recent(&st, cutoff_ms) {
                out.recent += 1;
                return Ok(());
            }
            match f.delete() {
                Ok(()) => {
                    out.files += 1;
                    out.bytes += st.size;
                }
                Err(_) => out.failed += 1,
            }
        }
        None => out.in_use += 1,
    }
    Ok(())
}

fn leftovers() -> anyhow::Result<Option<String>> {
    let windows = hoardini_win::windows_directory()?;
    let program_data = hoardini_win::program_data_directory()?;
    let cutoff = crate::store::now_ms() - super::LEFTOVER_AGE_HOURS * 3_600_000;
    let mut out = Cleared::default();
    for t in super::leftover_targets(&windows.to_string_lossy(), &program_data.to_string_lossy()) {
        if let Err(e) = clear_contents(Path::new(&t), cutoff, &mut out)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            out.failed += 1;
        }
    }
    Ok(Some(out.summary()))
}

fn free_bytes(volume: &str) -> Option<u64> {
    hoardini_win::volume_info(Path::new(volume))
        .ok()
        .map(|v| v.free_bytes)
}

fn run_op(op: &SystemOp, system_drive: &str) -> OpResult {
    let volume = op.volume(system_drive);
    let before = free_bytes(&volume);
    let res = match op {
        SystemOp::ComponentStore => component_store(),
        SystemOp::RestorePoints { volume } => restore_points(volume),
        SystemOp::Hibernation => hibernation(),
        SystemOp::CompactVhdx { path } => compact_vhdx(path),
        SystemOp::Leftovers => leftovers(),
        SystemOp::RecycleBin => Err(anyhow::anyhow!("the Recycle Bin is emptied as the user")),
    };
    let freed = match (before, free_bytes(&volume)) {
        (Some(b), Some(a)) => a.saturating_sub(b),
        _ => 0,
    };
    match res {
        Ok(detail) => OpResult {
            ok: true,
            freed,
            error: None,
            detail,
        },
        Err(e) => OpResult {
            freed,
            ..OpResult::failed(format!("{e:#}"))
        },
    }
}

pub fn run_helper(args: &[String]) -> i32 {
    let (result, ops) = match parse_helper_args(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e:#}");
            return 2;
        }
    };
    let mut out = match std::fs::File::create(&result) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot write {}: {e}", result.display());
            return 2;
        }
    };
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let mut all_ok = true;
    for op in &ops {
        let r = run_op(op, &drive);
        all_ok &= r.ok;
        if let Ok(line) = serde_json::to_string(&r) {
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    }
    if all_ok { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 86_400_000;

    struct Tree {
        _tmp: tempfile::TempDir,
        temp: PathBuf,
        outside: PathBuf,
    }

    fn write_aged(path: &Path, days: i64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"data").unwrap();
        hoardini_win::set_all_times(path, crate::store::now_ms() - days * DAY_MS).unwrap();
    }

    fn tree() -> Tree {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
        let temp = root.join("Temp");
        let outside = root.join("outside");
        write_aged(&temp.join(r"a\x.txt"), 3);
        write_aged(&outside.join("x.txt"), 3);
        Tree {
            _tmp: tmp,
            temp,
            outside,
        }
    }

    fn clear_with_hook(t: &Tree, hook: impl Fn(&str) + 'static) -> Cleared {
        AFTER_LIST.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
        let mut c = Cleared::default();
        let r = clear_contents(&t.temp, crate::store::now_ms() - DAY_MS, &mut c);
        AFTER_LIST.with(|h| *h.borrow_mut() = None);
        r.unwrap();
        c
    }

    fn is_a(listed: &str) -> bool {
        listed.to_ascii_lowercase().ends_with(r"\temp\a")
    }

    // Traces: ACT-WINDOWS-CLEANUP-RUN
    #[test]
    fn a_folder_swapped_for_a_junction_mid_walk_never_sends_a_delete_outside() {
        let t = tree();
        let (a, moved, outside) = (t.temp.join("a"), t.temp.join("a-moved"), t.outside.clone());
        let c = clear_with_hook(&t, move |listed| {
            if is_a(listed) {
                std::fs::rename(&a, &moved).unwrap();
                hoardini_win::create_junction(&a, &outside).unwrap();
            }
        });
        assert!(
            t.outside.join("x.txt").exists(),
            "the junction's target is untouched"
        );
        assert!(
            !t.temp.join(r"a-moved\x.txt").exists(),
            "the real file, under its new name, is the one deleted"
        );
        assert_eq!((c.files, c.failed), (1, 0), "{c:?}");
        assert!(t.temp.join("a").exists(), "the junction itself stays");
    }

    // Traces: ACT-WINDOWS-CLEANUP-RUN
    #[test]
    fn a_folder_moved_out_of_the_target_mid_walk_keeps_its_files() {
        let t = tree();
        let (a, away) = (t.temp.join("a"), t.outside.join("a-away"));
        let c = clear_with_hook(&t, move |listed| {
            if is_a(listed) {
                std::fs::rename(&a, &away).unwrap();
            }
        });
        assert!(
            t.outside.join(r"a-away\x.txt").exists(),
            "its final path is no longer under the target, so nothing in it is deleted"
        );
        assert!(t.outside.join("x.txt").exists());
        assert_eq!((c.files, c.failed), (0, 1), "{c:?}");
    }

    // Traces: ACT-WINDOWS-CLEANUP-RUN
    #[test]
    fn a_file_unpacked_today_with_an_old_last_write_is_recent() {
        let t = tree();
        let unpacked = t.temp.join("setup.cab");
        std::fs::write(&unpacked, b"cab").unwrap();
        let old = crate::store::now_ms() - 30 * DAY_MS;
        hoardini_win::set_file_times(&unpacked, Some(old), Some(old)).unwrap();
        let changed = t.temp.join("made-read-only-today.log");
        write_aged(&changed, 3);
        std::fs::set_permissions(&changed, {
            let mut p = std::fs::metadata(&changed).unwrap().permissions();
            p.set_readonly(true);
            p
        })
        .unwrap();
        let mut c = Cleared::default();
        clear_contents(&t.temp, crate::store::now_ms() - DAY_MS, &mut c).unwrap();
        assert!(
            unpacked.exists(),
            "created today, whatever its last write says"
        );
        assert!(
            changed.exists(),
            "made read-only today, so its change time is today"
        );
        assert!(!t.temp.join(r"a\x.txt").exists());
        assert_eq!((c.files, c.recent), (1, 2), "{c:?}");
    }
}
