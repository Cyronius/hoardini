use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use rusqlite::Connection;

use super::{Review, latest, needs_rerun};

pub enum Opened {
    Ready(Review),
    Building(Option<Review>),
}

type Clock = Box<dyn Fn() -> i64 + Send + Sync>;
type Done = Box<dyn Fn() + Send + Sync>;
type Job = Box<dyn FnOnce(i64) -> anyhow::Result<()> + Send>;

#[derive(Default)]
struct State {
    running: Option<String>,
    queued: Option<(String, Job)>,
    // Between a build ending and its announcement, so wait() returns only once it is announced.
    announcing: bool,
}

// One review build at a time, off the caller's thread, so opening the app never waits on Restart Manager.
pub struct Builds {
    state: Mutex<State>,
    idle: Condvar,
    clock: Clock,
    done: Done,
}

struct Finish<'a>(&'a Arc<Builds>);

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        self.0.finished();
    }
}

impl Builds {
    pub fn new(
        clock: impl Fn() -> i64 + Send + Sync + 'static,
        done: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Builds> {
        Arc::new(Builds {
            state: Mutex::new(State::default()),
            idle: Condvar::new(),
            clock: Box::new(clock),
            done: Box::new(done),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    pub fn building(&self) -> bool {
        let st = self.lock();
        st.running.is_some() || st.queued.is_some()
    }

    pub fn start<F>(self: &Arc<Self>, key: &str, job: F) -> bool
    where
        F: FnOnce(i64) -> anyhow::Result<()> + Send + 'static,
    {
        let mut st = self.lock();
        match &st.running {
            None => {
                st.running = Some(key.to_string());
                drop(st);
                self.spawn(Box::new(job));
                true
            }
            Some(k) if k == key => false,
            Some(_) if st.queued.as_ref().is_some_and(|(q, _)| q == key) => false,
            Some(_) => {
                st.queued = Some((key.to_string(), Box::new(job)));
                true
            }
        }
    }

    fn spawn(self: &Arc<Self>, job: Job) {
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("hoardini-review".into())
            .spawn(move || {
                let _f = Finish(&me);
                if let Err(e) = job(me.now()) {
                    tracing::warn!(error = %e, "review build failed");
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "review build did not start");
            self.finished();
        }
    }

    // A build that queued meanwhile starts before listeners hear of this one, so `building` never blinks off.
    fn finished(self: &Arc<Self>) {
        let next = {
            let mut st = self.lock();
            st.announcing = true;
            match st.queued.take() {
                Some((key, job)) => {
                    st.running = Some(key);
                    Some(job)
                }
                None => {
                    st.running = None;
                    None
                }
            }
        };
        (self.done)();
        self.lock().announcing = false;
        self.idle.notify_all();
        if let Some(job) = next {
            self.spawn(job);
        }
    }

    pub fn exclusive<T>(self: &Arc<Self>, job: impl FnOnce(i64) -> T) -> T {
        {
            let mut st = self.lock();
            while st.running.is_some() {
                st = self.idle.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            st.running = Some(String::new());
        }
        let _f = Finish(self);
        job(self.now())
    }

    pub fn wait(&self) {
        let mut st = self.lock();
        while st.running.is_some() || st.queued.is_some() || st.announcing {
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
        let current = latest(conn, volume_id)?;
        if let Some(r) = &current
            && !needs_rerun(conn, r, self.now())?
        {
            return Ok(Opened::Ready(r.clone()));
        }
        self.start(&format!("volume:{volume_id}"), job);
        Ok(Opened::Building(current))
    }
}
