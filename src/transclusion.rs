use super::note_links::{NATIVE_NOTE_PREFIX, native_note_target};
use super::vault_search::frontmatter;
use super::{Notebook, Result};
use crate::markdown::block_anchors::{BlockLookup, resolve_block};
use crate::markdown::{AnchorSyntax, HeadingLookup, resolve_heading};
use crate::vault::WikiIndex;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::ops::Range;

const MAX_DEPTH: usize = 4;
const MAX_PLACEMENTS: usize = 128;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXCERPT_BYTES: usize = 512 * 1024;
const MAX_PROJECTED_MARKS: usize = 128;

pub(crate) struct EmbedReference {
    pub range: Range<usize>,
    pub destination: String,
}

/// Return authored placements with offsets into the unchanged Markdown.
pub(crate) fn embed_references(body: &str) -> impl Iterator<Item = EmbedReference> + '_ {
    let (metadata, prose) = frontmatter(body);
    let without_bom = body.strip_prefix('\u{feff}').unwrap_or(body);
    let unclosed = metadata.is_none() && prose == without_bom
        && (prose.starts_with("---\n") || prose.starts_with("---\r\n"));
    let offset = body.len() - prose.len();
    let mut hidden = 0usize;
    let text = if unclosed || !prose.contains("![[") { "" } else { prose };
    let options = Options::ENABLE_WIKILINKS | Options::ENABLE_GFM | Options::ENABLE_TABLES;
    Parser::new_ext(text, options).into_offset_iter().filter_map(move |(event, range)| {
        match event {
            Event::Start(Tag::Image { link_type, dest_url, .. }) => {
                let outer = hidden == 0;
                hidden += 1;
                (outer && matches!(link_type, LinkType::WikiLink { .. })
                    && prose[range.clone()].starts_with("![["))
                .then(|| EmbedReference {
                    range: range.start + offset..range.end + offset,
                    destination: dest_url.trim().to_owned(),
                })
            }
            Event::Start(Tag::Link { .. } | Tag::CodeBlock(_) | Tag::HtmlBlock) => {
                hidden += 1;
                None
            }
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock | TagEnd::HtmlBlock) => {
                hidden -= 1;
                None
            }
            _ => None,
        }
    })
}

fn destination_parts(destination: &str) -> (&str, Option<&str>) {
    destination.find('#').map_or((destination, None), |i| (&destination[..i], Some(&destination[i..])))
}

/// New transclusions require an identity token even though older note links
/// accept bare numeric IDs. A number alone may name another workspace's note.
pub(crate) fn native_embed_target(db: &Connection, path: &str) -> Result<Option<i64>> {
    let token_present = path.strip_prefix(NATIVE_NOTE_PREFIX).is_some_and(|suffix| suffix.contains(':'));
    if token_present {
        return native_note_target(db, path);
    }
    Ok(None)
}

pub(super) fn resolve_embed_target(
    db: &Connection,
    host: i64,
    vault: Option<(&str, &WikiIndex<'_>)>,
    destination: &str,
) -> Result<Option<i64>> {
    if destination.is_empty() || destination.len() > 4096 || destination.chars().any(char::is_control) {
        return Ok(None);
    }
    let (path, anchor) = destination_parts(destination);
    if path.is_empty() {
        return Ok(anchor.map(|_| host));
    }
    if path.starts_with(NATIVE_NOTE_PREFIX) {
        return native_embed_target(db, path);
    }
    Ok(vault.and_then(|(source, index)| index.resolve(source, path)))
}

struct SavedNote {
    title: String,
    body: String,
    #[cfg(test)]
    token: String,
    location: Option<(i64, String)>,
}

fn read_active_note(db: &Connection, id: i64) -> Result<Option<SavedNote>> {
    Ok(db.query_row(
        "SELECT n.title,n.body,i.token,p.import_root,p.relative_path
         FROM notes n JOIN note_link_identities i ON i.note_id=n.id
         LEFT JOIN vault_paths p ON p.note_id=n.id LEFT JOIN notes r ON r.id=p.import_root
         WHERE n.id=?1 AND n.trash_root IS NULL AND r.trash_root IS NULL",
        [id],
        |row| {
            let root: Option<i64> = row.get(3)?;
            let location = root.map(|root| row.get(4).map(|path| (root, path))).transpose()?;
            Ok(SavedNote {
                title: row.get(0)?, body: row.get(1)?,
                #[cfg(test)]
                token: row.get(2)?,
                location,
            })
        },
    ).optional()?)
}

#[derive(Debug)]
enum Unavailable {
    Source,
    Missing,
    Ambiguous,
    Unsupported,
    Empty,
    Cycle,
    Limit,
}

impl Unavailable {
    fn explanation(self) -> &'static str {
        match self {
            Self::Source => "找不到來源或名稱不唯一",
            Self::Missing => "找不到指定區塊或標題",
            Self::Ambiguous => "指定區塊或標題有多個符合項目",
            Self::Unsupported => "無法辨識區塊或標題的寫法",
            Self::Empty => "來源沒有可顯示的正文",
            Self::Cycle => "偵測到循環引用",
            Self::Limit => "超出嵌入預覽上限",
        }
    }
}

