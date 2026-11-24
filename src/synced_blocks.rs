//! Native synchronized Markdown blocks share a saved note's unique block ID.
use super::{Note, Notebook, Result, save_in};
use super::navigation::{SourceSelection, editor_source_selection};
use super::transclusion::{EmbedReference, embed_references, native_embed_target};
use crate::markdown::block_anchors::{BlockLookup, has_block_markers, resolve_block};
use pulldown_cmark::{Event, Options, Parser, Tag};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use std::ops::Range;

const EDIT_LIMIT: usize = 256 * 1024;
const LIST_LIMIT: usize = 200;
const MANAGEMENT_SCAN: usize = 16 * 1024 * 1024;

pub(crate) struct SyncedPlacement {
    host: i64,
    host_version: i64,
    target: i64,
    target_version: i64,
    destination: String,
    span: Range<usize>,
    pub title: String,
    pub block: String,
    pub anchor: String,
    pub line: usize,
}

pub(crate) struct SyncConsumer {
    pub note: i64,
    pub label: String,
}

fn native_block<'a>(db: &Connection, destination: &'a str) -> Result<Option<(i64, &'a str)>> {
    let Some((path, block_id)) = destination.split_once("#^") else { return Ok(None) };
    let Some(id) = native_embed_target(db, path)? else { return Ok(None) };
    let anchor = &destination[path.len()..];
    if block_id.is_empty() { return Ok(None); }
    Ok(Some((id, anchor)))
}

fn block_range(body: &str, anchor: &str) -> Result<Range<usize>> {
    match resolve_block(body, anchor) {
        BlockLookup::Found { source, .. } => Ok(source),
        _ => Err("原始區塊遺失或 ID 重複。請檢查來源後再試。".into()),
    }
}

fn normalize_for(text: &str, previous: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if previous.contains("\r\n") && !previous.replace("\r\n", "").contains(['\r', '\n']) {
        text.replace('\n', "\r\n")
    } else if previous.contains('\r') && !previous.contains('\n') {
        text.replace('\n', "\r")
    } else {
        text
    }
}

fn replace_exact(body: &str, span: Range<usize>, replacement: &str) -> Result<String> {
    if replacement.len() > EDIT_LIMIT
        || body.len().saturating_sub(span.len()).saturating_add(replacement.len()) > super::MAX_NOTE_BYTES
    {
        return Err("變更超出區塊或筆記的大小上限。".into());
    }
    let mut next = body.to_owned();
    next.replace_range(span, replacement);
    Ok(next)
}

impl Notebook {
    fn active_sync_note(&self, id: i64) -> Result<Note> {
        self.note(id)?.filter(|note| note.row.trash_root.is_none())
            .ok_or_else(|| "來源或引用筆記不存在，或已移到垃圾桶。".into())
    }

