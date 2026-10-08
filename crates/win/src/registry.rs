use std::io;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, REG_SAM_FLAGS,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegGetValueW,
    RegOpenKeyExW, RegQueryInfoKeyW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::util::{from_wide, wide_str, win32};
use crate::{InstalledProgram, LastAccessSetting, UserAssistEntry, filetime_to_unix_ms};

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl Key {
    fn open(parent: HKEY, sub: &str, sam: REG_SAM_FLAGS) -> io::Result<Key> {
        let w = wide_str(sub)?;
        let mut h = HKEY::default();
        win32(unsafe { RegOpenKeyExW(parent, PCWSTR(w.as_ptr()), None, sam, &mut h) })?;
        Ok(Key(h))
    }

    fn subkeys(&self) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        let mut buf = [0u16; 256];
        for i in 0.. {
            let mut len = buf.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    self.0,
                    i,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            win32(rc)?;
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        Ok(out)
    }

    fn string(&self, sub: Option<&str>, name: &str) -> Option<String> {
        let sub_w = sub.map(wide_str).transpose().ok()?;
        let sub_p = sub_w
            .as_ref()
            .map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr()));
        let name_w = wide_str(name).ok()?;
        let mut buf: Vec<u16> = vec![0; 256];
        loop {
            let mut bytes = (buf.len() * 2) as u32;
            let rc = unsafe {
                RegGetValueW(
                    self.0,
                    sub_p,
                    PCWSTR(name_w.as_ptr()),
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr().cast()),
                    Some(&mut bytes),
                )
            };
            if rc == ERROR_MORE_DATA {
                buf = vec![0; (bytes as usize).div_ceil(2) + 1];
                continue;
            }
            if rc.is_err() {
                return None;
            }
            let s = from_wide(&buf[..(bytes as usize / 2).min(buf.len())]);
            let s = s.trim();
            return if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            };
        }
    }

    fn values(&self) -> io::Result<Vec<(Vec<u16>, Vec<u8>)>> {
        let (mut max_name, mut max_data) = (0u32, 0u32);
        win32(unsafe {
            RegQueryInfoKeyW(
                self.0,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&mut max_name),
                Some(&mut max_data),
                None,
                None,
            )
        })?;
        let mut name = vec![0u16; max_name as usize + 1];
        let mut data = vec![0u8; max_data as usize];
        let mut out = Vec::new();
        for i in 0.. {
            let mut name_len = name.len() as u32;
            let mut data_len = data.len() as u32;
            let rc = unsafe {
                RegEnumValueW(
                    self.0,
                    i,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut name_len,
                    None,
                    None,
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc == ERROR_MORE_DATA {
                // A value grew after RegQueryInfoKeyW; skip it rather than loop on a moving target.
                continue;
            }
            win32(rc)?;
            out.push((
                name[..name_len as usize].to_vec(),
                data[..data_len as usize].to_vec(),
            ));
        }
        Ok(out)
    }
}

pub fn last_access_setting() -> io::Result<LastAccessSetting> {
    let key = wide_str(r"SYSTEM\CurrentControlSet\Control\FileSystem")?;
    let name = wide_str("NtfsDisableLastAccessUpdate")?;
    let mut raw = 0u32;
    let mut bytes = 4u32;
    win32(unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut raw as *mut u32).cast()),
            Some(&mut bytes),
        )
    })?;
    Ok(decode_last_access(raw))
}

pub(crate) fn decode_last_access(raw: u32) -> LastAccessSetting {
    LastAccessSetting {
        raw,
        enabled: raw & 1 == 0,
        system_managed: raw & 0x8000_0000 != 0 && raw & 2 != 0,
    }
}

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