fn prose_range(body: &str) -> Range<usize> {
    body.len() - frontmatter(body).1.len()..body.len()
}

fn select_excerpt(body: &str, anchor: Option<&str>) -> std::result::Result<Range<usize>, Unavailable> {
    let section = match anchor {
        None => prose_range(body),
        Some(anchor) if anchor.starts_with("#^") => match resolve_block(body, anchor) {
            BlockLookup::Found { source, .. } => source,
            BlockLookup::Missing => return Err(Unavailable::Missing),
            BlockLookup::Ambiguous => return Err(Unavailable::Ambiguous),
            BlockLookup::Unsupported => return Err(Unavailable::Unsupported),
        },
        Some(anchor) => match resolve_heading(body, anchor, AnchorSyntax::Wiki) {
            HeadingLookup::Found { section, .. } => section,
            HeadingLookup::Missing => return Err(Unavailable::Missing),
            HeadingLookup::Ambiguous => return Err(Unavailable::Ambiguous),
            HeadingLookup::Unsupported => return Err(Unavailable::Unsupported),
        },
    };
    if body[section.clone()].trim().is_empty() {
        return Err(Unavailable::Empty);
    }
    Ok(section)
}

fn preview_label(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars();
    for c in chars.by_ref().take(180) {
        if c.is_control() {
            output.push(' ');
        } else {
            if c.is_ascii_punctuation() {
                output.push('\\');
            }
            output.push(c);
        }
    }
    if chars.next().is_some() {
        output.push('…');
    }
    output
}

fn valid_mark(body: &str, mark: &Range<usize>) -> bool {
    mark.start < mark.end && body.is_char_boundary(mark.start) && body.is_char_boundary(mark.end)
}

fn append_literal(
    body: &str,
    source: Range<usize>,
    projected: &mut String,
    marks: &mut &[Range<usize>],
    projected_marks: &mut Vec<Range<usize>>,
) {
    let offset = projected.len();
    projected.push_str(&body[source.clone()]);
    while let Some((mark, remaining)) = marks.split_first() {
        if mark.start >= source.end {
            break;
        }
        *marks = remaining;
        if mark.start >= source.start && mark.end <= source.end && valid_mark(body, mark) {
            let start = offset + (mark.start - source.start);
            projected_marks.push(start..start + mark.len());
        }
    }
}

struct Projection<'a> {
    db: &'a Connection,
    vaults: HashMap<i64, Vec<(i64, String)>>,
    remaining_placements: usize,
    remaining_source: usize,
    remaining_excerpt: usize,
    stack: Vec<(i64, Range<usize>)>,
}

