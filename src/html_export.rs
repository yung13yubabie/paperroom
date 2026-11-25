use super::attachments::{MAX_FILE_BYTES, MAX_STORAGE_BYTES, file_kind, has_expected_signature};
use super::note_links::{NATIVE_NOTE_PREFIX, native_note_target};
use super::{MAX_NOTE_BYTES, Notebook, Result};
use crate::output::{publish_new, windows_output_name};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const MAX_PAGES: usize = 10_000;
const MAX_MARKDOWN_BYTES: usize = 64 * 1024 * 1024;

struct Page {
    parent: Option<i64>,
    title: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct HtmlExport {
    pub pages: usize,
    pub attachments: usize,
}

impl Notebook {
    pub(crate) fn export_html_workspace(&self, destination: &Path) -> Result<HtmlExport> {
        let snapshot = self.connection.unchecked_transaction()?;
        let mut pages = BTreeMap::new();
        let mut children = HashMap::<Option<i64>, Vec<i64>>::new();
        let mut raw_bytes = 0;
        {
            let mut query = snapshot.prepare(
                "SELECT id,parent,title,length(CAST(body AS BLOB)) FROM notes
                 WHERE trash_root IS NULL ORDER BY id",
            )?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let parent: Option<i64> = row.get(1)?;
                let bytes = usize::try_from(row.get::<_, i64>(3)?)?;
                if id <= 0 || bytes > MAX_NOTE_BYTES || bytes > MAX_MARKDOWN_BYTES - raw_bytes
                    || pages.len() == MAX_PAGES
                {
                    return Err("HTML 匯出最多 10,000 篇筆記及 64 MiB Markdown 內文。".into());
                }
                raw_bytes += bytes;
                pages.insert(id, Page { parent, title: row.get(2)? });
            }
        }
        if pages.is_empty() { return Err("工作區沒有可匯出的筆記。".into()); }
        for (id, page) in &pages {
            let parent = page.parent.filter(|parent| pages.contains_key(parent));
            children.entry(parent).or_default().push(*id);
        }