    pub(crate) fn append_synced_block(&self, host: i64, version: i64, markup: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let page = self.active_sync_note(host)?;
        if page.version != version { return Err("引用筆記已變更，請重開後再插入。".into()); }
        // Treat the exact string as a single authored embed before writing anything.
        let mut references = embed_references(markup);
        let Some(EmbedReference { range, destination }) = references.next() else {
            return Err("找不到可插入的區塊。".into());
        };
        if range != (0..markup.len()) || references.next().is_some() {
            return Err("同步區塊標記格式不符。".into());
        }
        let (target, anchor) = native_block(&tx, &destination)?.ok_or("找不到有效的區塊來源。")?;
        if target == host { return Err("請在另一篇筆記插入同步區塊。".into()); }
        let origin = self.active_sync_note(target)?;
        block_range(&origin.body, anchor)?;
        let separator = if page.body.is_empty() { String::new() } else { normalize_for("\n\n", &page.body) };
        let span = page.body.len()..page.body.len();
        let next = replace_exact(&page.body, span, &(separator + markup))?;
        // In a note ending in an unclosed code fence, an appended marker is still code.
        if !embed_references(&next).any(|entry| entry.range.end == next.len() && entry.destination == destination) {
            return Err("筆記末尾的 Markdown 結構無法嵌入。請先關閉程式碼區塊，再插入。".into());
        }
        save_in(&tx, host, version, &page.row.title, &next)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn synchronized_blocks(&self, host: i64) -> Result<(Vec<SyncedPlacement>, bool)> {
        let tx = self.connection.unchecked_transaction()?;
        let page = self.active_sync_note(host)?;
        let mut placements = Vec::new();
        let mut remaining_bytes = MANAGEMENT_SCAN;
        let mut previous = 0;
        let mut line = 1;
        let mut limited = false;
        for (i, reference) in embed_references(&page.body).enumerate() {
            if i == LIST_LIMIT {
                limited = true;
                break;
            }
            line += page.body.as_bytes()[previous..=reference.range.start].windows(2)
                .filter(|pair| pair[0] == b'\n' || (pair[0] == b'\r' && pair[1] != b'\n')).count();
            previous = reference.range.start;
            let Some((target, anchor)) = native_block(&tx, &reference.destination)? else { continue };
            if target == host { continue; }
            let Some(source) = self.note(target)?.filter(|note| note.row.trash_root.is_none()) else { continue };
            let Some(remaining) = remaining_bytes.checked_sub(source.body.len()) else {
                limited = true;
                break;
            };
            remaining_bytes = remaining;
            let Ok(range) = block_range(&source.body, anchor) else { continue };
            let text = &source.body[range];
            if text.len() > EDIT_LIMIT { continue; }
            placements.push(SyncedPlacement {
                host, host_version: page.version, target, target_version: source.version,
                title: source.row.title, anchor: anchor.into(), block: text.into(),
                span: reference.range, destination: reference.destination, line,
            });
        }
        tx.commit()?;
        Ok((placements, limited))
    }

    fn validate_sync(&self, hit: &SyncedPlacement) -> Result<(Note, Note, Range<usize>)> {
        let host = self.active_sync_note(hit.host)?;
        let origin = self.active_sync_note(hit.target)?;
        if host.version != hit.host_version || origin.version != hit.target_version
            || native_block(&self.connection, &hit.destination)? != Some((hit.target, &*hit.anchor))
            || !embed_references(&host.body).any(|entry| entry.range == hit.span && entry.destination == hit.destination)
        {
            return Err("來源或引用筆記已變更，請重新開啟同步區塊。".into());
        }
        let range = block_range(&origin.body, &hit.anchor)?;
        if origin.body.get(range.clone()) != Some(hit.block.as_str()) {
            return Err("同步區塊已變更，請重新開啟同步區塊。".into());
        }
        Ok((host, origin, range))
    }

    pub(crate) fn synchronized_source(&self, hit: &SyncedPlacement) -> Result<(i64, i64, SourceSelection)> {
        let tx = self.connection.unchecked_transaction()?;
        let (_, source, span) = self.validate_sync(hit)?;
        let selection = editor_source_selection(&source.body, span).ok_or("無法定位共用區塊的原文。")?;
        tx.commit()?;
        Ok((source.row.id, source.version, selection))
    }

    pub(crate) fn update_synced_block(&self, hit: &SyncedPlacement, edited: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let (_, origin, span) = self.validate_sync(hit)?;
        let edited = normalize_for(edited, &origin.body);
        if edited.trim().is_empty() {
            return Err("共用原文需要內容。若要刪除區塊，請從來源筆記處理。".into());
        }
        if edited != hit.block {
            if has_block_markers(&hit.block) || has_block_markers(&edited) {
                return Err("共用原文包含其他區塊 ID。請從來源筆記編輯，以免改錯連結。".into());
            }
            let expected = span.start..span.start+edited.len();
            let next = replace_exact(&origin.body, span, &edited)?;
            if block_range(&next, &hit.anchor)? != expected {
                return Err("修改後的 Markdown 會改變區塊邊界或 ID。請從來源筆記檢查格式。".into());
            }
            save_in(&tx, hit.target, origin.version, &origin.row.title, &next)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn detach_synced_block(&self, hit: &SyncedPlacement) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let (host, _, _) = self.validate_sync(hit)?;
        let left = host.body[..hit.span.start].rsplit(['\r', '\n']).next().unwrap_or("");
        let right = host.body[hit.span.end..].split(['\r', '\n']).next().unwrap_or("");
        if !left.is_empty() || !right.is_empty() {
            return Err("請先將內文中的同步標記移到獨立一行，再解除同步。".into());
        }
        if has_block_markers(&hit.block) || Parser::new_ext(&hit.block, Options::ENABLE_WIKILINKS)
            .any(|event| matches!(event, Event::Start(Tag::Link { .. } | Tag::Image { .. })))
        {
            return Err("共用原文含連結、內嵌或其他區塊 ID。請從來源複製想保留的內容。".into());
        }
        let literal = normalize_for(&hit.block, &host.body);
        let next = replace_exact(&host.body, hit.span.clone(), &literal)?;
        save_in(&tx, hit.host, host.version, &host.row.title, &next)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn synced_consumers(&self, target: i64, markup: &str) -> Result<(Vec<SyncConsumer>, bool)> {
        let tx = self.connection.unchecked_transaction()?;
        let source = self.active_sync_note(target)?;
        let literal = embed_references(markup).next().ok_or("請選擇有效的區塊。")?.destination;
        let Some((id, anchor)) = native_block(&tx, &literal)? else { return Err("請選擇有效的區塊。".into()) };
        if id != target { return Err("來源區塊已變更。".into()); }
        block_range(&source.body, anchor)?;
        let mut statement = tx.prepare(
            "SELECT n.id,n.title,n.body,p.relative_path FROM notes n LEFT JOIN vault_paths p ON p.note_id=n.id
             WHERE n.trash_root IS NULL AND n.id<>?2 AND instr(n.body,?1)>0 ORDER BY n.id",
        )?;
        let mut rows = statement.query(params![literal, target])?;
        let mut output = Vec::new();
        let mut bytes = 0;
        let mut limited = false;
        while let Some(row) = rows.next()? {
            let body = row.get_ref(2)?.as_str()?;
            bytes += body.len();
            if bytes > super::vault_search::MAX_SCAN {
                limited = true;
                break;
            }
            let count = embed_references(body).filter(|entry| entry.destination == literal).count();
            if count == 0 { continue; }
            if output.len() == LIST_LIMIT { limited = true; break; }
            let note = row.get(0)?;
            let title = row.get_ref(1)?.as_str()?;
            let path = row.get_ref(3)?.as_str_or_null()?;
            let label = format!("{} [#{note}]　{count} 處", path.unwrap_or(title));
            output.push(SyncConsumer { note, label });
        }
        drop(rows);
        drop(statement);
        tx.commit()?;
        Ok((output, limited))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_edits_follow_one_source_trash_and_detach_across_backup() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let path = dir.join("live.sqlite3");
        let backup = dir.join("portable.sqlite3");
        let (origin, left, right) = {
            let mut db = Notebook::open(&path, false)?;
            let origin = db.add(None, "唯一區塊", "前行\r\n\r\n合成第一版 ^shared\r\n\r\n後行\r\n")?;
            let left = db.add(None, "甲", "段落甲\r\n")?;
            let right = db.add(None, "乙", "段落乙")?;
            let markup = db.authored_embed_destination(origin, Some("#^shared"))?;
            db.append_synced_block(left, db.note(left)?.unwrap().version, &markup)?;
            db.append_synced_block(right, db.note(right)?.unwrap().version, &markup)?;
            let live = db.synchronized_blocks(left)?.0.remove(0);
            let stale = db.synchronized_blocks(right)?.0.remove(0);
            assert_eq!(live.block, "合成第一版");
            assert!(db.update_synced_block(&live, "合成第一版 ^shared\n\n重複 ^shared").is_err());
            db.update_synced_block(&live, "共用第二版\n續行")?;
            assert!(db.update_synced_block(&stale, "不能改寫").is_err());
            let content = db.note(origin)?.unwrap().body;
            assert_eq!(content, "前行\r\n\r\n共用第二版\r\n續行 ^shared\r\n\r\n後行\r\n");
            assert_eq!(db.synced_consumers(origin, &markup)?.0.iter().map(|s| s.note).collect::<Vec<_>>(), [left, right]);
            let current = db.note(right)?.unwrap();
            db.save(right, current.version, &current.row.title, &format!("{}\n\n另一處\n\n{markup}", current.body))?;
            let (placements, limited) = db.synchronized_blocks(right)?;
            assert!(!limited);
            assert_eq!(placements.iter().map(|hit| hit.line).collect::<Vec<_>>(), [3, 7]);
            db.detach_synced_block(&placements[1])?;
            let remaining = db.note(right)?.unwrap();
            assert!(remaining.body.contains(&format!("{markup}\n\n另一處\n\n共用第二版")));
            db.detach_synced_block(&db.synchronized_blocks(right)?.0.remove(0))?;
            let content = db.note(right)?.unwrap().body;
            assert!(content.contains("共用第二版") && !content.contains("paperroom:note:"));
            assert_eq!(db.synced_consumers(origin, &markup)?.0.len(), 1);
            let current = db.note(origin)?.unwrap();
            db.update_synced_block(&db.synchronized_blocks(left)?.0.remove(0), "[相對連結](other.md) 續行")?;
            assert!(db.detach_synced_block(&db.synchronized_blocks(left)?.0.remove(0)).is_err());
            let updated = db.note(origin)?.unwrap();
            db.save(origin, updated.version, &current.row.title, &current.body)?;
            db.trash(origin)?;
            assert!(db.synchronized_blocks(left)?.0.is_empty());
            assert!(db.detach_synced_block(&live).is_err());
            db.restore(origin)?;
            db.backup(&backup)?;
            (origin, left, right)
        };
        {
            let db = Notebook::open(&backup, false)?;
            let saved = db.synchronized_blocks(left)?.0.remove(0);
            assert!(saved.block.contains("續行"));
            assert_eq!(db.synchronized_source(&saved)?.0, origin);
            db.update_synced_block(&saved, "已還原可繼續")?;
            assert!(db.note(origin)?.unwrap().body.contains("已還原可繼續"));
            assert!(db.note(right)?.unwrap().body.contains("共用第二版"));
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
