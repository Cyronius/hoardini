use std::io;

use windows::Win32::Foundation::{ERROR_EXTENDED_ERROR, NO_ERROR};
use windows::Win32::NetworkManagement::WNet::{
    NET_CONNECT_FLAGS, NETRESOURCEW, RESOURCETYPE_DISK, WNetAddConnection2W, WNetGetLastErrorW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::util::{from_wide, wide_str};

// WNetAddConnection2W only accepts \\server\share, so a deeper destination path is trimmed to it.
pub(crate) fn share_root(unc: &str) -> Option<String> {
    let rest = unc.strip_prefix(r"\\")?;
    let mut parts = rest.split(['\\', '/']).filter(|s| !s.is_empty());
    let server = parts.next()?;
    let share = parts.next()?;
    Some(format!(r"\\{server}\{share}"))
}

// WNetAddConnection2W; None credentials use the current Windows session.
pub fn smb_connect(unc: &str, user: Option<&str>, password: Option<&str>) -> io::Result<()> {
    let root = share_root(unc).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a UNC share path: {unc}"),
        )
    })?;
    let mut remote = wide_str(&root)?;
    let user_w = user.map(wide_str).transpose()?;
    let pass_w = password.map(wide_str).transpose()?;
    let nr = NETRESOURCEW {
        dwType: RESOURCETYPE_DISK,
        lpRemoteName: PWSTR(remote.as_mut_ptr()),
        ..Default::default()
    };
    let rc = unsafe {
        WNetAddConnection2W(
            &nr,
            pass_w
                .as_ref()
                .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
            user_w
                .as_ref()
                .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
            NET_CONNECT_FLAGS(0),
        )
    };
    if rc == NO_ERROR {
        return Ok(());
    }
    if rc == ERROR_EXTENDED_ERROR {
        let mut code = 0u32;
        let mut msg = [0u16; 512];
        let mut provider = [0u16; 128];
        if unsafe { WNetGetLastErrorW(&mut code, &mut msg, &mut provider) } == NO_ERROR {
            return Err(io::Error::other(format!(
                "{}: {} (code {code})",
                from_wide(&provider),
                from_wide(&msg)
            )));
        }
    }
    Err(io::Error::from_raw_os_error(rc.0 as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_root_trims_to_server_and_share() {
        assert_eq!(
            share_root(r"\\nas\backup\hoardini\x").as_deref(),
            Some(r"\\nas\backup")
        );
        assert_eq!(
            share_root(r"\\nas\backup").as_deref(),
            Some(r"\\nas\backup")
        );
        assert_eq!(
            share_root(r"\\nas\backup\").as_deref(),
            Some(r"\\nas\backup")
        );
        assert_eq!(share_root(r"\\nas"), None);
        assert_eq!(share_root(r"C:\x\y"), None);
    }
}
