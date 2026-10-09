#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use hoardini_core::paths;
use hoardini_server::{DialogProvider, ServerOptions};
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

struct TauriDialogs(tauri::AppHandle);

impl DialogProvider for TauriDialogs {
    fn pick_folder(&self) -> BoxFuture<'static, Option<PathBuf>> {
        let app = self.0.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let mut dlg = app.dialog().file();
            if let Some(w) = app.get_webview_window("main") {
                dlg = dlg.set_parent(&w);
            }
            dlg.pick_folder(move |p| {
                let _ = tx.send(p.and_then(|p| p.into_path().ok()));
            });
            rx.await.ok().flatten()
        })
    }
}

struct Shutdown(Mutex<Option<tokio::sync::oneshot::Sender<()>>>);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Must precede any Tauri/webview init: the server spawns this exe as the indexer child.
    if args.get(1).map(String::as_str) == Some("index") {
        std::process::exit(hoardini_core::index::run_child(&args[2..]));
    }
    // Release builds have no console (windows_subsystem), so stderr only helps in debug runs.
    let _log =
        hoardini_server::init_logging(&hoardini_server::default_log_dir(), cfg!(debug_assertions))
            .ok();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let rt_handle = rt.handle().clone();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let opts = ServerOptions {
                port: 0,
                token: None,
                db_path: paths::db_path(),
                ui_dir: None,
                dialogs: Some(Arc::new(TauriDialogs(app.handle().clone()))),
                refresh_on_start: true,
                indexer_exe: None,
            };
            let server = match rt_handle.block_on(hoardini_server::start(opts)) {
                Ok(s) => s,
                Err(e) => {
                    app.dialog()
                        .message(format!("{e:#}"))
                        .title("Hoardini failed to start")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    std::process::exit(1);
                }
            };
            let url = server.url().parse()?;
            app.manage(Shutdown(Mutex::new(Some(server.shutdown))));
            let opener = app.handle().clone();
            let nav_opener = opener.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .on_new_window(move |target, _| {
                    // Rust-side only: the page keeps zero IPC (UI-NO-IPC), so no capability is granted.
                    if matches!(target.scheme(), "http" | "https") {
                        let _ = opener.opener().open_url(target.as_str(), None::<&str>);
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .on_navigation(move |target| {
                    if target.host_str() == Some("127.0.0.1") {
                        return true;
                    }
                    if matches!(target.scheme(), "http" | "https") {
                        let _ = nav_opener.opener().open_url(target.as_str(), None::<&str>);
                    }
                    false
                })
                .title("Hoardini")
                .inner_size(1000.0, 820.0)
                .min_inner_size(720.0, 560.0)
                .center()
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Hoardini");

    app.run(|app, event| {
        if let RunEvent::Exit = event
            && let Some(s) = app.try_state::<Shutdown>()
            && let Some(tx) = s.0.lock().unwrap().take()
        {
            let _ = tx.send(());
        }
    });
    // Lets the server kill the indexer child before the runtime drops.
    rt.shutdown_timeout(std::time::Duration::from_secs(3));
}
