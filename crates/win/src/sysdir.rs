use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

pub fn system_directory() -> io::Result<PathBuf> {
    let mut buf = vec![0u16; 260];
    loop {
        let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n < buf.len() {
            return Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])));
        }
        buf.resize(n + 1, 0);
    }
}

pub fn system_tool(name: &str) -> io::Result<PathBuf> {
    let leaf = Path::new(name);
    if leaf.components().count() != 1 || leaf.file_name() != Some(leaf.as_os_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tool name must be a bare file name",
        ));
    }
    Ok(system_directory()?.join(leaf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_compact_in_system_directory() {
        let p = system_tool("compact.exe").unwrap();
        assert!(p.is_absolute());
        assert_eq!(p.parent().unwrap(), system_directory().unwrap());
        assert!(p.is_file(), "{}", p.display());
    }

    #[test]
    fn rejects_paths_and_traversal() {
        for bad in ["..\\evil.exe", "sub\\x.exe", "C:\\x.exe", "", ".."] {
            assert!(system_tool(bad).is_err(), "{bad}");
        }
    }
}
