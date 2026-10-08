// SEAM (wave 2): signatures are the contract between the server and the Tauri shell / CLI.
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;

// The Tauri shell implements this with native dialogs; headless serve has none (routes return 501).
pub trait DialogProvider: Send + Sync {
    fn pick_folder(&self) -> BoxFuture<'static, Option<PathBuf>>;
}

pub struct ServerOptions {
    pub port: u16,
    pub token: Option<String>,
    pub db_path: PathBuf,
    pub ui_dir: Option<PathBuf>,
    pub dialogs: Option<Arc<dyn DialogProvider>>,
    pub refresh_on_start: bool,
    // None = current_exe(); tests and the Tauri shell may point elsewhere.
    pub indexer_exe: Option<PathBuf>,
}

pub struct ServerHandle {
    pub port: u16,
    pub token: String,
    pub shutdown: tokio::sync::oneshot::Sender<()>,
}

impl ServerHandle {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/#token={}", self.port, self.token)
    }
}

pub async fn start(_opts: ServerOptions) -> anyhow::Result<ServerHandle> {
    anyhow::bail!("server not implemented yet")
}
