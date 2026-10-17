mod helper;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::index::{self, NodeKind, TopFilesQuery, normalize_path};
use crate::paths::humanize_bytes;
use crate::policy::Verdict;

pub use helper::{
    Cleared, OpResult, clear_contents, count_shadows, diskpart_scripts, helper_params,
    parse_helper_args, run_helper,
};

pub const MIN_SAVING: u64 = 1 << 30;
pub const MIN_WINSXS: u64 = 2 << 30;
pub const MIN_UNSEEN: u64 = 2 << 30;
pub const MIN_VHDX: u64 = 5 << 30;
pub const LEFTOVER_AGE_HOURS: i64 = 24;
// Catalog entries whose machine-wide folders only an administrator may clear; nothing else is ever cleared.
pub const LEFTOVER_ENTRIES: [&str; 5] = [
    "temp-system",
    "wu-download",
    "delivery-optimization",
    "crash-dumps",
    "windows-logs",
];
pub const OP_NAMES: [&str; 5] = [
    "component_store",
    "restore_points",
    "hibernation",
    "compact_vhdx",
    "recycle_bin",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SystemOp {
    ComponentStore,
    RestorePoints { volume: String },
    Hibernation,
    CompactVhdx { path: String },
    RecycleBin,
    Leftovers,
}

impl SystemOp {
    pub fn name(&self) -> &'static str {
        match self {
            SystemOp::ComponentStore => "component_store",
            SystemOp::RestorePoints { .. } => "restore_points",
            SystemOp::Hibernation => "hibernation",
            SystemOp::CompactVhdx { .. } => "compact_vhdx",
            SystemOp::RecycleBin => "recycle_bin",
            SystemOp::Leftovers => "leftovers",
        }
    }

    // The Recycle Bin is per user, so it is emptied as the user, outside the admin prompt.
    pub fn elevated(&self) -> bool {
        !matches!(self, SystemOp::RecycleBin)
    }

    pub fn irreversible(&self) -> bool {
        matches!(self, SystemOp::RestorePoints { .. } | SystemOp::RecycleBin)
    }

    pub fn verdict(&self) -> Verdict {
        match self {
            SystemOp::RestorePoints { .. } | SystemOp::RecycleBin => Verdict::Review,
            _ => Verdict::Safe,
        }
    }

    pub fn label(&self) -> String {
        match self {
            SystemOp::ComponentStore => "Windows Update leftovers (component store)".into(),
            SystemOp::RestorePoints { volume } => {
                format!("Older System Restore points on {volume}")
            }
            SystemOp::Hibernation => "Hibernation file at the reduced size".into(),
            SystemOp::CompactVhdx { path } => format!(
                "Compact {}",
                path.rsplit('\\').next().unwrap_or(path.as_str())
            ),
            SystemOp::RecycleBin => "Empty the Recycle Bin".into(),
            SystemOp::Leftovers => {
                "Windows temp files, update downloads, crash dumps and logs".into()
            }
        }
    }

    pub fn explain(&self) -> &'static str {
        match self {
            SystemOp::ComponentStore => {
                "Windows removes superseded versions of its own components from WinSxS (DISM /StartComponentCleanup, the same as Disk Cleanup's Windows Update Cleanup). Updates installed so far can no longer be uninstalled. It can take ten minutes or more."
            }
            SystemOp::RestorePoints { .. } => {
                "Windows deletes every restore point on this drive except the newest (vssadmin delete shadows /oldest, repeated). Deleted restore points cannot be brought back."
            }
            SystemOp::Hibernation => {
                "Windows shrinks hiberfil.sys to the reduced size (powercfg /h /type reduced). Fast Startup keeps working; the Hibernate option goes away, so a laptop on critical battery shuts down instead of hibernating. powercfg /h /type full undoes it."
            }
            SystemOp::CompactVhdx { .. } => {
                "Hoardini stops WSL first (wsl --shutdown ends every running Linux distro and Docker Desktop's engine; quit Docker Desktop before approving), then Windows compacts the virtual disk (diskpart compact vdisk) so the space Linux freed inside it goes back to the drive. Nothing inside the disk changes."
            }
            SystemOp::RecycleBin => {
                "Windows empties the Recycle Bin on every drive. The files in it are gone for good. This one needs no administrator prompt."
            }
            SystemOp::Leftovers => {
                "Hoardini empties Windows' own temp, update download, Delivery Optimization, crash dump and servicing log folders, which only an administrator may clear. Only files untouched for a day go; files in use and the folders themselves stay. Windows downloads again anything it still needs."
            }
        }
    }

    pub fn undo(&self) -> serde_json::Value {
        match self {
            SystemOp::Hibernation => serde_json::json!({
                "kind": "command",
                "reason": "run powercfg /h /type full to bring back the full hibernation file",
                "argv": ["powercfg.exe", "/h", "/type", "full"],
            }),
            SystemOp::ComponentStore => serde_json::json!({"kind": "none",
                "reason": "superseded components are gone; installed updates can no longer be uninstalled"}),
            SystemOp::RestorePoints { .. } => serde_json::json!({"kind": "none",
                "reason": "deleted restore points cannot be brought back"}),
            SystemOp::CompactVhdx { .. } => serde_json::json!({"kind": "none",
                "reason": "compacting changes nothing inside the disk; there is nothing to undo"}),
            SystemOp::RecycleBin => serde_json::json!({"kind": "none",
                "reason": "the Recycle Bin was emptied; its files are gone"}),
            SystemOp::Leftovers => serde_json::json!({"kind": "none",
                "reason": "the temp files, update downloads, crash dumps and logs were deleted; Windows makes new ones as it needs them"}),
        }
    }

    pub fn volume(&self, system_drive: &str) -> String {
        match self {
            SystemOp::RestorePoints { volume } => format!("{}\\", volume.trim_end_matches('\\')),
            SystemOp::CompactVhdx { path } => normalize_path(path.get(..2).unwrap_or(system_drive)),
            _ => normalize_path(system_drive),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpPart {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpView {
    #[serde(flatten)]
    pub op: SystemOp,
    pub label: String,
    pub explain: String,
    pub elevated: bool,
    pub irreversible: bool,
    pub estimate_known: bool,
    pub size: Option<u64>,
    // Shown only; the elevated helper derives its own targets (leftover_targets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<OpPart>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CleanupCandidate {
    pub op: SystemOp,
    pub target: String,
    pub estimate: Option<u64>,
    pub size: Option<u64>,
    pub note: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<OpPart>,
}

impl CleanupCandidate {
    pub fn view(&self) -> OpView {
        OpView {
            label: self.op.label(),
            explain: self.op.explain().into(),
            elevated: self.op.elevated(),
            irreversible: self.op.irreversible(),
            estimate_known: self.estimate.is_some(),
            size: self.size,
            parts: self.parts.clone(),
            op: self.op.clone(),
        }
    }
}

fn after_var<'a>(pattern: &'a str, var: &str) -> Option<&'a str> {
    let head = pattern.get(..var.len())?;
    if !head.eq_ignore_ascii_case(var) {
        return None;
    }
    pattern[var.len()..].strip_prefix('\\')
}

// Both the review and the elevated helper use this list, so the helper never takes a path from outside.
pub fn leftover_targets(windows_dir: &str, program_data: &str) -> Vec<String> {
    let catalog = crate::catalog::Catalog::embedded();
    let mut out: Vec<String> = Vec::new();
    for id in LEFTOVER_ENTRIES {
        let Some(e) = catalog.get(id) else {
            continue;
        };
        for p in &e.patterns {
            let full = match (after_var(p, "%SystemRoot%"), after_var(p, "%ProgramData%")) {
                (Some(rest), _) => join(windows_dir, rest),
                (_, Some(rest)) => join(program_data, rest),
                _ => continue,
            };
            if full.contains(['*', '?', '%']) || full.split('\\').any(|c| c == "..") {
                continue;
            }
            let full = normalize_path(&full);
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&full)) {
                out.push(full);
            }
        }
    }
    out
}

