//! Explicit single-file editing. No Vault watcher or stored folder permission.
#[cfg(any(windows, test))]
use crate::store::{MAX_NOTE_BYTES, Result};

fn encode(original: &[u8], edited: &str) -> Result<Vec<u8>> {
    let bom = original.starts_with(&[0xef, 0xbb, 0xbf]);
    let body = std::str::from_utf8(if bom { &original[3..] } else { original })?;
    if edited.contains('\0') { return Err("Markdown 不能包含 NUL。".into()); }
    if body == edited { return Ok(original.to_vec()); }
    let rest = body.replace("\r\n", "");
    let styles = [body.contains("\r\n"), rest.contains('\r'), rest.contains('\n')];
    if styles.iter().filter(|&&s| s).count() > 1 {
        return Err("原檔混用換行格式，請將草稿複製到其他編輯器。".into());
    }
    let lf = edited.replace("\r\n", "\n").replace('\r', "\n");
    let line = if styles[0] { "\r\n" } else if styles[1] { "\r" } else { "\n" };
    let mut result = if bom { original[..3].to_vec() } else { Vec::new() };
    result.extend_from_slice(lf.replace('\n', line).as_bytes());
    if result.len() > MAX_NOTE_BYTES { return Err("編輯後超過 4 MiB。".into()); }
    Ok(result)
}

#[cfg(windows)]
pub use local::Draft;

