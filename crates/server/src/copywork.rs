use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use hoardini_core::copycheck::{self, CheckEnv, Registry};
use hoardini_core::index;
use hoardini_core::store::Db;

use crate::supervisor::Supervisor;

// Journal refreshes come every few minutes; copy checks after them need not.
const MIN_GAP: Duration = Duration::from_secs(15 * 60);

pub struct CopyWorker {
    db: Db,
    supervisor: Weak<Supervisor>,
    running: AtomicBool,
    again: AtomicBool,
    // Set when a pass stopped early or never ran; the next `looking: true` starts one.
    owed: AtomicBool,
    shutdown: AtomicBool,
    finished_at: Mutex<Option<Instant>>,
    me: Weak<CopyWorker>,
}

impl CopyWorker {
    pub fn new(db: Db, supervisor: &Arc<Supervisor>) -> Arc<CopyWorker> {
        Arc::new_cyclic(|me| CopyWorker {
            db,
            supervisor: Arc::downgrade(supervisor),
            running: AtomicBool::new(false),
            again: AtomicBool::new(false),
            owed: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            finished_at: Mutex::new(None),
            me: me.clone(),
        })
    }

    fn go(&self) -> bool {
        !self.shutdown.load(Ordering::SeqCst)
            && self.supervisor.upgrade().is_some_and(|s| s.looking())
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    pub fn looking_changed(&self, looking: bool) {
        if looking && self.owed.load(Ordering::SeqCst) {
            self.kick();
        }
    }

    pub fn after_refresh(&self) {
        let recent = self
            .finished_at
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|t| t.elapsed() < MIN_GAP);
        if !recent || self.owed.load(Ordering::SeqCst) {
            self.kick();
        }
    }

    pub fn kick(&self) {
        self.owed.store(true, Ordering::SeqCst);
        if !self.go() {
            return;
        }
        if self.running.swap(true, Ordering::SeqCst) {
            self.again.store(true, Ordering::SeqCst);
            return;
        }
        let Some(me) = self.me.upgrade() else {
            self.running.store(false, Ordering::SeqCst);
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("hoardini-copycheck".into())
            .spawn(move || {
                if let Err(e) = hoardini_win::enter_thread_background_mode() {
                    tracing::debug!(error = %e, "copy checks run at normal priority");
                }
                loop {
                    me.again.store(false, Ordering::SeqCst);
                    me.pass();
                    if me.again.load(Ordering::SeqCst) && me.go() {
                        continue;
                    }
                    me.running.store(false, Ordering::SeqCst);
                    // A kick between the check above and this point saw `running` and only raised `again`.
                    if me.again.load(Ordering::SeqCst)
                        && me.go()
                        && !me.running.swap(true, Ordering::SeqCst)
                    {
                        continue;
                    }
                    break;
                }
            });
        if let Err(e) = spawned {
            self.running.store(false, Ordering::SeqCst);
            tracing::warn!(error = %e, "copy checks did not start");
        }
    }

    fn pass(&self) {
        let Ok(conn) = self.db.conn() else {
            return;
        };
        // Cleared now, not at the end, so a kick during this pass stays owed if the pass ends unwatched.
        self.owed.store(false, Ordering::SeqCst);
        let env = CheckEnv::system(&conn);
        let reg = Registry::standard();
        let go = || self.go();
        let mut finished = true;
        for root in index::watched(&conn) {
            if !go() {
                finished = false;
                break;
            }
            match copycheck::run_volume(&self.db, &env, &reg, &root, &go) {
                Ok(r) => {
                    finished &= !r.stopped;
                    tracing::info!(
                        volume = %root,
                        pruned = r.pruned,
                        candidates = r.candidates,
                        checked = r.checked,
                        hashed = r.hashed,
                        hashed_bytes = r.hashed_bytes,
                        online_waiting = r.online_waiting,
                        stopped = r.stopped,
                        "copy checks done"
                    );
                }
                Err(e) => tracing::warn!(volume = %root, error = %e, "copy checks failed"),
            }
        }
        if finished {
            if let Ok(mut g) = self.finished_at.lock() {
                *g = Some(Instant::now());
            }
        } else {
            self.owed.store(true, Ordering::SeqCst);
        }
    }
}
