use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::index::normalize_path;
use crate::store::now_ms;
use crate::vault::{SecretStore, destination_password_ref, is_ref};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub config: serde_json::Value,
    pub created_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewDestination {
    pub name: String,
    pub kind: String,
    pub config: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DestinationTest {
    pub ok: bool,
    pub free_bytes: Option<i64>,
    pub error: Option<String>,
}

const KINDS: &[&str] = &["local", "smb", "rclone"];

fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["pass", "secret", "token", "key", "credential"]
        .iter()
        .any(|w| k.contains(w))
}

// Secrets live in the vault; config may only point at them.
fn check_no_secrets(v: &serde_json::Value, at: &str) -> anyhow::Result<()> {
    if let Some(obj) = v.as_object() {
        for (k, val) in obj {
            let here = if at.is_empty() {
                k.clone()
            } else {
                format!("{at}.{k}")
            };
            match val {
                serde_json::Value::String(s) if looks_secret(k) && !s.is_empty() && !is_ref(s) => {
                    anyhow::bail!(
                        "config.{here} must be a secret:// reference; store the value in the vault (PUT /api/secrets) first"
                    )
                }
                serde_json::Value::Object(_) => check_no_secrets(val, &here)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn config_path(config: &serde_json::Value) -> Option<String> {
    config
        .get("path")
        .and_then(|p| p.as_str())
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(normalize_path)
}

fn validate(new: &NewDestination) -> anyhow::Result<serde_json::Value> {
    if new.name.trim().is_empty() {
        anyhow::bail!("a destination needs a name");
    }
    if !KINDS.contains(&new.kind.as_str()) {
        anyhow::bail!("destination kind must be local, smb, or rclone");
    }
    let mut config = if new.config.is_null() {
        serde_json::json!({})
    } else {
        new.config.clone()
    };
    if !config.is_object() {
        anyhow::bail!("destination config must be a JSON object");
    }
    check_no_secrets(&config, "")?;
    match new.kind.as_str() {
        "local" => {
            let p = config_path(&config)
                .ok_or_else(|| anyhow::anyhow!("a local destination needs config.path"))?;
            let b = p.as_bytes();
            let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
            if !drive && !p.starts_with(r"\\") {
                anyhow::bail!("config.path must be an absolute folder path like E:\\archive");
            }
            config["path"] = serde_json::Value::String(p);
        }
        "smb" => {
            let p = config_path(&config)
                .ok_or_else(|| anyhow::anyhow!("an smb destination needs config.path"))?;
            let parts: Vec<&str> = p
                .trim_start_matches('\\')
                .split('\\')
                .filter(|s| !s.is_empty())
                .collect();
            if !p.starts_with(r"\\") || parts.len() < 2 {
                anyhow::bail!("config.path must be a UNC path like \\\\nas\\share\\archive");
            }
            config["path"] = serde_json::Value::String(p);
        }
        _ => {}
    }
    Ok(config)
}

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Destination> {
    let json: String = r.get(3)?;
    Ok(Destination {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        config: serde_json::from_str(&json).unwrap_or(serde_json::Value::Null),
        created_ms: r.get(4)?,
    })
}

const COLS: &str = "id, name, kind, config_json, created_ms";

pub fn list(conn: &Connection) -> anyhow::Result<Vec<Destination>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM destination ORDER BY created_ms, id"
    ))?;
    Ok(st
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get(conn: &Connection, id: &str) -> anyhow::Result<Option<Destination>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM destination WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

pub fn create(conn: &Connection, new: NewDestination) -> anyhow::Result<Destination> {
    let config = validate(&new)?;
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO destination(id, name, kind, config_json, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, new.name.trim(), new.kind, config.to_string(), now_ms()],
    )?;
    get(conn, &id)?.ok_or_else(|| anyhow::anyhow!("destination vanished"))
}

pub fn update(conn: &Connection, id: &str, new: NewDestination) -> anyhow::Result<Destination> {
    let config = validate(&new)?;
    let n = conn.execute(
        "UPDATE destination SET name = ?2, kind = ?3, config_json = ?4 WHERE id = ?1",
        params![id, new.name.trim(), new.kind, config.to_string()],
    )?;
    if n == 0 {
        anyhow::bail!("unknown destination {id}");
    }
    get(conn, id)?.ok_or_else(|| anyhow::anyhow!("destination vanished"))
}

pub fn delete(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    Ok(conn.execute("DELETE FROM destination WHERE id = ?1", [id])? > 0)
}

pub fn root_path(dest: &Destination) -> anyhow::Result<PathBuf> {
    match dest.kind.as_str() {
        "local" | "smb" => config_path(&dest.config)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("destination {} has no path", dest.name)),
        _ => anyhow::bail!(
            "{} destinations are not available until wave 3 (rclone)",
            dest.kind
        ),
    }
}

pub fn connect(dest: &Destination, secrets: &dyn SecretStore) -> anyhow::Result<PathBuf> {
    let root = root_path(dest)?;
    if dest.kind == "smb" {
        let user = dest
            .config
            .get("username")
            .and_then(|u| u.as_str())
            .filter(|u| !u.trim().is_empty());
        let reference = dest
            .config
            .get("password")
            .and_then(|p| p.as_str())
            .filter(|p| is_ref(p))
            .map(str::to_string)
            .unwrap_or_else(|| destination_password_ref(&dest.id));
        let password = secrets.get(&reference)?;
        let unc = root.to_string_lossy();
        if let Err(err) =
            hoardini_win::smb_connect(&unc, user, password.as_ref().map(|p| p.expose_secret()))
        {
            // An existing session to the same server (e.g. Explorer) already grants access.
            if !root.is_dir() {
                anyhow::bail!("cannot connect to {unc}: {err}");
            }
        }
    }
    Ok(root)
}

pub fn free_bytes(path: &Path) -> anyhow::Result<u64> {
    let root = hoardini_win::volume_root_of(path).unwrap_or_else(|_| path.to_path_buf());
    Ok(hoardini_win::volume_info(&root)
        .or_else(|_| hoardini_win::volume_info(path))?
        .free_bytes)
}

fn check_writable(dir: &Path) -> anyhow::Result<()> {
    if !dir.is_dir() {
        anyhow::bail!("{} does not exist or is not a folder", dir.display());
    }
    let probe = dir.join(format!(
        ".hoardini-write-test-{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::write(&probe, b"hoardini")
        .map_err(|e| anyhow::anyhow!("{} is not writable: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

pub fn test(dest: &Destination, secrets: &dyn SecretStore) -> DestinationTest {
    let run = || -> anyhow::Result<i64> {
        if dest.kind == "rclone" {
            anyhow::bail!("rclone destinations are not available until wave 3");
        }
        let root = connect(dest, secrets)?;
        check_writable(&root)?;
        Ok(crate::index::to_i64(free_bytes(&root)?))
    };
    match run() {
        Ok(free) => DestinationTest {
            ok: true,
            free_bytes: Some(free),
            error: None,
        },
        Err(e) => DestinationTest {
            ok: false,
            free_bytes: None,
            error: Some(e.to_string()),
        },
    }
}
