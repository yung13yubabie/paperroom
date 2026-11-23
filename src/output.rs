use crate::store::Result;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(windows, test))]
fn windows_output_name(name: &std::ffi::OsStr) -> Result<&std::ffi::OsStr> {
    let text = name.to_string_lossy();
    let stem = text.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    let port = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"));
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || matches!(port, Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    if reserved || text.is_empty() || text.ends_with([' ', '.'])
        || text.chars().any(|c| c < ' ' || r#"<>:"/\|?*"#.contains(c))
    {
        return Err("請選用 Windows 可辨識的一般檔名，並移除結尾的空格或句點。".into());
    }
    Ok(name)
}

// Keep staging on the destination volume. Each operation owns its directory,
// so concurrent exports and leftovers from a terminated process stay separate.
pub fn publish_new(
    destination: &Path,
    write: impl FnOnce(&mut File, &Path) -> Result<()>,
) -> Result<()> {
    let parent = destination.parent().ok_or("找不到目標資料夾。")?;
    // Rust returns an extended-length absolute path on Windows. Use the same
    // parent for SQLite staging and the Win32 move, including on UNC shares.
    #[cfg(windows)]
    let parent = fs::canonicalize(if parent.as_os_str().is_empty() { Path::new(".") } else { parent })?;
    #[cfg(windows)]
    let destination = parent.join(windows_output_name(destination.file_name().ok_or("找不到目標檔名。")?)?);
    #[cfg(windows)]
    let destination = destination.as_path();
    if destination.try_exists()? {
        return Err(std::io::Error::from(ErrorKind::AlreadyExists).into());
    }
    let directory = loop {
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!(".paperroom-{}-{sequence}.partial", std::process::id());
        let directory = parent.join(name);
        match fs::create_dir(&directory) {
            Ok(()) => break directory,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    let staging = directory.join("output");
    let result = (|| {
        let mut file = File::create_new(&staging)?;
        write(&mut file, &staging)?;
        file.sync_all()?;
        drop(file);
        // Windows rename replaces an existing file. A move without the
        // replace flag publishes the complete file only if the name is free.
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
            }
            let native = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
            if unsafe { MoveFileW(native(&staging).as_ptr(), native(destination).as_ptr()) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        fs::hard_link(&staging, destination)?;
        Ok(())
    })();
    let _ = fs::remove_file(&staging);
    let _ = fs::remove_dir(&directory);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn windows_leaf_names_remain_accessible_in_explorer() {
        use std::ffi::OsStr;
        for name in ["筆記.md", ".archive", "COM10.md", "研究 草稿.sqlite3"] {
            assert!(windows_output_name(OsStr::new(name)).is_ok());
        }
        for name in ["筆記.md.", "筆記.md ", "AUX.md", "Lpt².txt", "NUL .md", "note:part"] {
            assert!(windows_output_name(OsStr::new(name)).is_err());
        }
    }

    #[test]
    fn failure_and_late_destination_do_not_publish_partial_output() -> Result<()> {
        let root = std::env::temp_dir().join(format!("paperroom-output-test-{}", std::process::id()));
        fs::create_dir(&root)?;
        let destination = root.join("notes.md");
        assert!(publish_new(&destination, |f, _| {
            f.write_all(b"unfinished")?;
            Err("interrupted".into())
        }).is_err());
        assert_eq!(root.read_dir()?.count(), 0);
        assert!(publish_new(&destination, |f, _| {
            f.write_all(b"new export")?;
            fs::write(&destination, b"existing")?;
            Ok(())
        }).is_err());
        assert_eq!(fs::read(&destination)?, b"existing");
        assert_eq!(root.read_dir()?.count(), 1);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
