use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use hoardini_agent::run::IndexControl;
use hoardini_core::index;
use hoardini_core::store::{Db, get_meta};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Notify, watch};

#[derive(Default)]
struct State {
    refreshing: bool,
    mode: Option<&'static str>,
    progress: Option<Value>,
}

pub struct Supervisor {
    db: Db,
    exe: PathBuf,
    state: Mutex<State>,
    want_fast: AtomicBool,
    switch: Notify,
    change: Arc<watch::Sender<u64>>,
    rt: tokio::runtime::Handle,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    me: Weak<Supervisor>,
}

enum ChildEnd {
    Finished,
    SwitchedToFast,
}

fn norm_mode(mode: &str) -> Option<&'static str> {
    match mode {
        "background" => Some("background"),
        "fast" => Some("fast"),
        _ => None,
    }
}

pub fn indexed_volumes(db: &Db) -> Vec<String> {
    let from_meta = db
        .conn()
        .ok()
        .and_then(|c| get_meta(&c, "indexed_volumes").ok().flatten())
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .filter(|v| !v.is_empty());
    from_meta.unwrap_or_else(|| {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        vec![format!("{}\\", drive.trim_end_matches('\\'))]
    })
}

impl Supervisor {
    pub fn new(
        db: Db,
        exe: PathBuf,
        change: Arc<watch::Sender<u64>>,
        rt: tokio::runtime::Handle,
    ) -> Arc<Supervisor> {
        Arc::new_cyclic(|me| Supervisor {
            db,
            exe,
            state: Mutex::new(State::default()),
            want_fast: AtomicBool::new(false),
            switch: Notify::new(),
            change,
            rt,
            task: Mutex::new(None),
            me: me.clone(),
        })
    }

    fn bump(&self) {
        self.change.send_modify(|n| *n += 1);
    }

    pub fn snapshot(&self) -> Value {
        let (refreshing, mode, progress) = match self.state.lock() {
            Ok(s) => (s.refreshing, s.mode, s.progress.clone()),
            Err(_) => (false, None, None),
        };
        let (last, generation) = self
            .db
            .conn()
            .ok()
            .and_then(|c| index::index_status(&c, None).ok().flatten())
            .map(|s| (s.last_refresh_ms, s.generation))
            .unwrap_or((None, 0));
        json!({
            "state": if refreshing { "refreshing" } else { "idle" },
            "mode": mode,
            "progress": progress,
            "last_refresh_ms": last,
            "generation": generation,
        })
    }

    pub fn start_refresh(&self, mode: &str) -> anyhow::Result<bool> {
        let Some(mode) = norm_mode(mode) else {
            anyhow::bail!("mode must be \"background\" or \"fast\"");
        };
        let Ok(mut st) = self.state.lock() else {
            anyhow::bail!("indexer state unavailable");
        };
        if !st.refreshing {
            st.refreshing = true;
            st.mode = Some(mode);
            st.progress = None;
            drop(st);
            self.want_fast.store(false, Ordering::SeqCst);
            let Some(me) = self.me.upgrade() else {
                anyhow::bail!("supervisor is shutting down");
            };
            let handle = self.rt.spawn(async move { me.run(mode).await });
            if let Ok(mut t) = self.task.lock() {
                *t = Some(handle);
            }
            self.bump();
            return Ok(true);
        }
        if mode == "fast" && st.mode == Some("background") {
            st.mode = Some("fast");
            drop(st);
            self.want_fast.store(true, Ordering::SeqCst);
            self.switch.notify_one();
            self.bump();
            return Ok(true);
        }
        Ok(false)
    }

    // Aborting the task drops the child handle, and kill_on_drop ends the process.
    pub fn shutdown(&self) {
        if let Ok(mut t) = self.task.lock()
            && let Some(h) = t.take()
        {
            h.abort();
        }
    }

    async fn run(self: Arc<Self>, first_mode: &'static str) {
        let mut mode = first_mode;
        for volume in indexed_volumes(&self.db) {
            loop {
                if self.want_fast.swap(false, Ordering::SeqCst) {
                    mode = "fast";
                }
                match self.run_child(&volume, mode).await {
                    Ok(ChildEnd::SwitchedToFast) => continue,
                    Ok(ChildEnd::Finished) => {}
                    Err(e) => tracing::warn!(volume, error = %e, "indexer child failed"),
                }
                break;
            }
        }
        if let Ok(mut st) = self.state.lock() {
            st.refreshing = false;
            st.mode = None;
            st.progress = None;
        }
        self.bump();
    }

    async fn run_child(&self, volume: &str, mode: &str) -> anyhow::Result<ChildEnd> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("index")
            .arg("--db")
            .arg(self.db.path())
            .arg("--volume")
            .arg(volume)
            .arg("--mode")
            .arg(mode)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn()?;
        let Some(stdout) = child.stdout.take() else {
            anyhow::bail!("indexer stdout unavailable");
        };
        let mut lines = BufReader::new(stdout).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(l)) => self.on_line(&l),
                    Ok(None) | Err(_) => break,
                },
                _ = self.switch.notified() => {
                    if self.want_fast.load(Ordering::SeqCst) && mode != "fast" {
                        let _ = child.kill().await;
                        return Ok(ChildEnd::SwitchedToFast);
                    }
                }
            }
        }
        let status = child.wait().await?;
        if !status.success() {
            tracing::warn!(volume, code = ?status.code(), "indexer exited with failure");
        }
        Ok(ChildEnd::Finished)
    }

    fn on_line(&self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("progress") => {
                let num = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
                let p = json!({
                    "phase": v.get("phase").and_then(Value::as_str).unwrap_or(""),
                    "dirs": num("dirs"),
                    "files": num("files"),
                    "bytes": num("bytes"),
                    "fraction": v.get("fraction").and_then(Value::as_f64),
                });
                if let Ok(mut st) = self.state.lock() {
                    st.progress = Some(p);
                }
                self.bump();
            }
            Some("start") | Some("done") => self.bump(),
            Some("error") => {
                let msg = v.get("message").and_then(Value::as_str).unwrap_or("");
                tracing::warn!(message = msg, "indexer reported an error");
            }
            _ => {}
        }
    }
}

impl IndexControl for Supervisor {
    fn status(&self) -> Value {
        self.snapshot()
    }
    fn refresh(&self, mode: &str) -> anyhow::Result<bool> {
        self.start_refresh(mode)
    }
}
