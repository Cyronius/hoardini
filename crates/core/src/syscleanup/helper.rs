use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

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

fn diskpart(script: &str, dir: &Path, name: &str) -> anyhow::Result<std::process::Output> {
    let file = dir.join(name);
    std::fs::write(&file, script)?;
    let mut c = tool("diskpart.exe")?;
    c.arg("/s").arg(&file);
    run(c, "diskpart")
}

// Detach runs on its own, so a failed compact never leaves the disk attached.
fn compact_vhdx(path: &str) -> anyhow::Result<Option<String>> {
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        bail!("{path} is not there anymore");
    }
    let dir = std::env::temp_dir().join(format!("hoardini-diskpart-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let compact = diskpart(
        &format!("select vdisk file=\"{path}\"\nattach vdisk readonly\ncompact vdisk\n"),
        &dir,
        "compact.txt",
    );
    let _ = diskpart(
        &format!("select vdisk file=\"{path}\"\ndetach vdisk\n"),
        &dir,
        "detach.txt",
    );
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
