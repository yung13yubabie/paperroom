//! Outgoing rows are independently durable. Never publish a guessed/new path.
use super::*;
use crate::vault_edit::Draft;
use std::path::PathBuf;

pub(crate) struct Push {
    preview: Preview,
    folder: PathBuf,
    drafts: BTreeMap<usize, Draft>,
    blocked: BTreeMap<usize,String>,
}

impl Push {
    pub(crate) fn report(&self) -> String {
        let mut lines=vec![format!("已配對 {} 列，未配對 {} 篇。只寫回勾選的既有檔案；每篇保留同資料夾的 .bak。", self.preview.entries.len(), self.preview.unmapped)];
        for (i,entry) in self.preview.entries.iter().enumerate() {
            let reason=if self.drafts.contains_key(&i) { "可寫回" }
                else if let Some(error)=self.blocked.get(&i) {error}
                else if entry.change==Change::Local {"未證實共同版本，保留兩端"}
                else {match entry.change {
                    Change::Conflict|Change::Unknown=>"兩端保留；可在批次讀入另存磁碟版",
                    _=>entry.change.label(),
                }};
            lines.push(format!("{}　{}　{}",i+1,reason,entry.path));
        }
        lines.join("\r\n")
    }
    pub(crate) fn actionable(&self)->bool { !self.drafts.is_empty() }
    pub(crate) fn details(&self,db:&Notebook,row:usize)->Result<String> { self.preview.details(db,row) }
    pub(crate) fn select(&self,input:&str)->Result<BTreeSet<usize>> {
        let mut result=BTreeSet::new();
        for token in input.split(',').map(str::trim) {
            if token=="安全" { result.extend(self.drafts.keys().copied()); }
            else {
                let i=token.parse::<usize>().ok().and_then(|n|n.checked_sub(1)).filter(|i|self.drafts.contains_key(i)).ok_or("該列不能寫回。")?;
                result.insert(i);
            }
        }
        if result.is_empty(){return Err("沒有可寫回的項目。".into());}
        Ok(result)
    }
}

impl Notebook {
    pub(crate) fn preview_vault_push(&self,root:i64,dir:&Path)->Result<Push>{
        let preview=self.preview_vault_pull(root,dir)?;
        let mut drafts=BTreeMap::new();
        let mut blocked=BTreeMap::new();
        for (i,entry) in preview.entries.iter().enumerate() {
            if entry.change!=Change::Local || !entry.verified {continue;}
            // Exact key came from traversal of this selected directory. Reject
            // ambiguous case-folded aliases, even on case-sensitive NTFS.
            let disk=&preview.disk[&entry.path];
            let prepare=(||->Result<Draft>{
                let path=dir.join(&entry.path);
                // Reserve room for timestamp, process ID and serial. Each
                // batch backup must remain identifiable after a sudden exit.
                if path.file_name().is_none_or(|n|n.to_string_lossy().encode_utf16().count()>176) {
                    return Err("檔名過長，無法建立含完整原檔名的批次復原檔。".into());
                }
                let draft=Draft::load(&path)?;
                if draft.raw_text()!=disk {return Err("磁碟剛有變更，請重做預覽。".into());}
                let note=self.note(entry.id)?.filter(|n|n.version==entry.version).ok_or("紙間剛有變更。")?;
                draft.encoded_text(editor_body(disk,&note.body))?;
                Ok(draft)
            })();
            match prepare {
                Ok(draft)=>{drafts.insert(i,draft);}
                Err(error)=>{blocked.insert(i,format!("略過：{error}"));}
            }
        }
        let mut aliases=BTreeMap::new();
        for path in preview.disk.keys(){*aliases.entry(path.to_lowercase()).or_insert(0usize)+=1;}
        drafts.retain(|i,_|{
            if aliases[&preview.entries[*i].path.to_lowercase()]>1 {
                blocked.insert(*i,"路徑大小寫不唯一，略過".into());
                false
            }else{true}
        });
        Ok(Push{preview,folder:dir.to_owned(),drafts,blocked})
    }