        let mut counts = HtmlExport { pages: pages.len(), attachments: 0 };
        let mut attachment_bytes = 0;
        publish_new(destination, |file, _| {
            let mut zip = ZipWriter::new(file);
            zip.start_file("index.html", SimpleFileOptions::default())?;
            start_document(&mut zip, "紙間工作區")?;
            zip.write_all("<h1>紙間工作區</h1><p>先解壓整個 ZIP，再從此頁開啟筆記。每篇頁面可另開 Markdown 原文。</p>".as_bytes())?;
            page_list(&mut zip, &pages, children.get(&None), "pages/", "最上層筆記")?;
            write!(zip, "<p>共 {} 篇使用中的筆記。</p></main></body></html>", counts.pages)?;

            let mut lookup = snapshot.prepare_cached("SELECT body FROM notes WHERE id=?1")?;
            let mut assets = snapshot.prepare_cached(
                "SELECT id,name,kind,content FROM attachments WHERE note_id=?1 ORDER BY id",
            )?;
            for (&id, page) in &pages {
                let body: String = lookup.query_row([id], |row| row.get(0))?;
                let mut links = Vec::new();
                let mut rows = assets.query([id])?;
                while let Some(row) = rows.next()? {
                    let asset: i64 = row.get(0)?;
                    let name = row.get_ref(1)?.as_str()?;
                    let kind = row.get_ref(2)?.as_str()?;
                    let bytes = row.get_ref(3)?.as_blob()?;
                    let extension = match kind {
                        "pdf" => "pdf", "png" => "png", "jpeg" => "jpg",
                        "text" => "txt", "markdown" => "md",
                        _ => return Err("工作區內含無法安全匯出的附件格式。".into()),
                    };
                    if asset <= 0 || bytes.len() > MAX_FILE_BYTES || !has_expected_signature(kind, bytes) {
                        return Err("有附件的內容與類型不符，HTML 匯出已停止。".into());
                    }
                    attachment_bytes += bytes.len();
                    if attachment_bytes > MAX_STORAGE_BYTES as usize {
                        return Err("附件總量超過 HTML 匯出的 128 MiB 上限。".into());
                    }
                    let path = format!("files/{asset}.{extension}");
                    zip.start_file(&path, SimpleFileOptions::default())?;
                    zip.write_all(bytes)?;
                    let original_name = if windows_output_name(OsStr::new(name)).is_ok()
                        && file_kind(Path::new(name)).is_ok_and(|expected| expected == kind)
                    {
                        Some(name.to_owned())
                    } else {
                        None
                    };
                    links.push((path, name.to_owned(), original_name));
                    counts.attachments += 1;
                }
                zip.start_file(format!("markdown/{id}.md"), SimpleFileOptions::default())?;
                zip.write_all(body.as_bytes())?;
                zip.start_file(format!("pages/{id}.html"), SimpleFileOptions::default())?;
                start_document(&mut zip, &page.title)?;
                zip.write_all("<nav><a href=\"../index.html\">工作區索引</a>".as_bytes())?;
                if let Some(parent) = page.parent.filter(|parent| pages.contains_key(parent)) {
                    write!(zip, " · <a href=\"{parent}.html\">上層筆記</a>")?;
                }
                write!(zip, " · <a href=\"../markdown/{id}.md\" download>Markdown 原文</a></nav><h1>")?;
                escape(&mut zip, &page.title)?;
                zip.write_all(b"</h1><article>")?;
                let marks = super::note_highlights::read_marks(&snapshot, id, &body)?;
                render_markdown(&mut zip, &snapshot, &pages, &body, &marks)?;
                zip.write_all(b"</article>")?;
                page_list(&mut zip, &pages, children.get(&Some(id)), "", "子筆記")?;
                if !links.is_empty() {
                    zip.write_all("<h2>附件</h2><ul>".as_bytes())?;
                    for (path, name, download_name) in links {
                        write!(zip, "<li><a href=\"../{path}\" download")?;
                        if let Some(download_name) = download_name {
                            zip.write_all(b"=\"")?;
                            escape(&mut zip, &download_name)?;
                            zip.write_all(b"\"")?;
                        }
                        zip.write_all(b">")?;
                        escape(&mut zip, &name)?;
                        zip.write_all(b"</a></li>")?;
                    }
                    zip.write_all(b"</ul>")?;
                }
                zip.write_all(b"</main></body></html>")?;
            }
            zip.finish()?;
            drop(lookup);
            drop(assets);
            snapshot.commit()?;
            Ok(())
        })?;
        Ok(counts)
    }
}

fn escape(output: &mut impl Write, value: &str) -> std::io::Result<()> {
    let mut start = 0;
    for (offset, c) in value.char_indices() {
        let escaped = match c {
            '&' => "&amp;", '<' => "&lt;", '>' => "&gt;", '"' => "&quot;", _ => continue,
        };
        output.write_all(value[start..offset].as_bytes())?;
        output.write_all(escaped.as_bytes())?;
        start = offset + 1;
    }
    output.write_all(value[start..].as_bytes())
}

fn start_document(output: &mut impl Write, title: &str) -> std::io::Result<()> {
    output.write_all(concat!(
        "<!doctype html><html lang=\"zh-Hant\"><head><meta charset=\"utf-8\">",
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; form-action 'none'; base-uri 'none'\">",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>",
    ).as_bytes())?;
    escape(output, title)?;
    output.write_all(concat!(
        "</title><style>body{color:#172b26;background:#f9f8f2;font:1rem/1.65 system-ui,sans-serif}",
        "main{max-width:72ch;margin:2rem auto;padding:0 1rem;overflow-wrap:anywhere}a{color:#13624e}",
        "blockquote{margin-inline:0;border-inline-start:3px solid #457b66;padding-inline:1rem}",
        "pre{white-space:pre-wrap}td,th{padding:.3rem;border-bottom:1px solid #aaa}",
        "</style></head><body><main>",
    ).as_bytes())
}

fn page_list(
    output: &mut impl Write, pages: &BTreeMap<i64, Page>, ids: Option<&Vec<i64>>,
    prefix: &str, title: &str,
) -> std::io::Result<()> {
    let Some(ids) = ids else { return Ok(()) };
    write!(output, "<h2>{title}</h2><ul>")?;
    for id in ids {
        write!(output, "<li><a href=\"{prefix}{id}.html\">")?;
        escape(output, &pages[id].title)?;
        output.write_all(b"</a></li>")?;
    }
    output.write_all(b"</ul>")
}

