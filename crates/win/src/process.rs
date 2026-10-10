use std::io;
use std::mem::size_of;
use std::os::windows::io::{OwnedHandle, RawHandle};

use windows::Win32::Foundation::{
    ERROR_PROCESS_MODE_ALREADY_BACKGROUND, ERROR_PROCESS_MODE_NOT_BACKGROUND, HANDLE, WIN32_ERROR,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetCurrentProcess, PROCESS_CREATION_FLAGS, PROCESS_MODE_BACKGROUND_BEGIN,
    PROCESS_MODE_BACKGROUND_END, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadInformation, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
    THREAD_MODE_BACKGROUND_END, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE, ThreadPowerThrottling,
};
use windows::core::PCWSTR;

use crate::util::{owned, raw, werr};

fn set_mode(mode: PROCESS_CREATION_FLAGS, already: WIN32_ERROR) -> io::Result<()> {
    match unsafe { SetPriorityClass(GetCurrentProcess(), mode) } {
        Ok(()) => Ok(()),
        Err(e) if WIN32_ERROR::from_error(&e) == Some(already) => Ok(()),
        Err(e) => Err(werr(e)),
    }
}

fn set_power_throttling(state_mask: u32, control_mask: u32) -> io::Result<()> {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control_mask,
        StateMask: state_mask,
    };
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(werr)
}

pub fn enter_background_mode() -> io::Result<()> {
    set_mode(
        PROCESS_MODE_BACKGROUND_BEGIN,
        ERROR_PROCESS_MODE_ALREADY_BACKGROUND,
    )?;
    set_power_throttling(
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    )
}

pub fn leave_background_mode() -> io::Result<()> {
    set_mode(
        PROCESS_MODE_BACKGROUND_END,
        ERROR_PROCESS_MODE_NOT_BACKGROUND,
    )?;
    // ControlMask 0 hands the QoS choice back to the system instead of pinning HighQoS.
    set_power_throttling(0, 0)
}

// Per thread, so background work inside the server leaves the rest of the process at normal priority.
pub fn enter_thread_background_mode() -> io::Result<()> {
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) }.map_err(werr)?;
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
    };
    unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&state as *const THREAD_POWER_THROTTLING_STATE).cast(),
            size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(werr)
}

pub fn leave_thread_background_mode() -> io::Result<()> {
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END) }.map_err(werr)?;
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: 0,
        StateMask: 0,
    };
    unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&state as *const THREAD_POWER_THROTTLING_STATE).cast(),
            size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(werr)
}

// Kept alive for the life of the process; dropping it closes the job handle.
pub struct JobGuard {
    _job: OwnedHandle,
}

pub fn cap_process_memory(bytes: u64) -> io::Result<JobGuard> {
    let job = owned(unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(werr)?);
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    info.ProcessMemoryLimit = usize::try_from(bytes).unwrap_or(usize::MAX);
    unsafe {
        SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(werr)?;
    unsafe { AssignProcessToJobObject(raw(&job), GetCurrentProcess()) }.map_err(werr)?;
    Ok(JobGuard { _job: job })
}

// Processes assigned here die when the last job handle closes, also when the owner is hard-killed.
pub struct KillOnCloseJob {
    job: OwnedHandle,
}

impl KillOnCloseJob {
    pub fn new() -> io::Result<KillOnCloseJob> {
        let job = owned(unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(werr)?);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(werr)?;
        Ok(KillOnCloseJob { job })
    }

    pub fn assign(&self, process: RawHandle) -> io::Result<()> {
        unsafe { AssignProcessToJobObject(raw(&self.job), HANDLE(process)) }.map_err(werr)
    }
}

pub fn current_working_set() -> io::Result<u64> {
    let mut pmc = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) }.map_err(werr)?;
    Ok(pmc.WorkingSetSize as u64)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessMemory {
    pub working_set: u64,
    pub peak_working_set: u64,
    pub private_bytes: u64,
}

pub fn process_memory(pid: u32) -> io::Result<ProcessMemory> {
    use windows::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };
    let h = owned(
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                false,
                pid,
            )
        }
        .map_err(werr)?,
    );
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessMemoryInfo(
            raw(&h),
            (&mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            pmc.cb,
        )
    }
    .map_err(werr)?;
    Ok(ProcessMemory {
        working_set: pmc.WorkingSetSize as u64,
        peak_working_set: pmc.PeakWorkingSetSize as u64,
        private_bytes: pmc.PrivateUsage as u64,
    })
}

// The child dies with this guard, so a crashed app never leaves an orphan behind.
pub fn kill_with_job(pid: u32) -> io::Result<JobGuard> {
    use windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
    let job = owned(unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(werr)?);
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(werr)?;
    let child = owned(
        unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }.map_err(werr)?,
    );
    unsafe { AssignProcessToJobObject(raw(&job), raw(&child)) }.map_err(werr)?;
    Ok(JobGuard { _job: job })
}

pub fn physical_cores() -> usize {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformation, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
    };
    let fallback = || {
        std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(1))
            .unwrap_or(1)
    };
    let mut len = 0u32;
    let _ = unsafe { GetLogicalProcessorInformation(None, &mut len) };
    let item = size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
    if len == 0 {
        return fallback();
    }
    let mut buf = vec![SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default(); len as usize / item + 1];
    if unsafe { GetLogicalProcessorInformation(Some(buf.as_mut_ptr()), &mut len) }.is_err() {
        return fallback();
    }
    let n = buf[..len as usize / item]
        .iter()
        .filter(|i| i.Relationship == RelationProcessorCore)
        .count();
    if n == 0 { fallback() } else { n }
}
