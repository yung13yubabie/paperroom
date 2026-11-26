//! Stage a verified standalone backup and the running executable, then publish
//! the whole directory in one no-replace rename. No source database is opened.
use super::{Notebook, Result, SCHEMA_VERSION, attachments};
use rusqlite::{Connection, OpenFlags};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn validate_database(path:&Path)->Result<()> {
    let flags=OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    #[cfg(windows)]
    let db=Connection::open_with_flags_and_vfs(path,flags,"win32-longpath")?;
    #[cfg(not(windows))]
    let db=Connection::open_with_flags(path,flags)?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY;")?;
    let version:i32=db.pragma_query_value(None,"user_version",|r|r.get(0))?;
    if version>SCHEMA_VERSION {return Err("這份紙間備份由較新版本建立。".into());}
    if version!=SCHEMA_VERSION && version!=43 {return Err("此版本尚不能核對這份紙間備份的舊結構。".into());}
    let reference=Notebook::open(Path::new(":memory:"),false)?;
    if version==43 {reference.connection.execute_batch("DROP TABLE obsidian_base_descending_groups")?;}
    // Compare all definitions, including triggers, views and FTS shadow tables,
    // before querying any user table. Only the trusted application builds the reference.
    let sql="SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name";
    let mut expected=reference.connection.prepare(sql)?;
    let mut expected=expected.query([])?;
    let mut actual=db.prepare(sql)?;
    let mut actual=actual.query([])?;
    while let Some(row)=expected.next()? {
        let other=actual.next()?.ok_or("缺少紙間備份的資料表或索引。")?;
        for i in 0..4 {
            if row.get_ref(i)?!=other.get_ref(i)? {return Err("紙間備份的結構與已知版本不符。".into());}
        }
    }
    if actual.next()?.is_some() {return Err("紙間備份含有未識別的結構。".into());}
    let integrity:String=db.query_row("PRAGMA integrity_check(1)",[],|r|r.get(0))?;
    if integrity!="ok" || db.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err("紙間備份的完整性或資料關聯檢查失敗。".into());
    }
    Ok(())
}

fn sidecars(source:&Path)->Result<()> {
    for suffix in ["-wal","-shm","-journal"] {
        let mut name=source.as_os_str().to_owned(); name.push(suffix);
        match fs::symlink_metadata(Path::new(&name)) {
            Err(e) if e.kind()==io::ErrorKind::NotFound=>(),
            Err(e)=>return Err(e.into()),
            Ok(_)=>return Err("請使用「備份工作區」產生的獨立紙間備份；來源旁仍有 SQLite 日誌檔。".into()),
        }
    }
    Ok(())
}

fn copy_new(input:&mut File,destination:&Path,created:&mut bool)->Result<()> {
    let mut output=OpenOptions::new().create_new(true).write(true).open(destination)?;
    *created=true;
    let size=input.metadata()?.len();
    if io::copy(input,&mut output)?!=size || size==0 {return Err("檔案複製不完整。".into());}
    output.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn origin_stream_path(path: &Path) -> PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    PathBuf::from(stream)
}