impl Projection<'_> {
    fn target(&mut self, host: i64, note: &SavedNote, destination: &str) -> Result<Option<i64>> {
        let (path, _) = destination_parts(destination);
        if path.is_empty() || path.starts_with(NATIVE_NOTE_PREFIX) {
            return resolve_embed_target(self.db, host, None, destination);
        }
        let Some((root, source)) = &note.location else { return Ok(None); };
        if let std::collections::hash_map::Entry::Vacant(entry) = self.vaults.entry(*root) {
            let mut query = self.db.prepare(
                "SELECT note_id,relative_path FROM vault_paths WHERE import_root=?1",
            )?;
            entry.insert(query.query_map([root], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?);
        }
        let index = WikiIndex::new(&self.vaults[root]);
        resolve_embed_target(self.db, host, Some((source, &index)), destination)
    }

    fn expansion(&mut self, host: i64, note: &SavedNote, destination: &str)
        -> Result<std::result::Result<String, Unavailable>>
    {
        if self.stack.len() > MAX_DEPTH || self.remaining_source == 0 {
            return Ok(Err(Unavailable::Limit));
        }
        let Some(target) = self.target(host, note, destination)? else {
            return Ok(Err(Unavailable::Source));
        };
        let saved;
        let target_note = if target == host {
            note
        } else {
            let Some(active) = read_active_note(self.db, target)? else {
                return Ok(Err(Unavailable::Source));
            };
            saved = active;
            &saved
        };
        let Some(remaining) = self.remaining_source.checked_sub(target_note.body.len()) else {
            self.remaining_source = 0;
            return Ok(Err(Unavailable::Limit));
        };
        self.remaining_source = remaining;
        let (_, anchor) = destination_parts(destination);
        let section = match select_excerpt(&target_note.body, anchor) {
            Ok(section) => section,
            Err(reason) => return Ok(Err(reason)),
        };
        if self.stack.contains(&(target, section.clone())) {
            return Ok(Err(Unavailable::Cycle));
        }
        let Some(remaining) = self.remaining_excerpt.checked_sub(section.len()) else {
            return Ok(Err(Unavailable::Limit));
        };
        self.remaining_excerpt = remaining;
        self.stack.push((target, section.clone()));
        let rendered = self.render(target, target_note, &target_note.body[section], &[]);
        self.stack.pop();
        let label = target_note.location.as_ref().map_or(target_note.title.as_str(), |(_, path)| path.as_str());
        let label = preview_label(&format!("{label}{}", anchor.unwrap_or("")));
        let mut output = format!("\n\n> **嵌入來源：{label}**\n>\n");
        // Keep an excerpt's Markdown delimiters inside its own block. An
        // unfinished fence in an embedded note cannot turn host prose into code.
        for line in rendered?.0.trim_end().lines() {
            output.push_str("> ");
            output.push_str(line);
            output.push('\n');
        }
        output.push('\n');
        Ok(Ok(output))
    }

    fn render(
        &mut self, host: i64, note: &SavedNote, body: &str, mut marks: &[Range<usize>],
    ) -> Result<(String, Vec<Range<usize>>)> {
        let mut projected = String::with_capacity(body.len());
        let mut projected_marks = Vec::new();
        let mut previous = 0;
        for reference in embed_references(body) {
            append_literal(body, previous..reference.range.start, &mut projected, &mut marks, &mut projected_marks);
            if self.remaining_placements == 0 {
                projected.push_str("\n\n**已達嵌入預覽上限。**\n\n");
                previous = reference.range.start;
                break;
            }
            self.remaining_placements -= 1;
            match self.expansion(host, note, &reference.destination)? {
                Ok(embed) => projected.push_str(&embed),
                Err(reason) => projected.push_str(&format!("\n\n**嵌入無法顯示（{}）：{}**\n\n",
                    reason.explanation(), preview_label(&reference.destination))),
            }
            previous = reference.range.end;
        }
        append_literal(body, previous..body.len(), &mut projected, &mut marks, &mut projected_marks);
        Ok((projected, projected_marks))
    }
}

impl Notebook {
    pub(crate) fn project_embeds(&self, host: i64, saved_body: &str) -> Result<String> {
        Ok(self.project_embeds_marked(host, saved_body, &[])?.0)
    }

    /// Translate sorted byte ranges from the saved host body into the preview.
    /// Only ranges wholly inside one literal host segment remain eligible.
    pub(crate) fn project_embeds_marked(
        &self, host: i64, saved_body: &str, marks: &[Range<usize>],
    ) -> Result<(String, Vec<Range<usize>>)> {
        let marks = &marks[..marks.len().min(MAX_PROJECTED_MARKS)];
        let literal_marks = || marks.iter().filter(|mark| valid_mark(saved_body, mark)).cloned().collect();
        if embed_references(saved_body).next().is_none() {
            return Ok((saved_body.to_owned(), literal_marks()));
        }
        let snapshot = self.connection.unchecked_transaction()?;
        let Some(saved) = read_active_note(&snapshot, host)?.filter(|saved| saved.body == saved_body) else {
            return Ok((format!("{saved_body}\n\n**請重新開啟這篇筆記以載入嵌入。**"), literal_marks()));
        };
        let mut projection = Projection {
            db: &snapshot,
            vaults: HashMap::new(),
            remaining_placements: MAX_PLACEMENTS,
            remaining_source: MAX_SOURCE_BYTES,
            remaining_excerpt: MAX_EXCERPT_BYTES,
            stack: vec![(host, prose_range(saved_body))],
        };
        let result = projection.render(host, &saved, saved_body, marks)?;
        snapshot.commit()?;
        Ok(result)
    }

