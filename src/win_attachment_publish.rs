//! Publish an attachment relative to its already validated and pinned parent.
use crate::store::Result;
use std::{
    ffi::{OsStr, c_void}, fs::File, io::{self, Write},
    mem::{offset_of, size_of},
    os::windows::{ffi::OsStrExt, io::{AsRawHandle, FromRawHandle}},
    ptr::{null, null_mut},
    sync::atomic::{AtomicUsize, Ordering},
};

type Handle = *mut c_void;
static NEXT: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
struct UnicodeString { length: u16, maximum_length: u16, buffer: *const u16 }

#[repr(C)]
struct ObjectAttributes {
    length: u32, root_directory: Handle, object_name: *const UnicodeString,
    attributes: u32, security_descriptor: *const c_void, security_qos: *const c_void,
}

#[repr(C)]
struct IoStatus { status_or_pointer: usize, information: usize }

#[repr(C)]
struct RenameInformation {
    replace_if_exists: u8, root_directory: Handle, file_name_length: u32,
    file_name: [u16; 1],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        handle: *mut Handle, desired_access: u32, attributes: *const ObjectAttributes,
        status: *mut IoStatus, allocation_size: *const i64, file_attributes: u32,
        share_access: u32, create_disposition: u32, create_options: u32,
        ea_buffer: *const c_void, ea_length: u32,
    ) -> i32;
    fn NtSetInformationFile(
        file: Handle, status: *mut IoStatus, info: *const c_void,
        length: u32, class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

fn status_result(status: i32) -> io::Result<()> {
    if status >= 0 { Ok(()) }
    else { Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)) }
}

