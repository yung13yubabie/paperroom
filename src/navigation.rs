//! Note navigation uses stable database IDs. Outline locations refer to the unmodified source.
use super::{Notebook,Result, note_links::NATIVE_NOTE_PREFIX};
use crate::markdown::block_anchors::{BlockLookup, resolve_block, unique_block_sources};
use rusqlite::{OptionalExtension, params};
use super::vault_search::{MAX_SCAN, TagFilter, TagMatch, matches_alias, matches_tag};

pub(crate) struct SwitchHit { pub id:i64, pub label:String }

impl Notebook {
    pub(crate) fn quick_notes(&self,query:&str)->Result<Vec<SwitchHit>> {
        let query=query.trim();
        if query.len()>4096 || query.chars().any(char::is_control) {return Err("搜尋文字最多 4096 位元組，不能含控制字元。".into());}
        let tag = if let Some(term) = query.strip_prefix("tag:=").filter(|term| term.starts_with('#')) {
            Some(TagFilter::parse(term, TagMatch::Exact)?)
        } else if let Some(term) = query.strip_prefix("tag:").filter(|term| term.starts_with('#')) {
            Some(TagFilter::parse(term, TagMatch::Descendants)?)
        } else {
            None
        };
        let needle=query.to_lowercase();
        let mut stmt=self.connection.prepare("SELECT n.id,n.title,r.title,r.id,p.relative_path,
            CASE WHEN ?2 THEN n.body
                 WHEN p.note_id IS NOT NULL AND ?1<>'' AND (substr(n.body,1,3)='---' OR substr(n.body,1,4)=char(65279)||'---')
                 THEN substr(n.body,1,65544) ELSE '' END
            FROM notes n LEFT JOIN vault_paths p ON p.note_id=n.id LEFT JOIN notes r ON r.id=p.import_root
            WHERE n.trash_root IS NULL AND r.trash_root IS NULL
            AND (NOT ?2 OR p.note_id IS NOT NULL) ORDER BY n.id")?;
        let mut rows=stmt.query(rusqlite::params![query, tag.is_some()])?;
        let mut hits=Vec::new();
        let mut scanned=0;
        while let Some(row)=rows.next()? {
            let id:i64=row.get(0)?;
            let title=row.get_ref(1)?.as_str()?;
            let path:Option<&str>=row.get_ref(4)?.as_str_or_null()?;
            let body=row.get_ref(5)?.as_str()?;
            scanned+=body.len();
            if scanned > MAX_SCAN {return Err("快速搜尋超過 128 MiB，請改用指定範圍的 Vault 來源搜尋。".into());}
            let rank = if let Some(filter) = &tag {
                if !matches_tag(body, filter)? { continue; }
                0
            } else if query==title || path==Some(query) {0}
                else if !body.is_empty() && matches_alias(body,query)? {1}
                else if title.to_lowercase().contains(&needle) || path.is_some_and(|p|p.to_lowercase().contains(&needle)){2}
                else {continue};
            let label=if let Some(path)=path {
                format!("{title}　{} (#{}) / {path}　[#{id}]",row.get_ref(2)?.as_str()?,row.get::<_,i64>(3)?)
            } else {format!("{title}　[#{id}]")};
            hits.push((rank,SwitchHit{id,label}));
        }
        hits.sort_by(|a,b|a.0.cmp(&b.0).then_with(||a.1.label.cmp(&b.1.label)));
        Ok(hits.into_iter().map(|(_,hit)|hit).collect())
    }
}


const BLOCK_RESULT_LIMIT: usize = 200;

pub(crate) struct BlockLinkHit {
    pub label: String,
    snapshot: BlockNoteSnapshot,
    anchor: String,
}

impl Notebook {
    pub(crate) fn block_link_candidates(
        &self, source_id: i64, vault_scope: bool, query: &str,
    ) -> Result<(Vec<BlockLinkHit>, bool)> {
        let query = query.trim();
        if query.len() > 4096 || query.chars().any(char::is_control) {
            return Err("區塊查詢最多 4096 位元組，不能含控制字元。".into());
        }
        let query = query.strip_prefix("#^").unwrap_or(query).to_lowercase();
        let tx = self.connection.unchecked_transaction()?;
        let root = tx.query_row(
            "SELECT coalesce(p.import_root, (
                 SELECT import_root FROM vault_paths WHERE import_root=n.id LIMIT 1
             )) FROM notes n
             LEFT JOIN vault_paths p ON p.note_id=n.id
             LEFT JOIN notes r ON r.id=p.import_root
             WHERE n.id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL",
            [source_id], |r| r.get::<_, Option<i64>>(0),
        ).optional()?.ok_or("找不到使用中的來源筆記。")?;
        if vault_scope && root.is_none() {
            return Err("這篇筆記沒有對應的 Vault 匯入批次。".into());
        }

