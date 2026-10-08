use std::path::Path;

use hoardini_core::destinations::{self, NewDestination};
use hoardini_core::store::Db;
use hoardini_core::vault::{
    DpapiVault, ExposeSecret, SecretStore, SecretString, destination_password_ref, provider_ref,
    search_ref,
};

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn all_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            all_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

// Traces: VAULT-AT-REST
#[test]
fn no_plaintext_secret_exists_at_rest_and_refs_still_resolve() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("hoardini");
    let db = Db::open(data.join("hoardini.db")).unwrap();
    let conn = db.conn().unwrap();
    let vault = DpapiVault::open(&data.join("secrets.vault")).unwrap();

    let api_key = "sk-AT-REST-provider-0123456789abcdef";
    let smb_password = "hunter2-SMB-pass-XYZ-4242";
    let search_key = "tvly-SEARCH-key-987654321";
    let rclone_pass = "rclone-config-pass-ZZTOP";

    let pref = provider_ref("local-lmstudio");
    vault.set(&pref, SecretString::from(api_key)).unwrap();
    conn.execute(
        "INSERT INTO provider(id, name, config_json, is_active, created_ms) VALUES ('local-lmstudio', 'LM Studio', ?1, 1, 0)",
        [serde_json::json!({"name": "LM Studio", "base_url": "http://localhost:1234/v1", "api_key": pref}).to_string()],
    )
    .unwrap();

    let refused = destinations::create(
        &conn,
        NewDestination {
            name: "nas".into(),
            kind: "smb".into(),
            config: serde_json::json!({"path": r"\\nas\archive", "username": "josh", "password": smb_password}),
        },
    );
    assert!(refused.unwrap_err().to_string().contains("secret://"));
    let d = destinations::create(
        &conn,
        NewDestination {
            name: "nas".into(),
            kind: "smb".into(),
            config: serde_json::json!({"path": r"\\nas\archive", "username": "josh"}),
        },
    )
    .unwrap();
    let dref = destination_password_ref(&d.id);
    vault.set(&dref, SecretString::from(smb_password)).unwrap();
    destinations::update(
        &conn,
        &d.id,
        NewDestination {
            name: "nas".into(),
            kind: "smb".into(),
            config: serde_json::json!({"path": r"\\nas\archive", "username": "josh", "password": dref}),
        },
    )
    .unwrap();

    let sref = search_ref("tavily");
    vault.set(&sref, SecretString::from(search_key)).unwrap();
    conn.execute(
        "INSERT INTO search_provider(id, kind, config_json, is_active, created_ms) VALUES ('tavily', 'tavily', ?1, 1, 0)",
        [serde_json::json!({"api_key": sref}).to_string()],
    )
    .unwrap();
    vault
        .set("secret://rclone/config", SecretString::from(rclone_pass))
        .unwrap();
    std::fs::write(
        data.join("settings.json"),
        serde_json::json!({"active_provider": "local-lmstudio", "api_key": pref}).to_string(),
    )
    .unwrap();

    // The open connection keeps the WAL file around, so it is scanned too.
    let mut files = Vec::new();
    all_files(&data, &mut files);
    assert!(
        files
            .iter()
            .any(|f| f.to_string_lossy().ends_with("secrets.vault"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.to_string_lossy().ends_with("hoardini.db-wal"))
    );
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        for secret in [api_key, smb_password, search_key, rclone_pass] {
            assert!(
                !contains(&bytes, secret.as_bytes()),
                "{secret} in {}",
                f.display()
            );
            assert!(
                !contains(&bytes, &utf16(secret)),
                "{secret} (UTF-16) in {}",
                f.display()
            );
        }
    }
    let row: String = conn
        .query_row(
            "SELECT config_json FROM destination WHERE id = ?1",
            [&d.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(row.contains(&dref));

    drop(vault);
    let reopened = DpapiVault::open(&data.join("secrets.vault")).unwrap();
    for (r, want) in [
        (pref.as_str(), api_key),
        (dref.as_str(), smb_password),
        (sref.as_str(), search_key),
        ("secret://rclone/config", rclone_pass),
    ] {
        assert_eq!(reopened.get(r).unwrap().unwrap().expose_secret(), want);
    }
    assert_eq!(reopened.refs().unwrap().len(), 4);
    let shown = format!("{reopened:?} {:?}", reopened.get(&pref).unwrap());
    assert!(!shown.contains(api_key), "{shown}");
}
