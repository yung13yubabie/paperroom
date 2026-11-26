//! Export only the witnessed attachment from an active note's saved revision.
use super::{Notebook,Result,Revision};
use super::super::{attachments,snapshot_sha256::sha256};
use rusqlite::{params,OptionalExtension};
use std::path::Path;

pub(crate) struct HistoricalFile {
    owner:i64,
    token:String,
    revision:i64,
    original:i64,
    pub(crate) name:String,
    kind:String,
    origin:Option<String>,
    size:usize,
    digest:[u8;32],
}

impl HistoricalFile {
    pub(crate) fn label(&self)->String {
        format!("[#{}]　{}　{} B",self.original,self.name,self.size)
    }
}

impl Notebook {
    pub(crate) fn history_attachment_choices(&self,note:i64,revision:&Revision)->Result<Vec<HistoricalFile>> {
        let tx=self.connection.unchecked_transaction()?;
        let token:String=tx.query_row("SELECT i.token FROM note_revisions r JOIN notes n ON n.id=r.note_id
            JOIN note_link_identities i ON i.note_id=n.id WHERE n.id=?1 AND n.trash_root IS NULL
            AND r.id=?2 AND r.prior_version=?3 AND r.title=?4 AND strftime('%Y-%m-%d %H:%M:%SZ',r.captured_at,'unixepoch')=?5",
            params![note,revision.id,revision.version,revision.title,revision.timestamp],|r|r.get(0))
            .optional()?.ok_or("筆記或版本已變更，請重新選取。")?;
        let mut files=Vec::new();
        {
            let mut stmt=tx.prepare("SELECT h.original_id,h.name,h.kind,h.origin,
                CASE WHEN typeof(h.content)='blob' AND length(h.content) BETWEEN 1 AND ?2 THEN h.content END
                FROM note_revision_attachments h JOIN note_revision_state s ON s.revision_id=h.revision_id
                WHERE h.revision_id=?1 ORDER BY h.original_id LIMIT ?3")?;
            let mut rows=stmt.query(params![revision.id,attachments::MAX_FILE_BYTES as i64,attachments::MAX_PER_NOTE+1])?;
            while let Some(row)=rows.next()? {
                if files.len()>=attachments::MAX_PER_NOTE as usize {return Err("歷史版本的附件數量超過上限。".into());}
                let id:i64=row.get(0)?;
                let name:String=row.get(1)?;
                let kind:String=row.get(2)?;
                let bytes=row.get_ref(4)?.as_blob().map_err(|_|"歷史附件的內容或大小無效。")?;
                if id<=0 || name.contains('/') || !super::super::notion_handoff::windows_zip_path(&name)
                    || attachments::file_kind(Path::new(&name)).ok()!=Some(kind.as_str())
                    || !attachments::has_expected_signature(&kind,bytes) {
                    return Err("歷史附件的檔名、類型或內容不符。".into());
                }
                files.push(HistoricalFile {owner:note,token:token.clone(),revision:revision.id,original:id,
                    name,kind,origin:row.get(3)?,size:bytes.len(),digest:sha256(bytes)});
            }
        }
        tx.commit()?;
        Ok(files)
    }

    pub(crate) fn export_history_attachment(&self,file:&HistoricalFile,destination:&Path)->Result<()> {
        let bytes:Vec<u8>=self.connection.query_row("SELECT h.content FROM note_revisions r
            JOIN notes n ON n.id=r.note_id JOIN note_link_identities i ON i.note_id=n.id
            JOIN note_revision_state s ON s.revision_id=r.id JOIN note_revision_attachments h ON h.revision_id=s.revision_id
            WHERE n.id=?1 AND n.trash_root IS NULL AND i.token=?2 AND r.id=?3 AND h.original_id=?4
            AND h.name=?5 COLLATE BINARY AND h.kind=?6 AND h.origin IS ?7 AND typeof(h.content)='blob' AND length(h.content)=?8",
            params![file.owner,file.token,file.revision,file.original,file.name,file.kind,file.origin,file.size as i64],|r|r.get(0))
            .optional()?.ok_or("歷史附件已變更或版本已移除，請重新選取。")?;
        if sha256(&bytes)!=file.digest {return Err("歷史附件內容已變更，請重新選取。".into());}
        attachments::publish_attachment(destination,&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn historical_copy_keeps_exact_identity_and_refuses_stale_or_malformed_sources()->Result<()> {
        let dir=super::super::super::tests::data_dir();
        let live=dir.join("live.db");
        let backup=dir.join("backup.db");
        let mut db=Notebook::open(&live,false)?;
        let note=db.add(None,"同名","第一版")?;
        let decoy=db.add(None,"同名","不同筆記")?;
        let source=dir.join("同名.txt");
        fs::write(&source,b"old-A")?;
        let old=db.import_attachment(note,&source)?;
        fs::write(&source,b"old-B")?;
        let twin=db.import_attachment(note,&source)?;
        db.remove_attachment(old)?;
        let revision=db.note_revisions(note)?.remove(0);
        fs::write(&source,b"new-C")?;
        db.import_attachment(note,&source)?;
        db.import_attachment(decoy,&source)?;
        let file=db.history_attachment_choices(note,&revision)?.remove(0);
        assert_eq!((file.original,file.size),(old,5));
        assert!(db.history_attachment_choices(decoy,&revision).is_err());
        db.backup(&backup)?;
        let isolated=dir.join("restored.db");
        fs::copy(&backup,&isolated)?;
        for (dbpath,suffix) in [(&live,"live"),(&isolated,"restored")] {
            let db=Notebook::open(dbpath,false)?;
            let before:i64=db.connection.query_row("SELECT total_changes()",[],|r|r.get(0))?;
            let choices=db.history_attachment_choices(note,&revision)?;
            assert_eq!(choices.iter().map(|f|f.original).collect::<Vec<_>>(),[old,twin]);
            for (f,wanted) in choices.iter().zip([b"old-A",b"old-B"]) {
                let output=dir.join(format!("{suffix}-{}.txt",f.original));
                db.export_history_attachment(f,&output)?;
                assert_eq!(fs::read(&output)?,wanted);
                fs::write(&output,b"already there")?;
                assert!(db.export_history_attachment(f,&output).is_err());
                assert_eq!(fs::read(output)?,b"already there");
            }
            assert_eq!(db.connection.query_row("SELECT total_changes()",[],|r|r.get::<_,i64>(0))?,before);
        }
        let absent=dir.join("must-not-exist.txt");
        let writer=Notebook::open(&live,false)?;
        writer.connection.execute("UPDATE note_revision_attachments SET content=?3 WHERE revision_id=?1 AND original_id=?2",
            params![revision.id,old,b"other".as_slice()])?;
        assert!(db.export_history_attachment(&file,&absent).is_err());
        writer.connection.execute("UPDATE note_revision_attachments SET name='../unusable.txt' WHERE revision_id=?1 AND original_id=?2",params![revision.id,old])?;
        assert!(db.history_attachment_choices(note,&revision).is_err());
        writer.connection.execute("UPDATE note_revision_attachments SET name='x.pdf',kind='pdf' WHERE revision_id=?1 AND original_id=?2",params![revision.id,old])?;
        assert!(db.history_attachment_choices(note,&revision).is_err());
        // Simulate normal retention removing this exact revision while Save As is open.
        writer.connection.execute("DELETE FROM note_revisions WHERE id=?1",[revision.id])?;
        assert!(db.export_history_attachment(&file,&absent).is_err());
        assert!(db.history_attachment_choices(note,&revision).is_err());
        assert!(!absent.exists());
        drop(writer);drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