#[cfg(windows)]
fn origin_mark(path: &Path) -> Result<Option<Vec<u8>>> {
    let input = match File::open(origin_stream_path(path)) {
        Ok(input) => input,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        // A filesystem without named streams cannot contain the mark.
        Err(error) if matches!(error.raw_os_error(), Some(50 | 123)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    const LIMIT: usize = 64 * 1024;
    let mut mark = Vec::new();
    input.take((LIMIT + 1) as u64).read_to_end(&mut mark)?;
    if mark.len() > LIMIT {
        return Err("Windows 來源標記大小超出可核對範圍。".into());
    }
    Ok(Some(mark))
}

#[cfg(windows)]
fn copy_application(source: &Path, destination: &Path, created: &mut bool) -> Result<()> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Storage::FileSystem::CopyFileExW;
    const COPY_FILE_FAIL_IF_EXISTS: u32 = 0x00000001;

    // Win32's copy also copies named streams such as the Windows download mark.
    let origin = origin_mark(source)?;
    let from = path_wide(source);
    let to = path_wide(destination);
    if unsafe {
        CopyFileExW(
            from.as_ptr(), to.as_ptr(), None, null_mut(), null_mut(), COPY_FILE_FAIL_IF_EXISTS,
        )
    } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    *created = true;
    if origin_mark(destination)? != origin {
        return Err("目的地無法保留目前紙間的 Windows 來源標記。".into());
    }
    Ok(())
}

#[cfg(windows)]
fn path_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn publish_folder(stage:&Path,destination:&Path)->Result<()> {
    #[cfg(windows)]
    {
        let from = path_wide(stage);
        let to = path_wide(destination);
        if unsafe {windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(),to.as_ptr(),0)}==0 {
            return Err(io::Error::last_os_error().into());
        }
    }
    #[cfg(target_os="linux")]
    {
        use std::{ffi::CString,os::unix::ffi::OsStrExt};
        unsafe extern "C" { fn renameat2(fd1:i32,from:*const std::ffi::c_char,fd2:i32,to:*const std::ffi::c_char,flags:u32)->i32; }
        let from=CString::new(stage.as_os_str().as_bytes())?;let to=CString::new(destination.as_os_str().as_bytes())?;
        if unsafe {renameat2(-100,from.as_ptr(),-100,to.as_ptr(),1)}!=0 {return Err(io::Error::last_os_error().into());}
    }
    #[cfg(not(any(windows,target_os="linux")))]
    return Err("目前平台無法安全發布新的工作區資料夾。".into());
    Ok(())
}

