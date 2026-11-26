use super::{Notebook, Result, is_heptabase_root, mapped_link_target, native_note_target, NATIVE_NOTE_PREFIX};
use crate::{markdown, vault::NoteLinkPaths};
use rusqlite::{Connection, OptionalExtension};
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BacklinkPosition {
    target: (i64, String, String),
    root: i64,
    pub(crate) source: i64,
    token: String,
    pub(crate) version: i64,
    path: String,
    pub(crate) range: Range<usize>,
    pub(crate) markup: String,
    pub(crate) label: String,
}

fn positions(db: &Connection, target: i64, only: Option<i64>) -> Result<Vec<BacklinkPosition>> {
    let (root, target_path, target_token): (i64, String, String) = db.query_row(
        "SELECT p.import_root,p.relative_path,i.token FROM vault_paths p JOIN notes n ON n.id=p.note_id
         JOIN notes r ON r.id=p.import_root JOIN note_link_identities i ON i.note_id=n.id
         WHERE n.id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL", [target],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or("目前的 Vault 目標不存在。")?;
    if is_heptabase_root(db,root)? {return Err("請選擇 Obsidian Vault 的筆記。".into())}
    // Include trashed paths in the resolver so trash cannot disambiguate a bare name.
    let mut paths = db.prepare("SELECT note_id,relative_path FROM vault_paths WHERE import_root=?1 LIMIT 5001")?;
    let paths = paths.query_map([root], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if paths.len()>5000 { return Err("此 Vault 超過 5,000 份筆記，無法完整核對反向連結。".into()) }
    let index=NoteLinkPaths::new(&paths);
    let mut stmt=db.prepare("SELECT n.id,p.relative_path,n.version,i.token,n.body FROM vault_paths p
        JOIN notes n ON n.id=p.note_id JOIN note_link_identities i ON i.note_id=n.id
        WHERE p.import_root=?1 AND n.id<>?2 AND n.trash_root IS NULL AND (?3 IS NULL OR n.id=?3)
        AND instr(n.body,'[')>0 ORDER BY p.relative_path,n.id")?;
    let mut records=stmt.query((root,target,only))?;
    let mut result=Vec::new();
    let mut bytes=0;
    while let Some(r)=records.next()? {
        let source:i64=r.get(0)?;
        let path:String=r.get(1)?;
        let version:i64=r.get(2)?;
        let token:String=r.get(3)?;
        let body=r.get_ref(4)?.as_str()?;
        bytes+=body.len();
        if bytes>64*1024*1024 {return Err("此 Vault 的含連結正文超過 64 MiB，無法完整核對反向連結。".into())}
        let parts=path.split('/').map(str::to_owned).collect::<Vec<_>>();
        let (mut prev,mut line)=(0,1);
        let (mut previous_hit_line,mut ordinal)=(0,0);
        for link in markdown::note_references(body) {
            let dest=&link.destination;
            let address=dest.split_once('#').map_or(dest.as_str(),|(s,_)|s);
            let resolved=if address.starts_with(NATIVE_NOTE_PREFIX){
                native_note_target(db,Some(source),address)?
            }else{mapped_link_target(&index,false,&path,&parts,link.syntax,dest,address)};
            if resolved!=Some(target){continue}
            if result.len()>=500 {return Err("此 Vault 超過 500 處反向連結，無法完整列出。".into())}
            let range=link.offset..link.end;
            let prefix=body.as_bytes().get(prev..range.start).ok_or("無法核對連結順序。")?;
            line+=prefix.iter().enumerate().filter(|&(i,&b)|b==b'\r' || b==b'\n' && (i==0 || prefix[i-1]!=b'\r')).count();
            prev=range.start;
            if line!=previous_hit_line {ordinal=0; previous_hit_line=line;}
            ordinal+=1;
            let suffix=if ordinal==1 {String::new()}else{format!("　（本行第 {ordinal} 處）")};
            let markup=body.get(range.clone()).ok_or("無法核對連結位置。")?;
            let preview:String=markup.chars().take(100).map(|c|if c.is_control(){' '}else{c}).collect();
            result.push(BacklinkPosition{target:(target,target_path.clone(),target_token.clone()),root,
                source,token:token.clone(),version,path:path.clone(),range,markup:markup.to_owned(),
                label:format!("{path}　第 {line} 行　{preview}{suffix}")});
        }
    }
    Ok(result)
}

impl Notebook {
    pub(crate) fn supports_vault_backlink_positions(&self,id:i64)->Result<bool> {
        Ok(self.connection.prepare_cached("SELECT 1 FROM vault_paths p JOIN notes n ON n.id=p.note_id
            JOIN notes r ON r.id=p.import_root WHERE n.id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL
            AND NOT EXISTS(SELECT 1 FROM heptabase_import_roots h WHERE h.import_root=p.import_root)")?.exists([id])?)
    }

    pub(crate) fn vault_backlink_positions(&self,target:i64)->Result<Vec<BacklinkPosition>> {
        let tx=self.connection.unchecked_transaction()?;
        let hits=positions(&tx,target,None)?;
        tx.commit()?;
        Ok(hits)
    }
    pub(crate) fn recheck_vault_backlink(&self,hit:&BacklinkPosition)->Result<super::Note> {
        let tx=self.connection.unchecked_transaction()?;
        if !positions(&tx,hit.target.0,Some(hit.source))?.contains(hit) {
            return Err("反向連結的來源或位置已變更，請重新選取。".into())
        }
        let note=self.note(hit.source)?.ok_or("找不到反向連結來源。")?;
        tx.commit()?;
        Ok(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;

    #[test]
    fn repeated_links_are_individual_saved_spans_and_stale_or_ambiguous_hits_do_not_rebind()->Result<()> {
        let dir=crate::store::tests::data_dir();
        let db_path=dir.join("workspace.sqlite3");
        let backup=dir.join("copy.sqlite3");
        let mut db=Notebook::open(&db_path,false)?;
        let page=|path:&str,body:&str| ImportedPage {
            title:path.rsplit('/').next().unwrap().trim_end_matches(".md").into(),
            body:body.into(), parts:path.split('/').map(str::to_owned).collect()
        };
        let body="# 😀\r\n[first](A/Target.md)\n\r[[A/Target|second]] [[A/Target|second]]\n[[Target]] `[code](A/Target.md)` [decoy](B/Target.md)";
        let root=db.import_vault("Synthetic",vec![page("A/Target.md","alpha"),page("B/Target.md","bravo"),page("Index.md",body)])?;
        let target=db.connection.query_row("SELECT note_id FROM vault_paths WHERE relative_path='A/Target.md'",[],|r|r.get::<_,i64>(0))?;
        assert!(!db.supports_vault_backlink_positions(root)?);
        let hits=db.vault_backlink_positions(target)?;
        assert_eq!(hits.len(),3);
        assert_eq!(hits[1].markup,hits[2].markup);
        assert_eq!(hits[2].label,format!("{}　（本行第 2 處）",hits[1].label));
        assert!(hits[1].range.end<hits[2].range.start);
        assert_eq!(hits[0].markup,"[first](A/Target.md)");
        assert!(hits[1].label.contains("第 4 行"));
        assert_eq!(&body[hits[1].range.clone()],"[[A/Target|second]]");
        let source=hits[0].source;
        db.recheck_vault_backlink(&hits[1])?;
        db.backup(&backup)?;
        drop(db);
        let mut db=Notebook::open(&db_path,false)?;
        db.recheck_vault_backlink(&hits[0])?;
        db.trash(source)?;
        assert!(db.vault_backlink_positions(target)?.is_empty());
        assert!(db.recheck_vault_backlink(&hits[0]).is_err());
        db.restore(source)?;
        let restored=db.vault_backlink_positions(target)?;
        assert_eq!(restored.len(),3);
        let n=db.note(source)?.unwrap();
        db.save(source,n.version,&n.row.title,&format!("changed\n{}",n.body))?;
        assert!(db.recheck_vault_backlink(&restored[1]).is_err());
        let isolated=Notebook::open(&backup,false)?;
        assert_eq!(isolated.vault_backlink_positions(target)?,hits);
        assert_eq!(isolated.note(source)?.unwrap().body,body);
        Ok(())
    }
}