        let mut statement = tx.prepare(
            "SELECT n.id,n.title,n.body,n.version,p.relative_path FROM notes n
             LEFT JOIN vault_paths p ON p.note_id=n.id
             WHERE n.trash_root IS NULL AND
                 ((NOT ?2 AND n.id=?1) OR (?2 AND p.import_root=?3))
             ORDER BY p.relative_path,n.id",
        )?;
        let mut rows = statement.query(params![source_id, vault_scope, root])?;
        let mut scan_bytes = 0;
        let mut hits = Vec::new();
        let mut truncated = false;
        'notes: while let Some(row) = rows.next()? {
            let body = row.get_ref(2)?.as_str()?;
            scan_bytes += body.len();
            if scan_bytes > MAX_SCAN {
                return Err("區塊來源超過 128 MiB，請改在單篇筆記中搜尋。".into());
            }
            let id = row.get(0)?;
            let title = row.get_ref(1)?.as_str()?;
            let path = row.get_ref(4)?.as_str_or_null()?;
            let snapshot = BlockNoteSnapshot::from_parts(id, row.get(3)?, title, body);
            for (block, range) in unique_block_sources(body) {
                let source = &body[range];
                if !query.is_empty() && !block.to_lowercase().contains(&query)
                    && !source.to_lowercase().contains(&query)
                {
                    continue;
                }
                if hits.len() == BLOCK_RESULT_LIMIT {
                    truncated = true;
                    break 'notes;
                }
                let mut words = source.split_whitespace()
                    .flat_map(|word| " ".chars().chain(word.chars())).skip(1);
                let mut excerpt = words.by_ref().take(54).collect::<String>();
                if words.next().is_some() { excerpt.push('…'); }
                let source = path.unwrap_or(title);
                let anchor = format!("#^{block}");
                let label = format!("{source} [#{id}]　{anchor}　{excerpt}");
                hits.push(BlockLinkHit { snapshot, anchor, label });
            }
        }
        drop(rows);
        drop(statement);
        tx.commit()?;
        Ok((hits, truncated))
    }

    pub(crate) fn stable_block_link(&self, hit: &BlockLinkHit) -> Result<String> {
        let tx = self.connection.unchecked_transaction()?;
        let note = self.note(hit.snapshot.id)?;
        if hit.snapshot.state(note.as_ref()) != BlockNoteState::Unchanged {
            return Err("區塊來源已修改或移除，請重新搜尋。".into());
        }
        let note = note.ok_or("找不到使用中的區塊來源。")?;
        if !matches!(resolve_block(&note.body, &hit.anchor), BlockLookup::Found { .. }) {
            return Err("區塊已無法唯一定位，請重新搜尋。".into());
        }
        let token: String = tx.query_row(
            "SELECT token FROM note_link_identities WHERE note_id=?1", [note.row.id], |row| row.get(0),
        )?;
        tx.commit()?;
        Ok(format!("[[{NATIVE_NOTE_PREFIX}{}:{token}{}]]", note.row.id, hit.anchor))
    }
}

pub(crate) struct Heading {pub title:String,pub level:u8,pub line:usize,pub start:usize,pub end:usize}

pub(crate) fn outline(body:&str)->Vec<Heading> {
    // The multiline Win32 EDIT receives CRLF for every source line ending.
    let normalized;
    let body=if body.contains('\r') {
        normalized=body.replace("\r\n","\n").replace('\r',"\n");
        normalized.as_str()
    } else {body};
    let (mut byte,mut utf16,mut line)=(0,0,1);
    let mut headings=Vec::new();
    for (at,level,title) in crate::markdown::source_headings(body) {
        let prefix=&body[byte..at];
        let breaks=prefix.matches('\n').count();
        utf16+=prefix.encode_utf16().count()+breaks;
        line+=breaks;
        let length=body[at..].split(['\r','\n']).next().unwrap_or("").encode_utf16().count();
        headings.push(Heading {title,level,line,start:utf16,end:utf16+length});
        byte=at;
    }
    headings
}

