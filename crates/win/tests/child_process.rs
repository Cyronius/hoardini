// harness = false: the binary re-invokes itself so the cap and priority changes never touch the runner.
use std::process::{Command, ExitCode};

use hoardini_win::*;

const ROLE: &str = "HOARDINI_WIN_CHILD";
const CAP: u64 = 64 << 20;

fn spawn(role: &str) -> (bool, String) {
    let out = Command::new(std::env::current_exe().unwrap())
        .env(ROLE, role)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn memory_priority() -> u32 {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessInformation, MEMORY_PRIORITY_INFORMATION,
        ProcessMemoryPriority,
    };
    let mut info = MEMORY_PRIORITY_INFORMATION::default();
    unsafe {
        GetProcessInformation(
            GetCurrentProcess(),
            ProcessMemoryPriority,
            (&mut info as *mut MEMORY_PRIORITY_INFORMATION).cast(),
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
    }
    .unwrap();
    info.MemoryPriority.0
}

fn power_throttling() -> (u32, u32) {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessInformation, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling,
    };
    let mut s = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..Default::default()
    };
    unsafe {
        GetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&mut s as *mut PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .unwrap();
    (s.ControlMask, s.StateMask)
}

fn child_background() {
    let normal = memory_priority();
    enter_background_mode().unwrap();
    enter_background_mode().unwrap();
    let bg = memory_priority();
    let eco = power_throttling();
    leave_background_mode().unwrap();
    leave_background_mode().unwrap();
    let after = memory_priority();
    let reset = power_throttling();
    println!("memory priority {normal} -> {bg} -> {after}; throttling {eco:?} -> {reset:?}");
    assert!(bg < normal, "background mode lowers memory priority");
    assert_eq!(eco, (1, 1), "EcoQoS execution-speed throttling on");
    assert_eq!(after, normal);
    assert_eq!(reset, (0, 0));
}

fn child_memcap() {
    let _job = cap_process_memory(CAP).unwrap();
    let mut small: Vec<u8> = Vec::new();
    small.try_reserve_exact(8 << 20).unwrap();
    small.resize(8 << 20, 1);
    let ws = current_working_set().unwrap();
    assert!(ws >= 8 << 20, "working set {ws} includes touched pages");
    let mut big: Vec<u8> = Vec::new();
    assert!(
        big.try_reserve_exact(256 << 20).is_err(),
        "commit beyond the job cap must fail"
    );
    println!("capped at {CAP}, working set {ws}, 256 MB reserve refused");
}

// The indexer caps itself in its own job, so the supervisor's kill-on-close job must still reach it.
fn child_capped_sleeper() {
    let _job = cap_process_memory(CAP).unwrap();
    println!("capped");
    std::thread::sleep(std::time::Duration::from_secs(60));
}

fn kill_on_close() -> bool {
    use std::io::{BufRead, BufReader};
    use std::os::windows::io::AsRawHandle;
    use std::time::{Duration, Instant};

    let job = KillOnCloseJob::new().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .env(ROLE, "capped_sleeper")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    job.assign(child.as_raw_handle()).unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "capped");
    assert!(child.try_wait().unwrap().is_none(), "sleeper is running");
    drop(job);
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    false
}

fn main() -> ExitCode {
    match std::env::var(ROLE).as_deref() {
        Ok("background") => child_background(),
        Ok("memcap") => child_memcap(),
        Ok("capped_sleeper") => child_capped_sleeper(),
        _ => {
            let killed = kill_on_close();
            println!(
                "kill-on-close job: {}",
                if killed { "ok" } else { "FAILED" }
            );
            if !killed {
                return ExitCode::FAILURE;
            }
            let mut failed = 0;
            for role in ["background", "memcap"] {
                let (ok, out) = spawn(role);
                println!(
                    "child {role}: {} {}",
                    if ok { "ok" } else { "FAILED" },
                    out.trim()
                );
                failed += usize::from(!ok);
            }
            if failed > 0 {
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