fn render_markdown(
    output: &mut impl Write, db: &Connection, pages: &BTreeMap<i64, Page>, body: &str,
    marks: &[std::ops::Range<usize>],
) -> Result<()> {
    let options = Options::ENABLE_GFM | Options::ENABLE_WIKILINKS |
        Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let mut links = Vec::new();
    for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => write!(output, "<{level}>")?,
            Event::End(TagEnd::Heading(level)) => write!(output, "</{level}>")?,
            Event::Start(Tag::List(Some(start))) => write!(output, "<ol start=\"{start}\">")?,
            Event::End(TagEnd::List(ordered)) => output.write_all(if ordered { b"</ol>" } else { b"</ul>" })?,
            Event::Start(Tag::Link { dest_url, .. }) => {
                // Only app-issued scoped note identities become hyperlinks.
                // Preserve every original destination in the companion Markdown.
                let scoped = !dest_url.contains('#') && dest_url.strip_prefix(NATIVE_NOTE_PREFIX)
                    .is_some_and(|id| id.contains(':'));
                let target = if scoped { native_note_target(db, &dest_url)? } else { None };
                let target = target.filter(|id| pages.contains_key(id));
                if let Some(id) = target { write!(output, "<a href=\"{id}.html\">")?; }
                links.push(target.is_some());
            }
            Event::End(TagEnd::Link) => {
                if links.pop() == Some(true) { output.write_all(b"</a>")?; }
            }
            Event::Start(Tag::Paragraph) => output.write_all(b"<p>")?,
            Event::End(TagEnd::Paragraph) => output.write_all(b"</p>")?,
            Event::Start(Tag::List(None)) => output.write_all(b"<ul>")?,
            Event::Start(Tag::Item) => output.write_all(b"<li>")?,
            Event::End(TagEnd::Item) => output.write_all(b"</li>")?,
            Event::Start(Tag::BlockQuote(_)) => output.write_all(b"<blockquote>")?,
            Event::End(TagEnd::BlockQuote(_)) => output.write_all(b"</blockquote>")?,
            Event::Start(Tag::CodeBlock(_)) => output.write_all(b"<pre><code>")?,
            Event::End(TagEnd::CodeBlock) => output.write_all(b"</code></pre>")?,
            Event::Start(Tag::Emphasis) => output.write_all(b"<em>")?,
            Event::End(TagEnd::Emphasis) => output.write_all(b"</em>")?,
            Event::Start(Tag::Strong) => output.write_all(b"<strong>")?,
            Event::End(TagEnd::Strong) => output.write_all(b"</strong>")?,
            Event::Start(Tag::Strikethrough) => output.write_all(b"<s>")?,
            Event::End(TagEnd::Strikethrough) => output.write_all(b"</s>")?,
            Event::Start(Tag::Table(_)) => output.write_all(b"<table>")?,
            Event::End(TagEnd::Table) => output.write_all(b"</table>")?,
            Event::Start(Tag::TableHead | Tag::TableRow) => output.write_all(b"<tr>")?,
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => output.write_all(b"</tr>")?,
            Event::Start(Tag::TableCell) => output.write_all(b"<td>")?,
            Event::End(TagEnd::TableCell) => output.write_all(b"</td>")?,
            Event::Text(text) => {
                if let Some(parts) = crate::markdown::highlight_parts(body, &text, range, marks) {
                    for (highlighted, part) in parts {
                        if highlighted { output.write_all(b"<mark>")?; }
                        escape(output, part)?;
                        if highlighted { output.write_all(b"</mark>")?; }
                    }
                } else {
                    escape(output, &text)?;
                }
            }
            Event::Html(text) | Event::InlineHtml(text) => escape(output, &text)?,
            Event::Code(text) => {
                output.write_all(b"<code>")?;
                escape(output, &text)?;
                output.write_all(b"</code>")?;
            }
            Event::HardBreak => output.write_all(b"<br>")?,
            Event::SoftBreak => output.write_all(b"\n")?,
            Event::Rule => output.write_all(b"<hr>")?,
            Event::TaskListMarker(done) => output.write_all(if done { "☑ ".as_bytes() } else { "☐ ".as_bytes() })?,
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Read;

    #[test]
    fn html_workspace_preserves_hierarchy_and_payload_without_live_external_markup() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let db_file = dir.join("notes.sqlite3");
        let output = dir.join("離線網頁.zip");
        let raw_body = "\u{feff}# 研究 <鏡頭>\r\n\r\n本機段落 **文字**\r\n\r\n![遠端](https://example.invalid/x.jpg)\r\n\r\n<script>literal</script>\r\n";
        let asset = "引用原文\r\n";
        let (parent, child, trash, attachment) = {
            let mut db = Notebook::open(&db_file, false)?;
            let parent = db.add(None, "同名 <鏡頭>", raw_body)?;
            let token: String = db.connection.query_row(
                "SELECT token FROM note_link_identities WHERE note_id=?1", [parent], |row| row.get(0),
            )?;
            let link = format!("{NATIVE_NOTE_PREFIX}{parent}:{token}");
            let child_body = format!("繁中子稿\n\n[本機原文]({link}) · [僅保留深連結文字]({link}#section)");
            let child = db.add(Some(parent), "同名 <鏡頭>", &child_body)?;
            let trash = db.add(None, "不可匯出", "垃圾桶內文")?;
            db.trash(trash)?;
            db.connection.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,'text',?3)",
                (child, "附檔 & 引用.txt", asset.as_bytes()))?;
            (parent, child, trash, db.connection.last_insert_rowid())
        };
        let db = Notebook::open(&db_file, false)?;
        assert_eq!(db.export_html_workspace(&output)?, HtmlExport { pages: 2, attachments: 1 });
        let bytes = fs::read(&output)?;
        let mut archive = zip::ZipArchive::new(File::open(&output)?)?;
        let read = |archive: &mut zip::ZipArchive<File>, path: &str| -> Result<String> {
            let mut text = String::new();
            archive.by_name(path)?.read_to_string(&mut text)?;
            Ok(text)
        };
        let index = read(&mut archive, "index.html")?;
        assert!(index.contains(&format!("pages/{parent}.html")));
        assert!(!index.contains(&format!("pages/{trash}.html")) && !index.contains("不可匯出"));
        let html = read(&mut archive, &format!("pages/{parent}.html"))?;
        assert!(html.contains("同名 &lt;鏡頭&gt;") && html.contains("<strong>文字</strong>"));
        assert!(html.contains("&lt;script&gt;literal&lt;/script&gt;"));
        assert!(!html.contains("https://example.invalid") && !html.contains("<script>"));
        assert!(html.contains(&format!("href=\"{child}.html\"")));
        let child_page = read(&mut archive, &format!("pages/{child}.html"))?;
        assert!(child_page.contains(&format!("<a href=\"{parent}.html\">本機原文</a>")));
        assert!(child_page.contains(" · 僅保留深連結文字"));
        assert!(!child_page.contains(&format!("href=\"{parent}.html#")));
        assert!(child_page.contains("download=\"附檔 &amp; 引用.txt\""));
        assert_eq!(read(&mut archive, &format!("markdown/{parent}.md"))?, raw_body);
        assert_eq!(read(&mut archive, &format!("files/{attachment}.txt"))?, asset);
        assert!(db.export_html_workspace(&output).is_err());
        assert_eq!(fs::read(&output)?, bytes);
        drop(archive);
        drop(db);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
    #[test]
    fn late_attachment_error_removes_staging_and_keeps_notes() -> Result<()> {
        let dir = super::super::tests::data_dir();
        let output = dir.join("不完整.zip");
        let mut db = Notebook::open(Path::new(":memory:"), false)?;
        db.add(None, "足夠先寫入的筆記", "內文")?;
        let broken = db.add(None, "含遺失資料的附件", "下一篇")?;
        db.connection.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,'缺件.png','png',X'ff')", [broken])?;
        assert!(db.export_html_workspace(&output).is_err());
        assert_eq!(fs::read_dir(&dir)?.count(), 0);
        assert!(db.note(broken)?.is_some());
        fs::remove_dir_all(dir)?;
        Ok(())
    }

}