pub(crate) struct SourceSelection {
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

pub(crate) fn editor_source_selection(body: &str, bytes: std::ops::Range<usize>) -> Option<SourceSelection> {
    fn position(text: &str) -> (usize, usize) {
        let mut chars = text.chars().peekable();
        let (mut units, mut breaks) = (0, 0);
        while let Some(c) = chars.next() {
            if matches!(c, '\r' | '\n') {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                units += 2;
                breaks += 1;
            } else {
                units += c.len_utf16();
            }
        }
        (units, breaks)
    }

    let prefix = body.get(..bytes.start)?;
    let span = body.get(bytes)?;
    let (start, lines_before) = position(prefix);
    let (length, _) = position(span);
    Some(SourceSelection { start, end: start + length, line: lines_before + 1 })
}

#[derive(Clone, Copy)]
pub(crate) struct BlockNoteSnapshot {
    pub id: i64,
    version: i64,
    text: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockNoteState {
    Unchanged,
    Revised,
    Unavailable,
}

impl BlockNoteSnapshot {
    pub(crate) fn capture(note: &super::Note) -> Self {
        Self::from_parts(note.row.id, note.version, &note.row.title, &note.body)
    }

    pub(super) fn from_parts(id: i64, version: i64, title: &str, body: &str) -> Self {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        title.hash(&mut hasher);
        body.hash(&mut hasher);
        Self { id, version, text: hasher.finish() }
    }

