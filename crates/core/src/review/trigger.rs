use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use rusqlite::Connection;

use super::{Review, latest, needs_rerun};

pub enum Opened {
    Ready(Review),
    // What to show until the build that is running ends; none before the drive's first review.
    Building(Option<Review>),
}

type Clock = Box<dyn Fn() -> i64 + Send + Sync>;
type Done = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct State {
    running: bool,
    // Between a build ending and its announcement, so wait() returns only once it is announced.
    announcing: bool,
}

// One review build at a time, off the caller's thread, so opening the app never waits on Restart Manager.
pub struct Builds {
    running: Mutex<State>,
    idle: Condvar,
    clock: Clock,
    done: Done,
}

struct Finish<'a>(&'a Builds);

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        {
            let mut st = self.0.lock();
            st.running = false;
            st.announcing = true;
        }
        (self.0.done)();
        self.0.lock().announcing = false;
        self.0.idle.notify_all();
    }
}

impl Builds {
    pub fn new(
        clock: impl Fn() -> i64 + Send + Sync + 'static,
        done: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Builds> {
        Arc::new(Builds {
            running: Mutex::new(State::default()),
            idle: Condvar::new(),
            clock: Box::new(clock),
            done: Box::new(done),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    pub fn building(&self) -> bool {
        self.lock().running
    }

    // False when a build is already running; that one's result is what the caller will see.
    pub fn start<F>(self: &Arc<Self>, job: F) -> bool
    where
        F: FnOnce(i64) -> anyhow::Result<()> + Send + 'static,
    {
        {
            let mut st = self.lock();
            if st.running {
                return false;
            }
            st.running = true;
        }
        let me = self.clone();
        let now = self.now();
        let spawned = std::thread::Builder::new()
            .name("hoardini-review".into())
            .spawn(move || {
                let _f = Finish(&me);
                if let Err(e) = job(now) {
                    tracing::warn!(error = %e, "review build failed");
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "review build did not start");
            drop(Finish(self));
            return false;
        }
        true
    }

    // Waits for a running build, then runs `job` here while holding others off.
    pub fn exclusive<T>(&self, job: impl FnOnce(i64) -> T) -> T {
        {
            let mut st = self.lock();
            while st.running {
                st = self.idle.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            st.running = true;
        }
        let _f = Finish(self);
        job(self.now())
    }

    pub fn wait(&self) {
        let mut st = self.lock();
        while st.running || st.announcing {
            st = self.idle.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    pub fn open<F>(
        self: &Arc<Self>,
        conn: &Connection,
        volume_id: i64,
        job: F,
    ) -> anyhow::Result<Opened>
    where
        F: FnOnce(i64) -> anyhow::Result<()> + Send + 'static,
    {
        let current = latest(conn, volume_id, None)?;
        if let Some(r) = current {
            if !needs_rerun(conn, &r, self.now())? {
                return Ok(Opened::Ready(r));
            }
            self.start(job);
            return Ok(Opened::Building(Some(r)));
        }
        self.start(job);
        Ok(Opened::Building(None))
    }
}
