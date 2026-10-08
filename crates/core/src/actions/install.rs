use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::{Executor, Job, Live, finish_job, insert_job, new_job};
use crate::runtime::{self, Pinned};

const PUBLISH_EVERY: Duration = Duration::from_millis(200);

impl Executor {
    // A runtime download shows up as an ordinary job so the UI can draw its progress bar.
    pub fn install_runtime(&self, pin: &'static Pinned, root: PathBuf) -> anyhow::Result<Job> {
        if let Some(l) =
            self.shared.live.lock().values().find(|l| {
                l.job.kind == "install" && l.job.progress.current.as_deref() == Some(pin.id)
            })
        {
            return Ok(l.job.clone());
        }
        let mut job = new_job("install", None);
        job.status = "running".into();
        job.progress.total_bytes = crate::index::to_i64(pin.size);
        job.progress.total_items = 1;
        job.progress.current = Some(pin.id.to_string());
        insert_job(&self.db.conn()?, &job)?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.shared.live.lock().insert(
            job.id.clone(),
            Live {
                job: job.clone(),
                cancel: cancel.clone(),
            },
        );
        let ctx = self.ctx();
        let thread_job = job.clone();
        let spawned = std::thread::Builder::new()
            .name("hoardini-install".into())
            .spawn(move || {
                let conn = ctx.db.conn().ok();
                let state = Mutex::new((thread_job, Instant::now()));
                let progress = |done: u64, total: u64| {
                    let mut g = state.lock();
                    g.0.progress.done_bytes = crate::index::to_i64(done);
                    g.0.progress.total_bytes = crate::index::to_i64(total);
                    if g.1.elapsed() >= PUBLISH_EVERY {
                        g.1 = Instant::now();
                        ctx.publish(None, &g.0);
                    }
                };
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(anyhow::Error::from)
                    .and_then(|rt| rt.block_on(runtime::ensure(pin, &root, &progress, &cancel)));
                let mut job = state.into_inner().0;
                match result {
                    Ok(_) => {
                        job.status = "done".into();
                        job.progress.done_items = 1;
                        job.progress.done_bytes = job.progress.total_bytes;
                    }
                    Err(e) => {
                        job.status = "failed".into();
                        job.error = Some(format!("{e:#}"));
                    }
                }
                finish_job(&ctx, conn.as_ref(), &mut job);
            });
        if let Err(e) = spawned {
            self.shared.live.lock().remove(&job.id);
            anyhow::bail!("could not start the download: {e}");
        }
        Ok(job)
    }
}