pub(crate) fn restore_workspace(source:&Path,parent:&Path,name:&str,live_db:&Path,exe:&Path)->Result<PathBuf> {
    crate::output::windows_output_name(OsStr::new(name))?;
    if name.encode_utf16().count()>180 || name.chars().any(char::is_control) {return Err("請使用簡短的一般資料夾名稱。".into());}
    if !parent.is_absolute() || !source.is_absolute() {return Err("請選用完整的本機路徑。".into());}
    #[cfg(windows)]
    let (_, _parent_guards)=attachments::pin_local_parents(&parent.join("Paperroom.exe"))?;
    let parent=fs::canonicalize(parent)?;
    let destination=parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_)=>return Err("目的地已存在，請輸入新的工作區資料夾名稱。".into()),
        Err(e) if e.kind()==io::ErrorKind::NotFound=>(),
        Err(e)=>return Err(e.into()),
    }
    #[cfg(windows)]
    let (_, _source_path_guards) = attachments::pin_local_parents(source)
        .map_err(|error| format!("無法檢查備份來源資料夾：{error}"))?;
    if fs::canonicalize(source)?==fs::canonicalize(live_db)? {return Err("請選擇獨立的紙間備份，不能使用目前開啟的工作區。".into());}
    let (mut db_file,_source_guards)=attachments::open_local_file(source)
        .map_err(|error| format!("無法讀取備份來源：{error}"))?;
    sidecars(source)?;
    let mut header=[0u8;20];
    if db_file.read_exact(&mut header).is_err() || &header[..16]!=b"SQLite format 3\0" || header[18..]!=[1,1] {
        return Err("請選擇「備份工作區」產生的獨立紙間備份。".into());
    }
    db_file.rewind()?;
    let (app_file, _exe_guards) = attachments::open_local_file(exe)
        .map_err(|error| format!("無法檢查目前紙間程式：{error}"))?;
    #[cfg(not(windows))]
    let mut app_file = app_file;
    #[cfg(windows)]
    let _app_file = app_file;
    let stage=loop {
        let stage=parent.join(format!(".paperroom-restore-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        if stage==destination {continue;}
        match fs::create_dir(&stage) {
            Ok(())=>break stage,
            Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>continue,
            Err(e)=>return Err(e.into()),
        }
    };
    let data=stage.join("paperroom-data");
    let staged_db=data.join("notes.sqlite3");
    let staged_exe=stage.join("Paperroom.exe");
    let (mut made_data,mut made_db,mut made_exe)=(false,false,false);
    let result=(||->Result<()>{
        #[cfg(windows)]
        let (_, _stage_guards)=attachments::pin_local_parents(&staged_exe)
            .map_err(|error| format!("無法檢查準備中的工作區：{error}"))?;
        fs::create_dir(&data).map_err(|error| format!("無法建立工作區資料目錄：{error}"))?;
        made_data=true;
        #[cfg(windows)]
        let (_, _data_guards)=attachments::pin_local_parents(&staged_db)
            .map_err(|error| format!("無法檢查工作區資料目錄：{error}"))?;
        copy_new(&mut db_file,&staged_db,&mut made_db)
            .map_err(|error| format!("無法複製紙間備份：{error}"))?;
        sidecars(source)?;
        validate_database(&staged_db).map_err(|e|format!("無法驗證紙間備份：{e}"))?;
        #[cfg(windows)]
        copy_application(exe, &staged_exe, &mut made_exe)
            .map_err(|error| format!("無法複製目前的紙間程式：{error}"))?;
        #[cfg(not(windows))]
        copy_new(&mut app_file, &staged_exe, &mut made_exe)?;
        if fs::read_dir(&stage)?.count()!=2 || fs::read_dir(&data)?.count()!=1 {
            return Err("建立期間資料夾內容已變更，已停止另開工作區。".into());
        }
        #[cfg(windows)]
        { drop(_data_guards);drop(_stage_guards); }
        publish_folder(&stage, &destination)
            .map_err(|error| format!("無法完成新的工作區資料夾：{error}"))?;
        Ok(())
    })();
    if let Err(error)=result {
        // Never recursively delete a directory or remove the user's destination.
        if made_exe {let _=fs::remove_file(staged_exe);}
        if made_db {let _=fs::remove_file(staged_db);}
        if made_data {let _=fs::remove_dir(data);}
        let _=fs::remove_dir(&stage);
        return Err(error);
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_backup_opens_independently_and_refuses_bad_or_occupied_destinations()->Result<()> {
        let dir=super::super::tests::data_dir().canonicalize()?;
        let live=dir.join("live.db");
        let backup=dir.join("saved.sqlite3");
        let exe=dir.join("original.exe");
        fs::write(&exe,b"synthetic binary identity")?;
        #[cfg(windows)]
        let expected_origin = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        #[cfg(windows)]
        fs::write(origin_stream_path(&exe), expected_origin)?;
        let mut db=Notebook::open(&live,false)?;
        let note=db.add(None,"分鏡","備份的正文")?;
        let text=dir.join("附件.txt");fs::write(&text,b"exact bytes")?;
        let asset=db.import_attachment(note,&text)?;
        db.backup(&backup).map_err(|error| format!("create test backup: {error}"))?;
        let original=fs::read(&backup)?;
        db.save(note,1,"分鏡","備份之後")?;
        let restored=restore_workspace(&backup,&dir,"副本",&live,&exe)?;
        let restored_exe = restored.join("Paperroom.exe");
        assert_eq!(fs::read(&restored_exe)?, fs::read(&exe)?);
        #[cfg(windows)]
        assert_eq!(origin_mark(&restored_exe)?.as_deref(), Some(expected_origin.as_slice()));
        let copy=restored.join("paperroom-data/notes.sqlite3");
        assert_eq!(fs::read(&copy)?,original);
        let work=Notebook::open(&copy,false)?;
        assert_eq!(work.note(note)?.unwrap().body,"備份的正文");
        work.save(note,1,"改副本","不同內容")?;
        let output=dir.join("附件副本");work.save_attachment_copy(asset,&output)?;
        assert_eq!(fs::read(output)?,b"exact bytes");
        assert_eq!(db.note(note)?.unwrap().body,"備份之後");
        assert_eq!(fs::read(&backup)?,original);
        assert!(restore_workspace(&backup,&dir,"副本",&live,&exe).is_err());
        fs::create_dir(dir.join("空資料夾"))?;
        assert!(restore_workspace(&backup,&dir,"空資料夾",&live,&exe).is_err());
        for name in ["../跳出","CON",".","name."] {assert!(restore_workspace(&backup,&dir,name,&live,&exe).is_err());}
        assert!(restore_workspace(&live,&dir,"活動來源",&live,&exe).is_err());
        let foreign=dir.join("not-paperroom.db");
        let invalid=Connection::open(&foreign)?;
        invalid.execute_batch("CREATE TABLE foo(x); PRAGMA user_version=44;")?;
        drop(invalid);
        assert!(restore_workspace(&foreign,&dir,"非工作區",&live,&exe).is_err());
        let changed=dir.join("modified.db");
        fs::copy(&backup,&changed)?;
        let extra=Connection::open(&changed)?;
        extra.execute_batch("CREATE TABLE extra_synthetic(id INTEGER);")?;
        drop(extra);
        assert!(restore_workspace(&changed,&dir,"未知結構",&live,&exe).is_err());
        fs::write(dir.join("saved.sqlite3-wal"),b"synthetic active journal")?;
        assert!(restore_workspace(&backup,&dir,"日誌中",&live,&exe).is_err());
        fs::remove_file(dir.join("saved.sqlite3-wal"))?;
        let newer=dir.join("future.db");
        fs::copy(&backup,&newer)?;
        let future=Connection::open(&newer)?;future.pragma_update(None,"user_version",99)?;drop(future);
        assert!(restore_workspace(&newer,&dir,"新版",&live,&exe).is_err());
        let previous=dir.join("previous.db");
        fs::copy(&backup,&previous)?;
        let old=Connection::open(&previous)?;
        old.execute_batch("DROP TABLE obsidian_base_descending_groups; PRAGMA user_version=43;")?;
        drop(old);
        // CI can also supply a backup written by the archived schema-43 build.
        if let Some(path)=std::env::var_os("PAPERROOM_QA_SCHEMA43_BACKUP") {
            let old=PathBuf::from(path);
            let original=fs::read(&old)?;
            let target=restore_workspace(&old,&dir,"封存版本",&live,&exe)?;
            let copy=target.join("paperroom-data/notes.sqlite3");
            assert_eq!(fs::read(&copy)?,original);
            let db=Notebook::open(&copy,false)?;
            assert_eq!(db.connection.query_row("SELECT count(*) FROM notes WHERE body='SYNTHETIC-V43-REOPEN'",[],|r|r.get::<_,i64>(0))?,1);
            assert_eq!(db.connection.pragma_query_value(None,"user_version",|r|r.get::<_,i32>(0))?,SCHEMA_VERSION);
            assert_eq!(fs::read(&old)?,original);
        }
        let prior=restore_workspace(&previous,&dir,"以前",&live,&exe)?;
        let upgraded=Notebook::open(&prior.join("paperroom-data/notes.sqlite3"),false)?;
        assert_eq!(upgraded.note(note)?.unwrap().body,"備份的正文");drop(upgraded);
        for target in ["非工作區","未知結構","日誌中","新版","活動來源"] {assert!(!dir.join(target).exists());}
        assert!(!dir.read_dir()?.any(|p|p.unwrap().file_name().to_string_lossy().starts_with(".paperroom-restore-")));
        drop(db);
        drop(work);
        fs::remove_dir_all(dir).map_err(|error| format!("remove closed test workspaces: {error}"))?;
        Ok(())
    }
}