    #[cfg(test)]
    pub(crate) fn authored_embed_destination(&self, target: i64, anchor: Option<&str>) -> Result<String> {
        let note = read_active_note(&self.connection, target)?.ok_or("找不到使用中的嵌入來源。")?;
        if anchor.is_some_and(|anchor| !anchor.starts_with('#') || anchor.len() > 4000
            || anchor.contains(['[', ']', '|', '\\']) || anchor.chars().any(char::is_control))
        {
            return Err("此區塊或標題的寫法無法用在嵌入。".into());
        }
        select_excerpt(&note.body, anchor).map_err(Unavailable::explanation)?;
        Ok(format!("![[{NATIVE_NOTE_PREFIX}{target}:{}{}]]", note.token, anchor.unwrap_or("")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn marked_projection_retains_only_literal_host_ranges_in_order() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let source = db.add(None, "來源", "與主筆記重複的末段")?;
        let link = db.authored_embed_destination(source, None)?;
        let body = format!("首段\n\n{link}\n\n末段");
        let host = db.add(None, "主筆記", &body)?;
        let first = 0.."首段".len();
        let embed_start = body.find(&link).unwrap();
        let last = body.rfind("末段").unwrap()..body.len();
        let marks = [
            first.clone(),
            0..last.end,
            1..3,
            embed_start..embed_start + link.len(),
            last.clone(),
            last.end..last.start,
            usize::MAX..usize::MAX,
        ];
        let (projected, mapped) = db.project_embeds_marked(host, &body, &marks)?;
        assert_eq!(projected, db.project_embeds(host, &body)?);
        assert_eq!(mapped, [first.clone(), projected.rfind("末段").unwrap()..projected.len()]);
        assert!(mapped[1].start > last.start);

        let note = db.note(host)?.unwrap();
        db.save(host, note.version, &note.row.title, "已儲存新版")?;
        let (stale, mapped) = db.project_embeds_marked(host, &body, &[first.clone(), last.clone()])?;
        assert_eq!(stale, db.project_embeds(host, &body)?);
        assert!(stale.contains("請重新開啟這篇筆記"));
        assert_eq!(mapped, [first, last]);
        Ok(())
    }

    #[test]
    fn vault_live_embeds_render_unique_fragments_and_exact_paths_after_reopen_and_backup() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let vault = dir.join("Vault");
        let db_path = dir.join("notes.sqlite");
        let backup = dir.join("backup.sqlite");
        for folder in ["甲", "乙", "現場"] {
            fs::create_dir_all(vault.join(folder))?;
        }
        let target_body = "---\r\nsecret: 不應讀入 YAML\r\n---\r\n# 前半部\r\n\r\n不同前文\r\n\r\n# 成果\r\n\r\n🎞️ 第一行\r\n正確區塊 ^clip\r\n\r\n章節中的另一段\r\n\r\n# 後半部\r\n\r\n尾端全文\r\n\r\n甲 ^dup\r\n\r\n乙 ^dup\r\n\r\n";
        fs::write(vault.join("乙/鏡頭.md"), target_body)?;
        fs::write(vault.join("甲/鏡頭.md"), "不應誤讀另一篇 ^clip")?;
        fs::write(vault.join("甲/外來.md"), "外來資料只在此處")?;
        let body = concat!(
            "\u{feff}---\r\nexample: ![[甲/外來]]\r\n---\r\n\r\n",
            "前文\r\n\r\n![[../乙/鏡頭.md#^clip]]\r\n\r\n後文\r\n\r\n",
            "![[鏡頭#^clip]]\r\n\r\n![[乙/鏡頭#^dup]]\r\n\r\n",
            "`![[甲/外來]]`\r\n\r\n```md\r\n![[甲/外來]]\r\n```\r\n",
            "\r\n\\![[甲/外來]]\r\n",
        );
        fs::write(vault.join("現場/入口.md"), body)?;
        fs::write(vault.join("現場/標題.md"), "![[乙/鏡頭#成果]]")?;
        fs::write(vault.join("現場/全文.md"), "![[乙/鏡頭]]")?;

        let mut db = Notebook::open(&db_path, false)?;
        let root = db.import_vault("第一個 Vault", crate::vault::read(&vault)?.pages)?;
        db.import_vault("第二個 Vault", crate::vault::read(&vault)?.pages)?;
        let find = |path: &str| -> Result<i64> {
            Ok(db.connection.query_row(
                "SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path=?2",
                (root, path),
                |row| row.get(0),
            )?)
        };
        let host = find("現場/入口.md")?;
        let heading = find("現場/標題.md")?;
        let whole = find("現場/全文.md")?;
        let target = find("乙/鏡頭.md")?;
        assert_eq!(embed_references(body).count(), 3);
        let projected = db.project_embeds(host, body)?;
        assert!(projected.contains("正確區塊") && projected.contains("前文") && projected.contains("後文"), "{projected}");
        assert!(projected.contains("找不到來源或名稱不唯一") && projected.contains("指定區塊或標題有多個符合項目"));
        for absent in ["不應誤讀另一篇", "外來資料只在此處", "不同前文", "章節中的另一段", "尾端全文", "^clip\n>"] {
            assert!(!projected.contains(absent), "{absent}");
        }
        let read = |id| -> Result<String> { db.project_embeds(id, &db.note(id)?.unwrap().body) };
        let excerpt = read(heading)?;
        assert!(excerpt.contains("章節中的另一段") && !excerpt.contains("尾端全文") && !excerpt.contains("不同前文"));
        let excerpt = read(whole)?;
        assert!(excerpt.contains("尾端全文") && excerpt.contains("不同前文") && !excerpt.contains("不應讀入 YAML"));
        let (_, incoming) = db.graph_neighbors(&db.note(target)?.unwrap())?;
        assert_eq!(incoming.iter().map(|it| it.0).collect::<std::collections::BTreeSet<_>>(),
            [host, heading, whole].into_iter().collect());

        let other = Notebook::open(&db_path, false)?;
        let old = other.note(target)?.unwrap();
        other.save(target, old.version, &old.row.title, &old.body.replace("正確區塊", "更新可見"))?;
        assert!(db.project_embeds(host, body)?.contains("更新可見"));
        db.backup(&backup)?;
        drop(other);
        drop(db);
        let archived = dir.join("renamed-vault");
        fs::rename(&vault, &archived)?;
        let mut db = Notebook::open(&db_path, false)?;
        assert!(db.project_embeds(host, body)?.contains("更新可見"));
        db.trash(target)?;
        assert!(!db.project_embeds(host, body)?.contains("更新可見"));
        assert!(!db.project_embeds(host, body)?.contains("不應誤讀另一篇"));
        db.restore(target)?;
        assert!(db.project_embeds(host, body)?.contains("更新可見"));
        let backup = Notebook::open(&backup, false)?;
        assert!(backup.project_embeds(host, body)?.contains("更新可見"));
        assert_eq!(backup.note(host)?.unwrap().body, body);
        assert_eq!(fs::read(archived.join("乙/鏡頭.md"))?, target_body.as_bytes());
        assert_eq!(fs::read(archived.join("現場/入口.md"))?, body.as_bytes());
        drop(backup);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn scoped_live_embeds_check_identity_cycles_stale_hosts_and_bounds() -> Result<()> {
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        let source = db.add(None, "來源", "字串 **內容** ^selected\n\n另一段")?;
        let link = db.authored_embed_destination(source, Some("#^selected"))?;
        assert!(link.starts_with(&format!("![[paperroom:note:{source}:")));
        assert!(db.authored_embed_destination(source, Some("#^missing")).is_err());
        let body = format!("{link}\n\n![[paperroom:note:{source}#^selected]]");
        let host = db.add(None, "讀取", &body)?;
        let preview = db.project_embeds(host, &body)?;
        assert!(preview.contains("字串 **內容**") && preview.contains("找不到來源或名稱不唯一"));
        assert!(!preview.contains("另一段"));

        let mut separate = Notebook::open(Path::new(":memory:"), false)?;
        let unrelated = separate.add(None, "不同的來源", "不能透過數字 ID 讀到 ^selected")?;
        assert_eq!(unrelated, source);
        let foreign_host = separate.add(None, "工作區", &link)?;
        let preview = separate.project_embeds(foreign_host, &link)?;
        assert!(!preview.contains("不能透過數字 ID 讀到"));

        let live = db.note(host)?.unwrap();
        db.save(host, live.version, &live.row.title, &link)?;
        assert!(db.project_embeds(host, &body)?.contains("請重新開啟這篇筆記"));
        let source_note = db.note(source)?.unwrap();
        let recursion = db.authored_embed_destination(host, None)?;
        db.save(source, source_note.version, &source_note.row.title, &format!("{recursion} ^selected"))?;
        assert!(db.project_embeds(host, &link)?.contains("偵測到循環引用"));

        let long = "a".repeat(MAX_EXCERPT_BYTES + 1);
        let big = db.add(None, "大型筆記", &long)?;
        let big = db.authored_embed_destination(big, None)?;
        let crowded = format!("{big}\n\n{}", "![[缺少的筆記]]\n".repeat(MAX_PLACEMENTS + 2));
        let crowded_host = db.add(None, "超大預覽", &crowded)?;
        let preview = db.project_embeds(crowded_host, &crowded)?;
        assert!(preview.contains("超出嵌入預覽上限"));
        assert!(preview.contains("已達嵌入預覽上限"));
        assert!(preview.len() < MAX_EXCERPT_BYTES);
        Ok(())
    }
}
