//! Explicit, foreground Vault synchronization. Paths are chosen for this operation.
use super::{Notebook, Result, insert};
use rusqlite::{Connection, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) const SCHEMA: &str = "CREATE TABLE vault_sync_base(
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    body TEXT NOT NULL
);";

// Older incoming keep-both operations recorded disk text as the baseline even
// though the mapped note kept a divergent body. Only seed provably equal rows.
pub(crate) const PUSH_SCHEMA: &str = "CREATE TABLE vault_push_verified(
    note_id INTEGER PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE);
    INSERT INTO vault_push_verified(note_id) SELECT b.note_id FROM vault_sync_base b
    JOIN notes n ON n.id=b.note_id WHERE n.body=b.body;";

#[path = "vault_admission.rs"]
mod admission;

#[path = "vault_relocation.rs"]
mod relocation;

#[cfg(windows)]
#[path = "vault_push.rs"]
mod push;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Change { Equal, Baseline, Incoming, Local, Conflict, Unknown, Missing }

impl Change {
    fn safe(self) -> bool { matches!(self, Self::Incoming | Self::Baseline) }
    fn conflict(self) -> bool { matches!(self, Self::Conflict | Self::Unknown) }
    fn label(self) -> &'static str {
        match self {
            Self::Equal => "一致", Self::Baseline => "內容相同，可記錄共同版本",
            Self::Incoming => "僅磁碟變更，可更新紙間", Self::Local => "僅紙間變更，保留",
            Self::Conflict => "兩端變更，可另存磁碟版本", Self::Unknown => "無共同版本，可另存磁碟版本",
            Self::Missing => "磁碟未讀到，保留紙間",
        }
    }
}

#[derive(Debug,PartialEq,Eq)]
struct Entry { path:String, id:i64, version:i64, parent:Option<i64>, title:String, change:Change, verified:bool }

#[derive(Debug,PartialEq,Eq)]
pub(crate) struct Preview {
    root:i64, entries:Vec<Entry>, disk:BTreeMap<String,String>, skipped:usize, unmapped:usize,
}

impl Preview {
    pub(crate) fn report(&self) -> String {
        let mut text=format!("已配對路徑 {} 筆；磁碟另有 {} 篇未配對，掃描略過 {} 個項目。\r\n僅將已配對的 Markdown 讀入紙間。資料夾與磁碟檔案維持原狀。\r\n",self.entries.len(),self.unmapped,self.skipped);
        for (i,e) in self.entries.iter().enumerate() {
            text.push_str(&format!("\r\n{}　{}　{}",i+1,e.change.label(),e.path));
        }
        text
    }
    pub(crate) fn details(&self, notebook: &Notebook, row: usize) -> Result<String> {
        let entry = row.checked_sub(1).and_then(|index| self.entries.get(index))
            .ok_or("列號超出預覽範圍。")?;
        let note = notebook.note(entry.id)?
            .filter(|note| note.version == entry.version && note.row.trash_root.is_none())
            .ok_or("紙間內容已變更，請重新預覽。")?;
        let Some(disk) = self.disk.get(&entry.path) else {
            return Ok(format!("{}\r\n磁碟未讀到此筆記。", entry.path));
        };
        let first = note.body.chars().zip(disk.chars())
            .position(|(left, right)| left != right)
            .unwrap_or_else(|| note.body.chars().count().min(disk.chars().count()));
        let start = first.saturating_sub(160);
        let excerpt = |body: &str| body.chars().skip(start).take(800).collect::<String>();
        let detail = if note.body == *disk {
            "紙間和磁碟原文一致。".to_owned()
        } else {
            format!("第 {} 個字元附近\r\n\r\n【紙間】\r\n{}\r\n\r\n【磁碟】\r\n{}",
                first + 1, excerpt(&note.body), excerpt(disk))
        };
        Ok(format!("{}　{}\r\n\r\n{detail}", entry.path, entry.change.label()))
    }

    pub(crate) fn actionable(&self) -> bool { self.entries.iter().any(|e|e.change.safe()||e.change.conflict()) }
    fn selected(&self, input:&str)->Result<Vec<&Entry>> {
        let mut ids=BTreeSet::new();
        for token in input.split(',').map(str::trim) {
            if token=="安全" {
                ids.extend(self.entries.iter().enumerate().filter(|(_,e)|e.change.safe()).map(|(i,_)|i));
            } else {
                let i=token.parse::<usize>().ok().and_then(|n|n.checked_sub(1)).ok_or("請輸入「安全」或預覽中的列號，以半形逗號分隔。")?;
                let e=self.entries.get(i).ok_or("列號超出預覽範圍。")?;
                if !e.change.safe() && !e.change.conflict() {return Err("選到的列沒有可套用的變更。".into());}
                ids.insert(i);
            }
        }
        if ids.is_empty(){return Err("這次沒有選擇可套用的項目。".into());}
        Ok(ids.into_iter().map(|i|&self.entries[i]).collect())
    }
    pub(crate) fn counts(&self,input:&str)->Result<(usize,usize)> {
        let rows=self.selected(input)?;
        Ok((rows.len(),rows.iter().filter(|e|e.change.conflict()).count()))
    }
}

