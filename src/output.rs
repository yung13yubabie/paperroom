use crate::store::Result;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

// Keep staging on the destination volume. Each operation owns its directory,
// so concurrent exports and leftovers from a terminated process stay separate.
pub fn publish_new(
    destination: &Path,
    write: impl FnOnce(&mut File, &Path) -> Result<()>,
) -> Result<()> {
    let parent = destination.parent().ok_or("找不到目標資料夾。")?;
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