    pub(crate) fn apply_vault_push(&mut self, plan:&Push, selection:&str, backup:&Path)->Result<String>{
        let chosen=plan.select(selection)?;
        let (disk,_)=scan(&plan.folder)?;
        if disk!=plan.preview.disk {return Err("預覽後磁碟內容改變。請重新預覽。".into());}
        drop(disk);
        self.backup(backup)?;
        let mut completed=Vec::new();
        let mut remaining=chosen.into_iter();
        let mut active=None;
        let result=(||->Result<()>{
            // Serialize all selected database versions with the filesystem
            // writes. A crash rolls back the baseline; disk and recovery copies
            // survive and the next preview compares actual bytes again.
            let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if classify(&tx,plan.preview.root,&plan.preview.disk)?!=plan.preview.entries {
                return Err("預覽後紙間內容改變。".into());
            }
            for i in remaining.by_ref() {
                let entry=&plan.preview.entries[i];
                active=Some(entry.path.as_str());
                let draft=&plan.drafts[&i];
                let body:String=tx.query_row("SELECT body FROM notes WHERE id=?1",[entry.id],|r|r.get(0))?;
                let edited=editor_body(draft.raw_text(),&body);
                let normalized=draft.encoded_text(edited)?;
                let saved=draft.save(edited)?;
                completed.push(match saved {
                    Some(path)=>format!("{} → {}",entry.path,path.display()),
                    None=>format!("{}（內容已符合原檔格式）",entry.path),
                });
                active=None;
                if body!=normalized {
                    tx.execute("UPDATE notes SET body=?1,version=version+1 WHERE id=?2",params![normalized,entry.id])?;
                }
                tx.execute("UPDATE vault_sync_base SET body=?1 WHERE note_id=?2",params![normalized,entry.id])?;
            }
            tx.commit()?;
            Ok(())
        })();
        if let Err(error)=result {
            let untouched=remaining.map(|i|plan.preview.entries[i].path.as_str()).collect::<Vec<_>>();
            let finished=if completed.is_empty() {"無".to_owned()} else {completed.join("\r\n")};
            let pending=if untouched.is_empty() {"無".to_owned()} else {untouched.join("\r\n")};
            return Err(format!("批次停止，資料庫基準未完成更新。請重新預覽，再決定是否復原。\r\n{error}\r\n已確認完成的磁碟項目與復原檔：\r\n{finished}\r\n處理中停止，請核對：{}\r\n尚未處理：\r\n{pending}\r\n工作區備份：{}",active.unwrap_or("無"),backup.display()).into());
        }
        Ok(format!("已完成 {} 列。\r\n{}\r\n工作區備份：{}",completed.len(),completed.join("\r\n"),backup.display()))
    }
}

