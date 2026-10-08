use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use secrecy::zeroize::Zeroize;

pub use secrecy::{ExposeSecret, SecretString};

pub const SECRET_SCHEME: &str = "secret://";
const ENTROPY: &[u8] = b"hoardini-vault-v1";
pub const VAULT_FILE: &str = "secrets.vault";

pub trait SecretStore: Send + Sync {
    fn get(&self, reference: &str) -> anyhow::Result<Option<SecretString>>;
    fn set(&self, reference: &str, value: SecretString) -> anyhow::Result<()>;
    fn delete(&self, reference: &str) -> anyhow::Result<()>;
    fn refs(&self) -> anyhow::Result<Vec<String>>;
}

pub fn is_ref(s: &str) -> bool {
    s.starts_with(SECRET_SCHEME)
}

pub fn provider_ref(id: &str) -> String {
    format!("{SECRET_SCHEME}provider/{id}")
}

pub fn destination_password_ref(id: &str) -> String {
    format!("{SECRET_SCHEME}destination/{id}/password")
}

pub fn search_ref(kind: &str) -> String {
    format!("{SECRET_SCHEME}search/{kind}")
}

pub struct DpapiVault {
    pub path: PathBuf,
    lock: Mutex<()>,
}

impl std::fmt::Debug for DpapiVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DpapiVault")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

type Map = BTreeMap<String, SecretString>;

impl DpapiVault {
    // Fails early when an existing file cannot be decrypted (another user, or corrupt), instead of overwriting it later.
    pub fn open(path: &Path) -> anyhow::Result<DpapiVault> {
        let v = DpapiVault {
            path: path.to_path_buf(),
            lock: Mutex::new(()),
        };
        v.load()?;
        Ok(v)
    }

    pub fn default_path() -> PathBuf {
        crate::paths::data_dir().join(VAULT_FILE)
    }

    fn load(&self) -> anyhow::Result<Map> {
        let blob = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(e) => return Err(anyhow::anyhow!("cannot read {}: {e}", self.path.display())),
        };
        let mut plain = hoardini_win::dpapi_unprotect(&blob, ENTROPY).map_err(|e| {
            anyhow::anyhow!(
                "cannot decrypt {} (written by another Windows user?): {e}",
                self.path.display()
            )
        })?;
        let parsed = serde_json::from_slice::<BTreeMap<String, String>>(&plain);
        plain.zeroize();
        let raw =
            parsed.map_err(|_| anyhow::anyhow!("{} is not a valid vault", self.path.display()))?;
        Ok(raw
            .into_iter()
            .map(|(k, v)| (k, SecretString::from(v)))
            .collect())
    }

    fn save(&self, map: &Map) -> anyhow::Result<()> {
        let exposed: BTreeMap<&str, &str> = map
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose_secret()))
            .collect();
        let mut plain = serde_json::to_vec(&exposed)?;
        let blob = hoardini_win::dpapi_protect(&plain, ENTROPY);
        plain.zeroize();
        let blob = blob?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("tmp-{}", uuid::Uuid::now_v7().simple()));
        std::fs::write(&tmp, &blob)?;
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }
}

fn check_ref(reference: &str) -> anyhow::Result<()> {
    if !is_ref(reference) || reference.len() <= SECRET_SCHEME.len() {
        anyhow::bail!("not a secret reference: must start with {SECRET_SCHEME}");
    }
    Ok(())
}

impl SecretStore for DpapiVault {
    fn get(&self, reference: &str) -> anyhow::Result<Option<SecretString>> {
        let _g = self.lock.lock();
        Ok(self.load()?.remove(reference))
    }

    fn set(&self, reference: &str, value: SecretString) -> anyhow::Result<()> {
        check_ref(reference)?;
        let _g = self.lock.lock();
        let mut map = self.load()?;
        map.insert(reference.to_string(), value);
        self.save(&map)
    }

    fn delete(&self, reference: &str) -> anyhow::Result<()> {
        let _g = self.lock.lock();
        let mut map = self.load()?;
        if map.remove(reference).is_some() {
            self.save(&map)?;
        }
        Ok(())
    }

    fn refs(&self) -> anyhow::Result<Vec<String>> {
        let _g = self.lock.lock();
        Ok(self.load()?.into_keys().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_deletes_and_redacts_debug() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(VAULT_FILE);
        let v = DpapiVault::open(&path).unwrap();
        v.set("secret://provider/a", SecretString::from("sk-one"))
            .unwrap();
        v.set("secret://search/tavily", SecretString::from("tv-two"))
            .unwrap();
        assert!(v.set("plain", SecretString::from("x")).is_err());
        let again = DpapiVault::open(&path).unwrap();
        assert_eq!(
            again
                .get("secret://provider/a")
                .unwrap()
                .unwrap()
                .expose_secret(),
            "sk-one"
        );
        again.delete("secret://provider/a").unwrap();
        assert!(v.get("secret://provider/a").unwrap().is_none());
        assert_eq!(
            v.refs().unwrap(),
            vec!["secret://search/tavily".to_string()]
        );
        assert!(!format!("{v:?}").contains("tv-two"));
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn refuses_a_file_it_cannot_decrypt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(VAULT_FILE);
        std::fs::write(&path, b"not dpapi").unwrap();
        assert!(DpapiVault::open(&path).is_err());
    }
}