    pub(crate) fn state(self, note: Option<&super::Note>) -> BlockNoteState {
        let Some(note) = note.filter(|note| note.row.id == self.id && note.row.trash_root.is_none()) else {
            return BlockNoteState::Unavailable;
        };
        match note.version.cmp(&self.version) {
            std::cmp::Ordering::Greater => BlockNoteState::Revised,
            std::cmp::Ordering::Equal if Self::capture(note).text == self.text => BlockNoteState::Unchanged,
            _ => BlockNoteState::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    #[test]
    fn block_jump_snapshot_rejects_stale_positions_and_unavailable_origins() -> Result<()> {
        let mut db = Notebook::open(std::path::Path::new(":memory:"), false)?;
        let origin = db.add(None, "原處", "📽 回到這一行")?;
        let note = db.note(origin)?.unwrap();
        let snapshot = BlockNoteSnapshot::capture(&note);
        assert_eq!(snapshot.state(Some(&note)), BlockNoteState::Unchanged);
        let other = db.add(None, "原處", "📽 回到這一行")?;
        assert_eq!(snapshot.state(db.note(other)?.as_ref()), BlockNoteState::Unavailable);
        let mut same_revision_different_text = db.note(origin)?.unwrap();
        same_revision_different_text.body = "不同文字".into();
        assert_eq!(snapshot.state(Some(&same_revision_different_text)), BlockNoteState::Unavailable);
        db.save(origin, note.version, "新標題", "更新的原筆記")?;
        assert_eq!(snapshot.state(db.note(origin)?.as_ref()), BlockNoteState::Revised);
        db.trash(origin)?;
        assert_eq!(snapshot.state(db.note(origin)?.as_ref()), BlockNoteState::Unavailable);
        db.restore(origin)?;
        assert_eq!(snapshot.state(db.note(origin)?.as_ref()), BlockNoteState::Revised);
        assert_eq!(snapshot.state(None), BlockNoteState::Unavailable);
        Ok(())
    }

    #[test]
    fn navigation_matches_stable_source_paths_titles_and_aliases() -> Result<()> {
        let mut db=Notebook::open(std::path::Path::new(":memory:"),false)?;
        let page=|folder:&str|ImportedPage{parts:vec![folder.into(),"場景.md".into()],title:"場景".into(),body:"---\naliases: [鏡位]\n---\n".into()};
        db.import_vault("專案",vec![page("甲"),page("乙")])?;
        let other=db.add(None,"鏡位清單","")?;
        let hits=db.quick_notes("鏡位")?;
        assert_eq!(hits.len(),3);assert_eq!(hits[2].id,other);
        let specific=db.quick_notes("甲/場景.md")?;
        assert_eq!(specific.len(),1);
        assert!(hits[..2].iter().any(|h|h.id==specific[0].id));
        db.trash(specific[0].id)?;
        assert_eq!(db.quick_notes("場景")?.len(),1);
        Ok(())
    }

    #[test]
    fn block_search_copies_exact_stable_id_and_rejects_duplicate_or_edited_sources() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("blocks.sqlite3");
        let (source, target, link) = {
            let mut db = Notebook::open(&path, false)?;
            let page = |folder: &str, body: &str| ImportedPage {
                parts: vec![folder.into(), "場景.md".into()],
                title: "場景".into(),
                body: body.into(),
            };
            db.import_vault("合成批次", vec![
                page("甲", "先拍 ^part\r\n\r\n重複 ^part"),
                page("乙", "🎬 先拍 ^part"),
            ])?;
            db.import_vault("另一批次", vec![page("乙", "🎬 先拍 ^part")])?;
            let mut hits = db.quick_notes("甲/場景.md")?;
            let source = hits.remove(0).id;
            assert!(db.block_link_candidates(source, false, "")?.0.is_empty());
            let (hits, truncated) = db.block_link_candidates(source, true, "#^PART")?;
            assert_eq!(hits.len(), 1);
            assert!(!truncated);
            assert!(hits[0].label.contains("乙/場景.md") && hits[0].label.contains("🎬 先拍"));
            let target = hits[0].snapshot.id;
            let link = db.stable_block_link(&hits[0])?;
            let original = db.note(source)?.unwrap();
            db.save(source, original.version, &original.row.title, &link)?;
            (source, target, link)
        };
        {
            let db = Notebook::open(&path, false)?;
            let note = db.note(source)?.unwrap();
            assert_eq!(note.body, link);
            let refs = db.vault_note_links(&note)?;
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].note_id, target);
            assert_eq!(refs[0].anchor.as_deref(), Some("#^part"));
            assert!(db.graph_neighbors(&note)?.0.iter().any(|(id, ..)| *id == target));
            let mut other = Notebook::open(std::path::Path::new(":memory:"), false)?;
            other.connection.execute(
                "INSERT INTO notes(id,title,body) VALUES(?1,'同編號','🎬 先拍 ^part')", [target],
            )?;
            let pasted = other.add(None, "跨工作區貼上", &link)?;
            let pasted = other.note(pasted)?.unwrap();
            assert!(other.vault_note_links(&pasted)?.is_empty());
            assert!(other.graph_neighbors(&pasted)?.0.is_empty());

            let backup = dir.join("backup.sqlite3");
            db.backup(&backup)?;
            let restored = Notebook::open(&backup, false)?;
            assert_eq!(restored.vault_note_links(&note)?[0].note_id, target);
            drop(restored);

            let (hits, _) = db.block_link_candidates(source, true, "先拍")?;
            let target = db.note(target)?.unwrap();
            db.save(target.row.id, target.version, "場景 新版", &target.body)?;
            assert!(db.stable_block_link(&hits[0]).is_err());
            assert_eq!(db.vault_note_links(&note)?[0].note_id, target.row.id);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn outline_disambiguates_utf16_offsets_and_skips_frontmatter_and_code() {
        let body="---\r\naliases: [演員]\r\n---\r\n😀\r\n# 場景\r\n```\r\n# 隱藏\r\n```\r\n## 場景";
        let hits=outline(body);
        assert_eq!(hits.len(),2);assert_eq!((hits[0].line,hits[1].line),(5,9));
        let buf:Vec<_>=body.encode_utf16().collect();
        assert_eq!(String::from_utf16(&buf[hits[1].start..hits[1].end]).unwrap(),"## 場景");
        for ending in ["\n","\r"] {
            let normalized=body.replace("\r\n",ending);
            let selected=outline(&normalized);
            let last=selected.last().unwrap();
            assert_eq!(String::from_utf16(&buf[last.start..last.end]).unwrap(),"## 場景");
        }
    }
}
