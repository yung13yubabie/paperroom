//! A read-only block backlink audit for one imported Vault snapshot.
use super::{NATIVE_NOTE_PREFIX, NoteLinkPaths, Notebook, Result, mapped_link_target, native_note_target};
use crate::markdown::{self, block_anchors::{BlockLookup, resolve_blocks}};
use rusqlite::{OptionalExtension, params};

const MAX_LISTED: usize = 100;

struct Hit {
    source: i64,
    path: String,
    line: usize,
    label: String,
    block: String,
}

fn shown(value: &str) -> String {
    let mut chars = value.chars();
    let mut text = chars.by_ref().take(90).map(|c| if c.is_control() { ' ' } else { c }).collect::<String>();
    if chars.next().is_some() { text.push('…'); }
    text
}

impl Notebook {
    pub(crate) fn vault_incoming_block_report(&self, target: i64) -> Result<String> {
        let tx = self.connection.unchecked_transaction()?;
        let (root, target_path, target_body): (i64, String, String) = tx.query_row(
            "SELECT p.import_root,p.relative_path,n.body FROM vault_paths p
             JOIN notes n ON n.id=p.note_id JOIN notes r ON r.id=p.import_root
             WHERE n.id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL",
            [target], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?.ok_or("請先開啟這個 Vault 已匯入且仍使用中的目標筆記。")?;
        let files = {
            let mut stmt = tx.prepare(
                "SELECT p.note_id,p.relative_path FROM vault_paths p
                 JOIN notes n ON n.id=p.note_id WHERE p.import_root=?1 AND n.trash_root IS NULL",
            )?;
            stmt.query_map([root], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let index = NoteLinkPaths::new(&files);
        let mut hits = Vec::new();
        let (mut count, mut consumed) = (0, 0);
        {
            let mut stmt = tx.prepare(
                "SELECT p.note_id,p.relative_path,n.body FROM vault_paths p JOIN notes n ON n.id=p.note_id
                 WHERE p.import_root=?1 AND n.id<>?2 AND n.trash_root IS NULL
                   AND instr(n.body,'#')>0 ORDER BY p.relative_path,n.id",
            )?;
            let mut rows = stmt.query(params![root, target])?;
            while let Some(row) = rows.next()? {
                let source: i64 = row.get(0)?;
                let path = row.get_ref(1)?.as_str()?;
                let body = row.get_ref(2)?.as_str()?;
                consumed += body.len();
                if consumed > super::super::vault_search::MAX_SCAN {
                    return Err("這個 Vault 需查核的正文超過 128 MiB。".into());
                }
                let parts = path.split('/').map(str::to_owned).collect::<Vec<_>>();
                let (mut previous, mut line) = (0, 1);
                for reference in markdown::note_references(body) {
                    let Some((destination, anchor)) = reference.destination.split_once('#') else { continue; };
                    let Some(block) = anchor.strip_prefix('^') else { continue; };
                    if destination.is_empty() { continue; }
                    let id = if destination.starts_with(NATIVE_NOTE_PREFIX) {
                        native_note_target(&tx, Some(source), destination)?
                    } else {
                        mapped_link_target(&index, path, &parts, reference.syntax, &reference.destination, destination)
                    };
                    if id != Some(target) { continue; }
                    count += 1;
                    if hits.len() < MAX_LISTED {
                        let prefix = body.get(previous..reference.offset)
                            .ok_or("無法核對這筆區塊連結在原文的行號。")?;
                        line += prefix.matches(['\n', '\r']).count() - prefix.matches("\r\n").count();
                        previous = reference.offset;
                        hits.push(Hit { source, path: path.into(), line, label: reference.label, block: block.into() });
                    }
                }
            }
        }
        tx.commit()?;
        let ids = resolve_blocks(&target_body, hits.iter().map(|hit| hit.block.as_str()));
        let mut report = format!(
            "目標：{} (#{target})\r\n同批次其他匯入筆記有 {count} 個明確指向這篇筆記的區塊連結。",
            shown(&target_path),
        );
        for hit in &hits {
            let status = match ids.get(hit.block.as_str()) {
                Some(BlockLookup::Found { .. }) => "可定位",
                Some(BlockLookup::Ambiguous) => "區塊 ID 重複",
                Some(BlockLookup::Unsupported) => "區塊 ID 無效",
                _ => "找不到區塊 ID",
            };
            report.push_str(&format!(
                "\r\n{} (#{}) 第 {} 行　#^{}　{}　{}",
                shown(&hit.path), hit.source, hit.line, shown(&hit.block), status, shown(&hit.label),
            ));
        }
        if count > MAX_LISTED {
            report.push_str(&format!("\r\n顯示前 {MAX_LISTED} 個，另有 {} 個未列出。", count - MAX_LISTED));
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::ImportedPage;
    use std::path::Path;

    #[test]
    fn audit_reports_only_incoming_links_to_exact_id_and_real_source_lines() -> Result<()> {
        let page = |path: &str, body: &str| ImportedPage {
            parts: path.split('/').map(str::to_owned).collect(), title: "索引".into(), body: body.into(),
        };
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let root = db.import_vault("主批次", vec![
            page("甲/鏡頭.md", "正文 ^shot\r\n\r\n第一 ^dup\r\n\r\n第二 ^dup"),
            page("乙/鏡頭.md", "另一篇 ^shot"),
            page("來源/列表.md", concat!(
                "📽 前頁\r\n",
                "[[../甲/鏡頭#^shot|有效Wiki]]\r\n",
                "[遺失Markdown](../甲/鏡頭.md#^gone)\r\n",
                "[[甲/鏡頭#^dup|重複引用]]\r\n",
                "[[鏡頭#^shot|同名不可靠]] [[乙/鏡頭#^shot|別篇]] `[[甲/鏡頭#^shot|程式碼]]`",
            )),
        ])?;
        db.import_vault("外批次", vec![page("甲/鏡頭.md", "外批次 ^shot"),
            page("外批次入口.md", "[[甲/鏡頭#^shot|外批次]]")])?;
        let lookup = |path: &str| db.connection.query_row(
            "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
            (root, path), |r| r.get::<_, i64>(0),
        );
        let target = lookup("甲/鏡頭.md")?;
        let source = lookup("來源/列表.md")?;
        let unchanged = db.note(source)?.unwrap();
        let count = db.connection.total_changes();
        let report = db.vault_incoming_block_report(target)?;
        assert!(report.contains("有 3 個明確指向"), "{report}");
        assert!(report.contains(&format!("來源/列表.md (#{source}) 第 2 行　#^shot　可定位　有效Wiki")), "{report}");
        assert!(report.contains("第 3 行　#^gone　找不到區塊 ID　遺失Markdown"), "{report}");
        assert!(report.contains("第 4 行　#^dup　區塊 ID 重複　重複引用"), "{report}");
        for excluded in ["同名不可靠", "外批次", "別篇", "程式碼"] {
            assert!(!report.contains(excluded), "{report}");
        }
        assert_eq!(db.connection.total_changes(), count);
        assert_eq!(db.note(source)?.unwrap().body, unchanged.body);
        db.trash(source)?;
        assert!(db.vault_incoming_block_report(target)?.contains("有 0 個明確指向"));
        Ok(())
    }
}
