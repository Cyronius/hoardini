// SEAM (wave 2): the trait is the contract; DpapiVault's body is filled by the vault owner.
use std::path::{Path, PathBuf};

use secrecy::SecretString;

pub const SECRET_SCHEME: &str = "secret://";

pub trait SecretStore: Send + Sync {
    fn get(&self, reference: &str) -> anyhow::Result<Option<SecretString>>;
    fn set(&self, reference: &str, value: SecretString) -> anyhow::Result<()>;
    fn delete(&self, reference: &str) -> anyhow::Result<()>;
    fn refs(&self) -> anyhow::Result<Vec<String>>;
}

pub fn is_ref(s: &str) -> bool {
    s.starts_with(SECRET_SCHEME)
}

pub struct DpapiVault {
    pub path: PathBuf,
}

impl DpapiVault {
    pub fn open(path: &Path) -> anyhow::Result<DpapiVault> {
        Ok(DpapiVault {
            path: path.to_path_buf(),
        })
    }
}

impl SecretStore for DpapiVault {
    fn get(&self, _reference: &str) -> anyhow::Result<Option<SecretString>> {
        anyhow::bail!("vault not implemented yet")
    }
    fn set(&self, _reference: &str, _value: SecretString) -> anyhow::Result<()> {
        anyhow::bail!("vault not implemented yet")
    }
    fn delete(&self, _reference: &str) -> anyhow::Result<()> {
        anyhow::bail!("vault not implemented yet")
    }
    fn refs(&self) -> anyhow::Result<Vec<String>> {
        anyhow::bail!("vault not implemented yet")
    }
}
