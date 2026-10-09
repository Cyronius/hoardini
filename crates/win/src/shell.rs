use std::io;
use std::mem::size_of;
use std::path::Path;

use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHERB_NOCONFIRMATION,
    SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO, SHQueryRecycleBinW,
    ShellExecuteExW,
};
use windows::core::PCWSTR;

use crate::util::{owned, raw, werr, wide, wide_str};

const SW_HIDE: i32 = 0;
const SW_SHOWNORMAL: i32 = 1;

// The shell resolves the file (or URL) and shows its own UAC prompt when the target's manifest asks for one.
pub fn shell_open(file: &str, params: Option<&str>) -> io::Result<()> {
    let verb = wide_str("open")?;
    let file_w = wide_str(file)?;
    let params_w = params.map(wide_str).transpose()?;
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: params_w
            .as_ref()
            .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(werr)
}

// One UAC prompt; a declined prompt is ERROR_CANCELLED (raw OS error 1223).
pub fn run_elevated(exe: &Path, params: &str, mut tick: impl FnMut()) -> io::Result<u32> {
    let verb = wide_str("runas")?;
    let exe_w = wide(exe.as_os_str())?;
    let params_w = wide_str(params)?;
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe_w.as_ptr()),
        lpParameters: PCWSTR(params_w.as_ptr()),
        nShow: SW_HIDE,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(werr)?;
    if info.hProcess.is_invalid() {
        return Err(io::Error::other("the elevated helper did not start"));
    }
    let process = owned(info.hProcess);
    loop {
        match unsafe { WaitForSingleObject(raw(&process), 500) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => tick(),
            _ => return Err(io::Error::last_os_error()),
        }
    }
    let mut code = 0u32;
    unsafe { GetExitCodeProcess(raw(&process), &mut code) }.map_err(werr)?;
    Ok(code)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecycleBin {
    pub bytes: u64,
    pub items: u64,
}

// Every drive's bin for the current user.
pub fn recycle_bin() -> io::Result<RecycleBin> {
    let mut info = SHQUERYRBINFO {
        cbSize: size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.map_err(werr)?;
    Ok(RecycleBin {
        bytes: u64::try_from(info.i64Size).unwrap_or(0),
        items: u64::try_from(info.i64NumItems).unwrap_or(0),
    })
}

pub fn empty_recycle_bin() -> io::Result<()> {
    unsafe {
        SHEmptyRecycleBinW(
            None,
            PCWSTR::null(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    }
    .map_err(werr)
}

pub fn physical_memory() -> io::Result<u64> {
    let mut m = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut m) }.map_err(werr)?;
    Ok(m.ullTotalPhys)
}
