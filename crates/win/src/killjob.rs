use std::io;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::OnceLock;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::core::PCWSTR;

use crate::util::{owned, raw, werr};

static JOB: OnceLock<Result<OwnedHandle, String>> = OnceLock::new();

fn job() -> io::Result<&'static OwnedHandle> {
    JOB.get_or_init(|| {
        let make = || -> io::Result<OwnedHandle> {
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
            Ok(job)
        };
        make().map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| io::Error::other(e.clone()))
}

// The child dies with this process, even on a crash, because the job handle closes with it.
pub fn kill_child_on_exit(child: &impl AsRawHandle) -> io::Result<()> {
    let job = job()?;
    let h = HANDLE(child.as_raw_handle());
    unsafe { AssignProcessToJobObject(raw(job), h) }.map_err(werr)
}