fn create_stage(parent: &File, name: &str) -> io::Result<File> {
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, SYNCHRONIZE,
    };
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    const OBJ_DONT_REPARSE: u32 = 0x1000;
    const FILE_CREATE: u32 = 2;
    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x40;

    let wide: Vec<u16> = name.encode_utf16().collect();
    let len = (wide.len() * size_of::<u16>()) as u16;
    let name = UnicodeString { length: len, maximum_length: len, buffer: wide.as_ptr() };
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &name, attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        security_descriptor: null(), security_qos: null(),
    };
    let mut handle: Handle = null_mut();
    let mut status = IoStatus { status_or_pointer: 0, information: 0 };
    // Space and file identity are scoped to the pinned parent handle. Create-new and
    // no sharing retain exclusive ownership of the stage until publish or cleanup.
    status_result(unsafe {
        NtCreateFile(&mut handle, FILE_GENERIC_WRITE | DELETE | SYNCHRONIZE,
            &attributes, &mut status, null(), FILE_ATTRIBUTE_NORMAL, 0, FILE_CREATE,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE, null(), 0)
    })?;
    if handle.is_null() {
        return Err(io::Error::other("Windows did not return a staging file handle"));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn rename_stage(file: &File, leaf: &OsStr) -> io::Result<()> {
    const FILE_RENAME_INFORMATION: u32 = 10;
    let wide: Vec<u16> = leaf.encode_wide().collect();
    if wide.len() > 255 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Windows attachment name is too long"));
    }
    let name_offset = offset_of!(RenameInformation, file_name);
    let name_bytes = wide.len() * size_of::<u16>();
    let len = size_of::<RenameInformation>() + name_bytes;
    // Vec<usize> supplies pointer alignment for the variable-length NT structure.
    let mut storage = vec![0usize; len.div_ceil(size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<RenameInformation>();
    let mut status = IoStatus { status_or_pointer: 0, information: 0 };
    unsafe {
        (*info).file_name_length = name_bytes as u32;
        info.cast::<u8>().add(name_offset).cast::<u16>()
            .copy_from_nonoverlapping(wide.as_ptr(), wide.len());
        // Zero RootDirectory + a single leaf means the staging file's own parent,
        // not a cross-directory move which would reopen our pinned directory.
        status_result(NtSetInformationFile(file.as_raw_handle(), &mut status,
            info.cast(), len as u32, FILE_RENAME_INFORMATION))
    }
}

fn discard_stage(file: &File) -> io::Result<()> {
    const FILE_DISPOSITION_INFORMATION: u32 = 13;
    let mut status = IoStatus { status_or_pointer: 0, information: 0 };
    let delete = 1u8;
    status_result(unsafe {
        NtSetInformationFile(file.as_raw_handle(), &mut status,
            (&delete as *const u8).cast(), 1, FILE_DISPOSITION_INFORMATION)
    })
}

fn allocate_stage(parent: &File, leaf: &OsStr) -> io::Result<(File, String)> {
    loop {
        let name = format!(".paperroom-{}-{}.partial", std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed));
        if leaf.to_string_lossy().eq_ignore_ascii_case(&name) { continue }
        match create_stage(parent, &name) {
            Ok(file) => return Ok((file, name)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn write_stage(stage: &mut File, contents: &[u8]) -> io::Result<()> {
    stage.write_all(contents)?;
    stage.sync_all()
}

pub(crate) fn publish(parent: &File, leaf: &OsStr, contents: &[u8]) -> Result<()> {
    let leaf = crate::output::windows_output_name(leaf)?;
    let (mut stage, _) = allocate_stage(parent, leaf)?;
    let result = write_stage(&mut stage, contents).and_then(|()| rename_stage(&stage, leaf));
    if let Err(error) = result {
        if let Err(cleanup) = discard_stage(&stage) {
            // The published name is untouched. Do not silently leave a stage behind.
            return Err(format!("無法儲存附件：{error}；也無法清理暫存檔：{cleanup}").into());
        }
        return Err(error.into());
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    struct TestChild(Child);
    impl Drop for TestChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn forced_exit_before_publish_preserves_unpublished_stage_and_unrelated_files() -> Result<()> {
        const CHILD_ROOT: &str = "PAPERROOM_TEST_ATTACHMENT_CRASH_CHILD_ROOT";
        const STAGED: &[u8] = b"attachment payload before interrupted rename";
        const UNRELATED: &[u8] = b"other work, never delete";
        const LEAF: &str = "saved-attachment.txt";
        const READY: &str = "crash-test-stage-ready";
        if let Some(path) = std::env::var_os(CHILD_ROOT) {
            // This subprocess branch belongs exclusively to the harness invocation below.
            let root = std::path::PathBuf::from(path);
            assert_eq!(root.parent(), Some(std::env::temp_dir().as_path()));
            assert!(root.file_name().and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with("paperroom-attachment-crash-test-")));
            assert_eq!(fs::read(root.join("unrelated.partial"))?, UNRELATED);
            let destination = root.join(LEAF);
            let (_, pins) = super::super::pin_local_parents(&destination)?;
            let (mut stage, stage_name) = allocate_stage(pins.last().expect("pinned root"), OsStr::new(LEAF))?;
            write_stage(&mut stage, STAGED)?;
            fs::write(root.join(READY), stage_name)?;
            // Keep the actual non-sharing NTFS staging handle open until the parent kills us.
            loop { std::thread::park(); }
        }

        let root = std::env::temp_dir().join(format!("paperroom-attachment-crash-test-{}-{}",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        fs::create_dir(&root)?;
        fs::write(root.join("unrelated.partial"), UNRELATED)?;
        let destination = root.join(LEAF);
        let check = (|| -> Result<()> {
            let mut child = TestChild(Command::new(std::env::current_exe()?)
                .arg("--exact")
                .arg(concat!(module_path!(), "::forced_exit_before_publish_preserves_unpublished_stage_and_unrelated_files")
                    .split_once("::").expect("test module path").1)
                .env(CHILD_ROOT, &root)
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .spawn()?);
            let stage_prefix = format!(".paperroom-{}-", child.0.id());
            let deadline = Instant::now() + Duration::from_secs(15);
            let stage_name = loop {
                if let Ok(name) = fs::read_to_string(root.join(READY)) {
                    if name.strip_prefix(&stage_prefix).and_then(|s| s.strip_suffix(".partial"))
                        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())) {
                        break name;
                    }
                }
                if child.0.try_wait()?.is_some() || Instant::now() >= deadline {
                    return Err("Native attachment subprocess did not reach the sync barrier".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            assert!(!destination.try_exists()?);
            let stage_path = root.join(stage_name);
            assert!(stage_path.is_file());
            child.0.kill()?;
            assert!(!child.0.wait()?.success());
            assert!(!destination.try_exists()?);
            assert_eq!(fs::read(&stage_path)?, STAGED);
            let (_, pins) = super::super::pin_local_parents(&destination)?;
            publish(pins.last().expect("pinned root"), OsStr::new(LEAF), b"retried")?;
            assert_eq!(fs::read(&destination)?, b"retried");
            assert_eq!(fs::read(&stage_path)?, STAGED);
            assert_eq!(fs::read(root.join("unrelated.partial"))?, UNRELATED);
            let files = fs::read_dir(&root)?.collect::<io::Result<Vec<_>>>()?;
            assert_eq!(files.len(), 4, "destination, original stage, unrelated file, test signal");
            Ok(())
        })();
        let cleanup = fs::remove_dir_all(&root);
        check?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn attachment_on_pinned_parent_is_atomic_and_never_replaces_an_existing_file() -> Result<()> {
        let root = std::env::temp_dir().join(format!("paperroom-pinned-publish-{}", std::process::id()));
        fs::create_dir(&root)?;
        let destination = root.join("附件資料.txt");
        let (absolute, pins) = super::super::pin_local_parents(&destination)?;
        let parent = pins.last().expect("pin the destination parent");
        let leaf = absolute.file_name().expect("destination leaf");
        assert!(!destination.exists());
        publish(parent, leaf, b"first version")?;
        assert_eq!(fs::read(&destination)?, b"first version");
        let conflict = publish(parent, leaf, b"discard this stage");
        assert!(conflict.is_err());
        assert_eq!(fs::read(&destination)?, b"first version");
        assert_eq!(fs::read_dir(&root)?.collect::<io::Result<Vec<_>>>()?.len(), 1);
        drop(pins);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
