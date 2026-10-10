mod helper;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::index::{self, NodeKind, TopFilesQuery, normalize_path};
use crate::paths::humanize_bytes;
use crate::policy::Verdict;

pub use helper::{OpResult, count_shadows, helper_params, parse_helper_args, run_helper};

pub const MIN_SAVING: u64 = 1 << 30;
pub const MIN_WINSXS: u64 = 2 << 30;
pub const MIN_UNSEEN: u64 = 2 << 30;
pub const MIN_VHDX: u64 = 5 << 30;
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
}

impl SystemOp {
    pub fn name(&self) -> &'static str {
        match self {
            SystemOp::ComponentStore => "component_store",
            SystemOp::RestorePoints { .. } => "restore_points",
            SystemOp::Hibernation => "hibernation",
            SystemOp::CompactVhdx { .. } => "compact_vhdx",
            SystemOp::RecycleBin => "recycle_bin",
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CleanupCandidate {
    pub op: SystemOp,
    pub target: String,
    pub estimate: Option<u64>,
    pub size: Option<u64>,
    pub note: String,
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
            op: self.op.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WslDisk {
    pub distro: String,
    pub path: String,
}

// Everything detection reads outside the index; `gather` reads it without elevation.
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

// `wanted` names operations from OP_NAMES (empty = every detected one); a wanted one not detected is dropped.
pub fn propose(
    db: &crate::store::Db,
    facts: &SystemFacts,
    wanted: &[&str],
    conversation_id: Option<String>,
    tool_call_id: Option<String>,
) -> anyhow::Result<crate::proposals::Proposal> {
    let found = candidates(&db.conn()?, facts)?;
    let ops: Vec<CleanupCandidate> = found
        .into_iter()
        .filter(|c| wanted.is_empty() || wanted.contains(&c.op.name()))
        .collect();
    let dropped = wanted
        .iter()
        .filter(|w| !ops.iter().any(|c| c.op.name() == **w))
        .map(|w| crate::proposals::DroppedItem {
            path: (*w).to_string(),
            reason: "not detected on this PC: nothing big enough for Windows to clean up".into(),
        })
        .collect();
    crate::proposals::create_system_cleanup(
        db,
        crate::proposals::NewSystemCleanup {
            conversation_id,
            tool_call_id,
            summary: "Cleanups only Windows can do".into(),
            ops,
            dropped,
        },
    )
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
        });
    }
    Ok(out)
}
