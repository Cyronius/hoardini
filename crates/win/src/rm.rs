use std::io;
use std::path::PathBuf;

use windows::Win32::Foundation::ERROR_MORE_DATA;
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmStartSession,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::ProcessUse;
use crate::util::{from_wide, owned, raw, wide, win32};

struct Session(u32);

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let _ = RmEndSession(self.0);
        }
    }
}

// Restart Manager: processes holding any of these files open. Files only, not directories.
pub fn processes_using(files: &[PathBuf]) -> io::Result<Vec<ProcessUse>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let wides = files
        .iter()
        .map(|f| wide(std::path::absolute(f)?.as_os_str()))
        .collect::<io::Result<Vec<_>>>()?;
    let ptrs: Vec<PCWSTR> = wides.iter().map(|w| PCWSTR(w.as_ptr())).collect();

    let mut handle = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    win32(unsafe { RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr())) })?;
    let session = Session(handle);
    win32(unsafe { RmRegisterResources(session.0, Some(&ptrs), None, None) })?;

    let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
    loop {
        let mut needed = 0u32;
        let mut count = infos.len() as u32;
        let mut reasons = 0u32;
        let rc = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut count,
                if infos.is_empty() {
                    None
                } else {
                    Some(infos.as_mut_ptr())
                },
                &mut reasons,
            )
        };
        if rc == ERROR_MORE_DATA {
            // The list can grow between calls, so leave headroom and retry.
            infos = vec![RM_PROCESS_INFO::default(); needed as usize + 4];
            continue;
        }
        win32(rc)?;
        infos.truncate(count as usize);
        break;
    }

    Ok(infos
        .iter()
        .map(|p| ProcessUse {
            pid: p.Process.dwProcessId,
            app_name: from_wide(&p.strAppName),
            exe: exe_of(p.Process.dwProcessId),
        })
        .collect())
}

fn exe_of(pid: u32) -> Option<String> {
    let h = owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            raw(&h),
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}