#[cfg(windows)]
mod local {
    use super::*;
    use std::{fs::{File, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, os::windows::{ffi::OsStrExt,fs::OpenOptionsExt,io::AsRawHandle}, path::{Component,Path,PathBuf,Prefix},sync::atomic::{AtomicU64,Ordering}};
    use windows_sys::Win32::Storage::FileSystem::*;
    static SERIAL: AtomicU64 = AtomicU64::new(0);

    fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
        let mut info = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(info)
    }
    fn pin(path: &Path) -> Result<Vec<File>> {
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else { return Err("請選擇本機磁碟。".into()); };
        let Prefix::Disk(letter) = prefix.kind() else { return Err("僅接受本機磁碟的一般絕對路徑。".into()); };
        if components.next() != Some(Component::RootDir) { return Err("請選擇完整路徑。".into()); }
        let root = format!("{}:\\", letter as char);
        let wide: Vec<_> = std::ffi::OsStr::new(&root).encode_wide().chain(Some(0)).collect();
        if !matches!(unsafe {GetDriveTypeW(wide.as_ptr())}, 2 | 3 | 6) { return Err("原檔須位於可寫入的本機磁碟。".into()); }
        for c in components {
            let Component::Normal(name) = c else { return Err("不接受相對路徑。".into()); };
            let s = name.to_str().ok_or("路徑包含無效文字。")?;
            let base = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
            let port = base.strip_prefix("COM").or_else(||base.strip_prefix("LPT"));
            if s.ends_with([' ','.']) || s.contains([':', '<','>', '"','|','?','*']) || s.chars().any(char::is_control)
                || matches!(base.as_str(),"CON"|"AUX"|"NUL"|"PRN"|"CONIN$"|"CONOUT$") || port.is_some_and(|p| p.chars().count()==1 && "0123456789¹²³".contains(p)) {
                return Err("不支援裝置名稱、資料流或特殊路徑。".into());
            }
        }
        if !path.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("md") || s.eq_ignore_ascii_case("markdown")) { return Err("請選擇 Markdown 檔。".into()); }
        let mut dirs: Vec<_> = path.ancestors().skip(1).collect();
        dirs.reverse();
        let mut pins = Vec::new();
        for dir in dirs {
            let f = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES).share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT).open(dir)?;
            let attrs = info(&f)?.dwFileAttributes;
            if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 || attrs & FILE_ATTRIBUTE_DIRECTORY == 0 { return Err("資料夾含連結或重新解析點。".into()); }
            pins.push(f);
        }
        Ok(pins)
    }
    fn open_file(path:&Path, write:bool)->Result<(File,[u32;5])> {
        let f = OpenOptions::new().read(true).write(write).share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        let i = info(&f)?;
        if i.nNumberOfLinks != 1 || i.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) != 0 || unsafe {GetFileType(f.as_raw_handle())} != FILE_TYPE_DISK {
            return Err("只支援本機一般檔案，不接受硬連結或重新解析點。".into());
        }
        Ok((f,[i.dwVolumeSerialNumber,i.nFileIndexHigh,i.nFileIndexLow,i.ftLastWriteTime.dwHighDateTime,i.ftLastWriteTime.dwLowDateTime]))
    }
    fn read(file:&mut File)->Result<Vec<u8>> {
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.take(MAX_NOTE_BYTES as u64+1).read_to_end(&mut bytes)?;
        if bytes.len()>MAX_NOTE_BYTES {return Err("原檔超過 4 MiB。".into());}
        Ok(bytes)
    }
    fn recovery(path:&Path)->Result<(File,PathBuf)> {
        let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        for _ in 0..128 {
            let name=path.file_name().and_then(|n|n.to_str()).ok_or("無法識別原檔名稱。")?;
            let prefix=format!(".paperroom-recovery-{now}-{}-{}",std::process::id(),SERIAL.fetch_add(1,Ordering::Relaxed));
            let named=format!("{prefix}-{name}.bak");
            let target=if named.encode_utf16().count()<=255 {path.with_file_name(named)}
                else {path.with_file_name(format!("{prefix}.bak"))};
            match OpenOptions::new().read(true).write(true).create_new(true).share_mode(FILE_SHARE_READ).open(&target) {
                Ok(f)=>return Ok((f,target)),
                Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>continue,
                Err(e)=>return Err(e.into()),
            }
        }
        Err("無法建立專用復原檔。".into())
    }
    pub struct Draft {path:PathBuf,stamp:[u32;5],bytes:Vec<u8>}
    impl Draft {
        pub fn load(path:&Path)->Result<Self>{
            let _dirs=pin(path)?;
            let (mut f,stamp)=open_file(path,false)?;
            let bytes=read(&mut f)?;
            let text=std::str::from_utf8(&bytes)?;
            if text.contains('\0'){return Err("文字包含 NUL。".into());}
            Ok(Self{path:path.to_owned(),stamp,bytes})
        }
        pub(crate) fn raw_text(&self) -> &str {
            std::str::from_utf8(&self.bytes).expect("validated UTF-8")
        }
        pub(crate) fn encoded_text(&self, edited: &str) -> Result<String> {
            Ok(String::from_utf8(encode(&self.bytes, edited)?)?)
        }
        pub fn text(&self)->&str{
            std::str::from_utf8(self.bytes.strip_prefix(&[0xef,0xbb,0xbf]).unwrap_or(&self.bytes)).expect("validated UTF-8")
        }
        pub fn save(&self,edited:&str)->Result<Option<PathBuf>>{
            let new=encode(&self.bytes,edited)?;
            let _dirs=pin(&self.path)?;
            let (mut f,stamp)=open_file(&self.path,new!=self.bytes)?;
            if stamp!=self.stamp || read(&mut f)?!=self.bytes{return Err("原檔已在其他程式變更，已停止寫入。".into());}
            if new==self.bytes{return Ok(None);}
            let (mut backup,path)=recovery(&self.path)?;
            backup.write_all(&self.bytes)?;
            backup.sync_all()?;
            if read(&mut backup)?!=self.bytes{return Err("復原檔內容驗證失敗，原檔未變更。".into());}
            let write = (|| -> Result<()> {
                f.seek(SeekFrom::Start(0))?;
                f.write_all(&new)?;
                f.set_len(new.len() as u64)?;
                f.sync_all()?;
                if read(&mut f)?!=new {return Err("寫入後核對失敗。".into());}
                Ok(())
            })();
            if let Err(e)=write{return Err(format!("原檔可能只寫入部分內容。請從 {} 複製完整舊版復原。\n{e}",path.display()).into());}
            Ok(Some(path))
        }
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn keeps_bom_and_crlf(){assert_eq!(encode(b"\xef\xbb\xbfa\r\nb","新\n頁").unwrap(),"\u{feff}新\r\n頁".as_bytes());}
    #[test]fn preserves_mixed_unchanged_and_rejects_reformat(){let a="a\r\nb\nc";assert_eq!(encode(a.as_bytes(),a).unwrap(),a.as_bytes());assert!(encode(a.as_bytes(),"new").is_err());}
    #[cfg(windows)]
    #[test]fn external_change_is_retained_and_success_has_original_backup(){
        let dir=std::env::temp_dir().join(format!("paperroom-vault-edit-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&dir).unwrap();
        let path=dir.join("編輯.md");
        std::fs::write(&path,b"before").unwrap();
        let original=Draft::load(&path).unwrap();
        std::fs::write(&path,b"external").unwrap();
        assert!(original.save("mine").is_err());
        assert_eq!(std::fs::read(&path).unwrap(),b"external");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(),1);
        let current=Draft::load(&path).unwrap();
        let recovery=current.save("mine").unwrap().unwrap();
        assert_eq!(std::fs::read(recovery).unwrap(),b"external");
        assert_eq!(std::fs::read(&path).unwrap(),b"mine");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
