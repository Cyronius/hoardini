use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Phase {
    List,
    Wof,
    Wait,
    Apply,
    Children,
    Write,
    Commit,
    Gone,
    Groups,
    Summary,
}

const PHASES: [Phase; 10] = [
    Phase::List,
    Phase::Wof,
    Phase::Wait,
    Phase::Apply,
    Phase::Children,
    Phase::Write,
    Phase::Commit,
    Phase::Gone,
    Phase::Groups,
    Phase::Summary,
];

static NANOS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static COUNTS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

// HOARDINI_INDEX_PROFILE=1: a full refresh prints wall time per phase (summed over threads) to stderr.
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("HOARDINI_INDEX_PROFILE").is_some())
}

pub(crate) struct Span(Option<(Phase, Instant)>);

pub(crate) fn span(p: Phase) -> Span {
    Span(on().then(|| (p, Instant::now())))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((p, t)) = self.0 {
            NANOS[p as usize].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
            COUNTS[p as usize].fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) fn report(label: &str) {
    if !on() {
        return;
    }
    eprintln!("profile {label}:");
    for p in PHASES {
        let n = NANOS[p as usize].swap(0, Ordering::Relaxed);
        let c = COUNTS[p as usize].swap(0, Ordering::Relaxed);
        if c > 0 {
            eprintln!(
                "  {:<10} {:>10.3} s {:>10} calls {:>8.1} us/call",
                format!("{p:?}"),
                n as f64 / 1e9,
                c,
                n as f64 / 1e3 / c as f64
            );
        }
    }
}