pub fn leftovers_candidate(mut parts: Vec<OpPart>) -> Option<CleanupCandidate> {
    parts.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    let total: u64 = parts.iter().map(|p| p.size).sum();
    let target = parts.first()?.path.clone();
    Some(CleanupCandidate {
        op: SystemOp::Leftovers,
        target,
        estimate: Some(total),
        size: Some(total),
        note: format!(
            "{} of Windows temp files, update downloads, crash dumps and logs untouched for a day, in {} place{}",
            humanize_bytes(total),
            parts.len(),
            if parts.len() == 1 { "" } else { "s" }
        ),
        parts,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WslDisk {
    pub distro: String,
    pub path: String,
}

#[derive(Clone, Debug, Default)]
pub struct SystemFacts {
    pub system_drive: String,
    pub windows_dir: String,
    pub ram_bytes: Option<u64>,
    pub hibernate_enabled: Option<bool>,
    pub recycle_bin: Option<hoardini_win::RecycleBin>,
    pub wsl_disks: Vec<WslDisk>,
    pub docker_dir: Option<String>,
}

fn strip_verbatim(p: &str) -> String {
    normalize_path(p.strip_prefix(r"\\?\").unwrap_or(p))
}

impl SystemFacts {
    pub fn gather() -> SystemFacts {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let drive = env("SystemDrive").unwrap_or_else(|| "C:".into());
        SystemFacts {
            system_drive: normalize_path(&drive),
            windows_dir: normalize_path(
                &env("SystemRoot").unwrap_or_else(|| format!("{drive}\\Windows")),
            ),
            ram_bytes: hoardini_win::physical_memory().ok(),
            hibernate_enabled: hoardini_win::hibernate_enabled(),
            recycle_bin: hoardini_win::recycle_bin().ok(),
            wsl_disks: hoardini_win::wsl_distros()
                .into_iter()
                .filter(|d| d.version >= 2)
                .map(|d| WslDisk {
                    path: format!(
                        "{}\\{}",
                        strip_verbatim(&d.base_path),
                        d.vhd_file.as_deref().unwrap_or("ext4.vhdx")
                    ),
                    distro: d.name,
                })
                .collect(),
            docker_dir: env("LOCALAPPDATA").map(|l| normalize_path(&format!(r"{l}\Docker\wsl"))),
        }
    }
}

fn indexed_size(conn: &Connection, path: &str, kind: NodeKind) -> anyhow::Result<Option<u64>> {
    Ok(index::node_by_path(conn, path)?
        .filter(|n| n.kind == kind)
        .map(|n| n.size))
}

fn join(dir: &str, name: &str) -> String {
    format!("{}\\{name}", dir.trim_end_matches('\\'))
}

// Fixed order on the card; sizes come from the index wherever it can tell.
pub fn candidates(conn: &Connection, facts: &SystemFacts) -> anyhow::Result<Vec<CleanupCandidate>> {
    let mut out = Vec::new();
    let winsxs = join(&facts.windows_dir, "WinSxS");
    if let Some(size) = indexed_size(conn, &winsxs, NodeKind::Dir)?
        && size >= MIN_WINSXS
    {
        out.push(CleanupCandidate {
            op: SystemOp::ComponentStore,
            target: index::node_by_path(conn, &winsxs)?.map_or(winsxs, |n| n.path),
            estimate: None,
            size: Some(size),
            note: format!(
                "WinSxS holds {} that is not hard-linked into Windows itself; Windows decides how much of it is superseded",
                humanize_bytes(size)
            ),
            parts: Vec::new(),
        });
    }
    let volume = facts.system_drive.trim_end_matches('\\').to_string();
    if let Some(unseen) = index::index_status(conn, Some(&facts.system_drive))?
        .and_then(|s| s.unaccounted_bytes)
        .and_then(|b| u64::try_from(b).ok())
        && unseen >= MIN_UNSEEN
    {
        out.push(CleanupCandidate {
            op: SystemOp::RestorePoints {
                volume: volume.clone(),
            },
            target: join(&facts.system_drive, "System Volume Information"),
            estimate: None,
            size: Some(unseen),
            note: format!(
                "{} on {volume} is used but outside what the index can see; restore points are the usual cause",
                humanize_bytes(unseen)
            ),
            parts: Vec::new(),
        });
    }
    let hiberfil = join(&facts.system_drive, "hiberfil.sys");
    if facts.hibernate_enabled != Some(false)
        && let (Some(size), Some(ram)) = (
            indexed_size(conn, &hiberfil, NodeKind::File)?,
            facts.ram_bytes,
        )
        && size.saturating_sub(ram / 5) >= MIN_SAVING
    {
        out.push(CleanupCandidate {
            op: SystemOp::Hibernation,
            target: index::node_by_path(conn, &hiberfil)?.map_or(hiberfil, |n| n.path),
            estimate: Some(size - ram / 5),
            size: Some(size),
            note: format!(
                "hiberfil.sys is {}; the reduced size is about {} (a fifth of the RAM)",
                humanize_bytes(size),
                humanize_bytes(ram / 5)
            ),
            parts: Vec::new(),
        });
    }
    let mut disks: Vec<(String, String)> = facts
        .wsl_disks
        .iter()
        .map(|d| (normalize_path(&d.path), d.distro.clone()))
        .collect();
    if let Some(dir) = &facts.docker_dir {
        let q = TopFilesQuery {
            under: Some(dir.clone()),
            min_size: MIN_VHDX,
            ext: Some("vhdx".into()),
            not_used_for_days: None,
            limit: 20,
        };
        if let Ok(found) = index::top_files(conn, &q) {
            disks.extend(
                found
                    .items
                    .into_iter()
                    .map(|f| (f.path, "Docker Desktop".to_string())),
            );
        }
    }
    let mut seen: Vec<String> = Vec::new();
    for (path, owner) in disks {
        if seen.iter().any(|p| p.eq_ignore_ascii_case(&path)) {
            continue;
        }
        seen.push(path.clone());
        let Some(n) = index::node_by_path(conn, &path)?.filter(|n| n.kind == NodeKind::File) else {
            continue;
        };
        if n.size < MIN_VHDX {
            continue;
        }
        out.push(CleanupCandidate {
            op: SystemOp::CompactVhdx {
                path: n.path.clone(),
            },
            note: format!(
                "{owner}'s virtual disk is {}; compacting returns the space Linux has freed inside it",
                humanize_bytes(n.size)
            ),
            target: n.path,
            estimate: None,
            size: Some(n.size),
            parts: Vec::new(),
        });
    }
    if let Some(bin) = facts.recycle_bin
        && bin.bytes >= MIN_SAVING
    {
        out.push(CleanupCandidate {
            op: SystemOp::RecycleBin,
            target: join(&facts.system_drive, "$Recycle.Bin"),
            estimate: Some(bin.bytes),
            size: Some(bin.bytes),
            note: format!(
                "{} item{} ({}) in the Recycle Bin across every drive",
                bin.items,
                if bin.items == 1 { "" } else { "s" },
                humanize_bytes(bin.bytes)
            ),
            parts: Vec::new(),
        });
    }
    Ok(out)
}