fn editor_body<'a>(original:&str, edited:&'a str)->&'a str {
    if original.starts_with('\u{feff}') {edited.strip_prefix('\u{feff}').unwrap_or(edited)} else {edited}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consecutive_pushes_keep_each_backup_and_conflict_copy_scoped_to_its_folder()->Result<()> {
        let dir=crate::store::tests::data_dir();
        let vault=dir.join("Vault");
        for folder in ["A","B"] {
            std::fs::create_dir_all(vault.join(folder))?;
            std::fs::write(vault.join(folder).join("同名.md"),format!("{folder} 原稿"))?;
        }
        let mut db=Notebook::open(&dir.join("workspace.sqlite"),false)?;
        let root=db.import_vault("V",crate::vault::read(&vault)?.pages)?;
        let file=vault.join("A").join("同名.md");
        let older=vault.join("A").join(".paperroom-recovery-preexisting-同名.md.bak");
        std::fs::write(&older,"A 第零版")?;
        let id=db.preview_vault_pull(root,&vault)?.entries[0].id;
        for (generation,body) in ["第二版","第三版"].iter().enumerate() {
            let note=db.note(id)?.unwrap();
            db.save(id,note.version,&note.row.title,body)?;
            let plan=db.preview_vault_push(root,&vault)?;
            db.apply_vault_push(&plan,"1",&dir.join(format!("push-{generation}.sqlite")))?;
            assert_eq!(std::fs::read_to_string(&file)?,*body);
        }
        let mut old=BTreeSet::new();
        for entry in std::fs::read_dir(vault.join("A"))? {
            let path=entry?.path();
            if path.extension().is_some_and(|ext|ext=="bak"){old.insert(std::fs::read_to_string(path)?);}
        }
        assert_eq!(old,BTreeSet::from(["A 第零版".to_owned(),"A 原稿".to_owned(),"第二版".to_owned()]));
        assert_eq!(std::fs::read_to_string(older)?,"A 第零版");
        let scanned=crate::vault::scan_for_sync(&vault)?;
        assert_eq!(scanned.hidden,3);
        for (generation,(draft,shared)) in [("第二版","A 原稿"),("第三版","第二版")].into_iter().enumerate() {
            let backup=Notebook::open(&dir.join(format!("push-{generation}.sqlite")),false)?;
            assert_eq!(backup.note(id)?.unwrap().body,draft);
            let base:String=backup.connection.query_row("SELECT body FROM vault_sync_base WHERE note_id=?1",[id],|r|r.get(0))?;
            assert_eq!(base,shared);
        }
        assert_eq!(scanned.pages.len(),2);
        assert_eq!(scanned.attachments,0);
        assert!(scanned.media_paths.is_empty());
        let note=db.note(id)?.unwrap();
        db.save(id,note.version,&note.row.title,"紙間保留版")?;
        std::fs::write(&file,"磁碟保留版")?;
        let conflict=db.preview_vault_pull(root,&vault)?;
        assert_eq!(conflict.counts("1")?,(1,1));
        db.apply_vault_pull(&conflict,&vault,"1",&dir.join("pull.sqlite"))?;
        assert_eq!(db.note(id)?.unwrap().body,"紙間保留版");
        let copies:i64=db.connection.query_row("SELECT COUNT(*) FROM notes WHERE parent=?1 AND body='磁碟保留版' AND title LIKE '磁碟版本・%'",[note.row.parent],|r|r.get(0))?;
        assert_eq!(copies,1);
        assert_eq!(std::fs::read_to_string(file)?,"磁碟保留版");
        assert_eq!(std::fs::read_to_string(vault.join("B").join("同名.md"))?,"B 原稿");
        assert!(!db.preview_vault_push(root,&vault)?.actionable());
        assert!(!db.preview_vault_pull(root,&vault)?.actionable());
        let backup=Notebook::open(&dir.join("pull.sqlite"),false)?;
        assert_eq!(backup.note(id)?.unwrap().body,"紙間保留版");
        let copied_before:i64=backup.connection.query_row("SELECT COUNT(*) FROM notes WHERE body='磁碟保留版'",[],|r|r.get(0))?;
        assert_eq!(copied_before,0);
        drop(backup);
        drop(db);std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn stopped_batch_keeps_first_backup_and_rechecks_actual_files()->Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        let dir=crate::store::tests::data_dir();
        let vault=dir.join("Vault");
        std::fs::create_dir(&vault)?;
        for name in ["A.md","B.md","C.md"] { std::fs::write(vault.join(name),"old")?; }
        let mut db=Notebook::open(&dir.join("workspace.sqlite"),false)?;
        let root=db.import_vault("V",crate::vault::read(&vault)?.pages)?;
        for row in db.preview_vault_pull(root,&vault)?.entries {
            db.save(row.id,row.version,&row.title,"new")?;
        }
        let plan=db.preview_vault_push(root,&vault)?;
        let guard=std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(vault.join("B.md"))?;
        let result=db.apply_vault_push(&plan,"安全",&dir.join("before.sqlite"));
        let error=result.unwrap_err().to_string();
        assert!(error.contains("A.md →"));
        assert!(error.contains("處理中停止，請核對：B.md"));
        assert!(error.contains("尚未處理：\r\nC.md"));
        drop(guard);
        assert_eq!(std::fs::read_to_string(vault.join("A.md"))?,"new");
        assert_eq!(std::fs::read_to_string(vault.join("B.md"))?,"old");
        assert_eq!(std::fs::read_to_string(vault.join("C.md"))?,"old");
        let retry=db.preview_vault_push(root,&vault)?;
        assert_eq!(retry.select("安全")?.into_iter().collect::<Vec<_>>(),vec![1,2]);
        db.apply_vault_push(&retry,"安全",&dir.join("retry.sqlite"))?;
        assert_eq!(std::fs::read_to_string(vault.join("B.md"))?,"new");
        let backups=std::fs::read_dir(&vault)?.filter_map(|e|e.ok()).filter(|e|e.path().extension().is_some_and(|s|s=="bak")).count();
        assert_eq!(backups,3);
        drop(db);std::fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    fn unicode_push_preserves_source_format_and_checks_the_actual_file()->Result<()>{
        let dir=crate::store::tests::data_dir();
        let vault=dir.join("Vault");
        std::fs::create_dir(&vault)?;
        let path=vault.join("筆記.md");
        let original="\u{feff}舊\r\n版";
        std::fs::write(&path,original)?;
        let mut db=Notebook::open(&dir.join("workspace.sqlite"),false)?;
        let root=db.import_vault("V",crate::vault::read(&vault)?.pages)?;
        let id:i64=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1",[root],|r|r.get(0))?;
        let note=db.note(id)?.unwrap();
        db.save(id,note.version,"筆記","新\n版")?;
        let stale=db.preview_vault_push(root,&vault)?;
        assert!(stale.actionable());
        std::fs::write(&path,"外部")?;
        assert!(db.apply_vault_push(&stale,"安全",&dir.join("stale.sqlite")).is_err());
        assert_eq!(std::fs::read_to_string(&path)?,"外部");
        assert!(!db.preview_vault_push(root,&vault)?.actionable());
        std::fs::write(&path,original)?;
        let plan=db.preview_vault_push(root,&vault)?;
        db.apply_vault_push(&plan,"1",&dir.join("before.sqlite"))?;
        assert_eq!(std::fs::read_to_string(&path)?,"\u{feff}新\r\n版");
        assert_eq!(db.note(id)?.unwrap().body,"\u{feff}新\r\n版");
        assert!(!db.preview_vault_push(root,&vault)?.actionable());
        let backup=std::fs::read_dir(&vault)?.filter_map(|e|e.ok()).map(|e|e.path()).find(|p|p.extension().is_some_and(|s|s=="bak")).unwrap();
        assert_eq!(std::fs::read_to_string(backup)?,original);
        drop(db);std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