pub fn installed_programs() -> io::Result<Vec<InstalledProgram>> {
    let wow = format!(r"SOFTWARE\WOW6432Node\{}", &UNINSTALL["SOFTWARE\\".len()..]);
    let roots = [
        (HKEY_LOCAL_MACHINE, "HKLM", UNINSTALL.to_string()),
        (HKEY_LOCAL_MACHINE, "HKLM", wow),
        (HKEY_CURRENT_USER, "HKCU", UNINSTALL.to_string()),
    ];
    let mut out = Vec::new();
    for (hive, hive_name, path) in roots {
        let key = match Key::open(hive, &path, KEY_READ | KEY_WOW64_64KEY) {
            Ok(k) => k,
            Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND.0 as i32) => continue,
            Err(e) => return Err(e),
        };
        for sub in key.subkeys()? {
            let Some(name) = key.string(Some(&sub), "DisplayName") else {
                continue;
            };
            out.push(InstalledProgram {
                name,
                publisher: key.string(Some(&sub), "Publisher"),
                version: key.string(Some(&sub), "DisplayVersion"),
                install_location: key
                    .string(Some(&sub), "InstallLocation")
                    .map(|s| s.trim_matches('"').to_string())
                    .filter(|s| !s.is_empty()),
                uninstall_key: format!(r"{hive_name}\{path}\{sub}"),
            });
        }
    }
    Ok(out)
}

const USER_ASSIST: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist";

pub fn user_assist() -> io::Result<Vec<UserAssistEntry>> {
    let root = match Key::open(HKEY_CURRENT_USER, USER_ASSIST, KEY_READ) {
        Ok(k) => k,
        Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND.0 as i32) => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for guid in root.subkeys()? {
        let Ok(count) = Key::open(root.0, &format!(r"{guid}\Count"), KEY_READ) else {
            continue;
        };
        for (name, data) in count.values()? {
            if let Some(e) = parse_user_assist(&name, &data) {
                out.push(e);
            }
        }
    }
    Ok(out)
}

pub(crate) fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a') + 13) % 26 + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A') + 13) % 26 + b'A') as char,
            _ => c,
        })
        .collect()
}

pub(crate) fn parse_user_assist(name: &[u16], data: &[u8]) -> Option<UserAssistEntry> {
    if data.len() != 72 {
        return None;
    }
    let run_count = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let ft = u64::from_le_bytes(data[60..68].try_into().ok()?);
    Some(UserAssistEntry {
        name: rot13(&String::from_utf16_lossy(name)),
        run_count,
        last_run_ms: (ft != 0).then(|| filetime_to_unix_ms(ft)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_access_values_decode() {
        let d = |r| {
            let s = decode_last_access(r);
            (s.enabled, s.system_managed)
        };
        assert_eq!(d(0x8000_0000), (true, false));
        assert_eq!(d(0x8000_0001), (false, false));
        assert_eq!(d(0x8000_0002), (true, true));
        assert_eq!(d(0x8000_0003), (false, true));
        assert_eq!(d(0), (true, false));
        assert_eq!(d(1), (false, false));
    }

    #[test]
    fn rot13_round_trips_and_keeps_digits() {
        assert_eq!(
            rot13(r"{6Q809377-6NS0-444O-8957-N3773S02200R}\Abgrcnq++\abgrcnq++.rkr"),
            r"{6D809377-6AF0-444B-8957-A3773F02200E}\Notepad++\notepad++.exe"
        );
        assert_eq!(rot13(&rot13("UEME_CTLSESSION")), "UEME_CTLSESSION");
    }

    #[test]
    fn user_assist_record_parses_count_and_last_run() {
        let mut data = [0u8; 72];
        data[4..8].copy_from_slice(&7u32.to_le_bytes());
        // 2024-01-01T00:00:00Z as FILETIME.
        let ft: u64 = 133_485_408_000_000_000;
        data[60..68].copy_from_slice(&ft.to_le_bytes());
        let name: Vec<u16> = "P:\\Gbbyf\\n.rkr".encode_utf16().collect();
        let e = parse_user_assist(&name, &data).unwrap();
        assert_eq!(e.name, "C:\\Tools\\a.exe");
        assert_eq!(e.run_count, 7);
        assert_eq!(e.last_run_ms, Some(1_704_067_200_000));

        data[60..68].fill(0);
        assert_eq!(parse_user_assist(&name, &data).unwrap().last_run_ms, None);
        assert!(parse_user_assist(&name, &data[..16]).is_none());
    }
}