fn scan(root:&Path)->Result<(BTreeMap<String,String>,usize)> {
    let vault=crate::vault::scan_for_sync(root)?;
    let mut pages=BTreeMap::new();
    for page in vault.pages {
        if pages.insert(page.parts.join("/"),page.body).is_some() {return Err("磁碟路徑重複，已停止。".into());}
    }
    Ok((pages,vault.skipped))
}
fn classify(conn:&Connection,root:i64,disk:&BTreeMap<String,String>)->Result<Vec<Entry>> {
    // Compare bodies inside SQLite, without materializing old or very large notebook bodies.
    let mut query=conn.prepare("SELECT p.relative_path,n.id,n.version,n.parent,n.title, EXISTS(SELECT 1 FROM vault_push_verified v WHERE v.note_id=n.id)
        FROM vault_paths p JOIN notes n ON n.id=p.note_id JOIN notes r ON r.id=p.import_root
        WHERE p.import_root=?1 AND r.trash_root IS NULL AND n.trash_root IS NULL ORDER BY p.relative_path")?;
    let mut compare=conn.prepare("SELECT n.body=?2,b.note_id IS NOT NULL,
        COALESCE(b.body=n.body,0),COALESCE(b.body=?2,0)
        FROM notes n LEFT JOIN vault_sync_base b ON b.note_id=n.id WHERE n.id=?1")?;
    let mut rows=query.query([root])?;
    let mut entries=Vec::new();
    while let Some(row)=rows.next()? {
        let path:String=row.get(0)?;
        let id=row.get(1)?;
        let change=if let Some(body)=disk.get(&path) {
            let (equal,known,local_base,disk_base):(bool,bool,bool,bool)=compare.query_row(params![id,body],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            match (equal,known,local_base,disk_base) {
                (true,_,_,true) if row.get::<_,bool>(5)?=>Change::Equal, (true,_,_,_)=>Change::Baseline,
                (_,false,_,_)=>Change::Unknown, (_,_,true,_)=>Change::Incoming,
                (_,_,_,true)=>Change::Local, _=>Change::Conflict,
            }
        } else {Change::Missing};
        entries.push(Entry{path,id,version:row.get(2)?,parent:row.get(3)?,title:row.get(4)?,verified:row.get(5)?,change});
        if entries.len()>crate::archive::MAX_ENTRIES {return Err("配對路徑超過 15,000 筆。".into());}
    }
    if entries.is_empty(){return Err("這個 Vault 沒有可配對的有效筆記。".into());}
    Ok(entries)
}

impl Notebook {
    pub(crate) fn preview_vault_pull(&self, root:i64, dir:&Path)->Result<Preview>{
        let (disk,skipped)=scan(dir)?;
        let tx=self.connection.unchecked_transaction()?;
        let entries=classify(&tx,root,&disk)?;
        tx.commit()?;
        let unmapped=disk.len()-entries.iter().filter(|e|disk.contains_key(&e.path)).count();
        Ok(Preview{root,entries,disk,skipped,unmapped})
    }
    pub(crate) fn apply_vault_pull(&mut self, preview:&Preview, dir:&Path, selection:&str, backup:&Path)->Result<usize>{
        let selected=preview.selected(selection)?;
        let (disk,skipped)=scan(dir)?;
        if disk!=preview.disk || skipped!=preview.skipped{return Err("磁碟內容已變更，請重新預覽。".into());}
        drop(disk);
        self.backup(backup)?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if classify(&tx,preview.root,&preview.disk)?!=preview.entries{return Err("紙間內容已變更，請重新預覽。備份已保留。".into());}
        for entry in &selected {
            let body=preview.disk.get(&entry.path).ok_or("預覽缺少磁碟內容。")?;
            if entry.change==Change::Incoming {
                tx.execute("UPDATE notes SET body=?2,version=version+1 WHERE id=?1",params![entry.id,body])?;
            } else if entry.change.conflict(){
                let title=format!("磁碟版本・{}",entry.title.chars().take(super::MAX_TITLE_CHARS-5).collect::<String>());
                insert(&tx,entry.parent,&title,body)?;
            }
            if entry.change.conflict() {
                tx.execute("DELETE FROM vault_push_verified WHERE note_id=?1",[entry.id])?;
            } else {
                tx.execute("INSERT OR IGNORE INTO vault_push_verified(note_id) VALUES(?1)",[entry.id])?;
            }
            tx.execute("INSERT INTO vault_sync_base(note_id,body) VALUES(?1,?2)
                ON CONFLICT(note_id) DO UPDATE SET body=excluded.body",params![entry.id,body])?;
        }
        tx.commit()?;
        Ok(selected.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_rows_preserve_missing_files_and_legacy_unknowns()->Result<()> {
        let dir=super::super::tests::data_dir();
        let vault=dir.join("Vault");
        std::fs::create_dir(&vault)?;
        for name in ["A.md","B.md","C.md","D.md"] {
            std::fs::write(vault.join(name),"old")?;
        }
        let mut db=Notebook::open(&dir.join("notes.sqlite"),false)?;
        let root=db.import_vault("V",crate::vault::read(&vault)?.pages)?;
        db.connection.execute("DELETE FROM vault_sync_base WHERE note_id IN
            (SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path IN ('B.md','C.md'))",[root])?;
        std::fs::write(vault.join("A.md"),"incoming")?;
        std::fs::write(vault.join("C.md"),"unknown")?;
        std::fs::remove_file(vault.join("D.md"))?;
        std::fs::write(vault.join("E.md"),"unmapped")?;
        let preview=db.preview_vault_pull(root,&vault)?;
        assert_eq!(preview.unmapped,1);
        assert_eq!(preview.entries.iter().map(|e|e.change).collect::<Vec<_>>(),
            vec![Change::Incoming,Change::Baseline,Change::Unknown,Change::Missing]);
        assert_eq!(preview.counts("安全")?,(2,0));
        assert!(preview.counts("4").is_err());
        let backup=dir.join("backup.sqlite");
        assert_eq!(db.apply_vault_pull(&preview,&vault,"1",&backup)?,1);
        let after=db.preview_vault_pull(root,&vault)?;
        assert_eq!(after.entries[0].change,Change::Equal);
        assert_eq!(after.entries[1].change,Change::Baseline);
        assert_eq!(db.note(after.entries[3].id)?.unwrap().body,"old");
        assert_eq!(db.note(after.entries[0].id)?.unwrap().body,"incoming");
        let saved=Notebook::open(&backup,false)?;
        assert_eq!(saved.note(after.entries[0].id)?.unwrap().body,"old");
        for name in ["A.md","B.md","C.md","E.md"] { std::fs::remove_file(vault.join(name))?; }
        let empty=db.preview_vault_pull(root,&vault)?;
        assert!(!empty.actionable());
        assert!(empty.entries.iter().all(|entry|entry.change==Change::Missing));
        drop(saved);drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    fn stale_batch_refuses_then_keep_both_preserves_original_and_backup()->Result<()>{
        let dir=super::super::tests::data_dir();
        let vault=dir.join("Vault");
        std::fs::create_dir(&vault)?;
        let file=vault.join("A.md");
        std::fs::write(&file,"old")?;
        let mut db=Notebook::open(&dir.join("notes.sqlite"),false)?;
        let root=db.import_vault("V",crate::vault::read(&vault)?.pages)?;
        let id:i64=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1",[root],|r|r.get(0))?;
        std::fs::write(&file,"disk")?;
        let first=db.preview_vault_pull(root,&vault)?;
        assert_eq!(first.counts("安全")?,(1,0));
        let note=db.note(id)?.unwrap();
        db.save(id,note.version,"A","local")?;
        assert!(db.apply_vault_pull(&first,&vault,"安全",&dir.join("stale.sqlite")).is_err());
        let conflict=db.preview_vault_pull(root,&vault)?;
        assert_eq!(conflict.counts("1")?,(1,1));
        let details = conflict.details(&db, 1)?;
        assert!(details.contains("local") && details.contains("disk"));
        assert!(conflict.details(&db, 2).is_err());
        std::fs::write(&file,"newer")?;
        assert!(db.apply_vault_pull(&conflict,&vault,"1",&dir.join("disk-stale.sqlite")).is_err());
        let current=db.preview_vault_pull(root,&vault)?;
        let backup=dir.join("backup.sqlite");
        db.apply_vault_pull(&current,&vault,"1",&backup)?;
        assert_eq!(db.note(id)?.unwrap().body,"local");
        assert_eq!(std::fs::read_to_string(file)?,"newer");
        let count:i64=db.connection.query_row("SELECT COUNT(*) FROM notes WHERE body='newer'",[],|r|r.get(0))?;
        assert_eq!(count,1);
        assert!(!db.preview_vault_pull(root,&vault)?.actionable());
        let fork=db.preview_vault_pull(root,&vault)?;
        assert_eq!(fork.entries[0].change,Change::Local);
        assert!(!fork.entries[0].verified,"a preserved fork must never qualify for overwrite");
        let saved=Notebook::open(&backup,false)?;
        assert_eq!(saved.note(id)?.unwrap().body,"local");
        drop(saved);drop(db);
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
