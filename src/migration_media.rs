//! Bind explicit local attachment references to source notes during a Vault import.
use super::{Result, attachments::{MAX_FILE_BYTES, MAX_PER_NOTE, MAX_STORAGE_BYTES, file_kind, has_expected_signature}};
use crate::{output::windows_output_name, vault::{Vault, hidden, open_entry}};
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use rusqlite::{Transaction,OptionalExtension,params};
use std::{collections::{BTreeSet,HashMap,HashSet}, ffi::OsStr, fs::File, io::Read, path::Path};

#[path = "vault_attachment_refresh.rs"]
mod refresh;

pub(super) const SCHEMA: &str = "
CREATE TABLE attachment_origins (
 attachment_id INTEGER PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
 relative_path TEXT NOT NULL
);";

#[derive(Default)]
pub(crate) struct MediaPlan {
    bindings: HashMap<String,BTreeSet<String>>,
    files: HashMap<String,Vec<u8>>,
    pub unresolved: usize,
    bytes: usize,
    details: Vec<String>,
    issues: usize,
    note_matches: usize,
    note_misses: usize,
}

pub(crate) fn supported(path:&Path) -> bool {
    // Notion CSV members are registered separately as original source files.
    !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
        && matches!(file_kind(path),Ok("pdf"|"png"|"jpeg"|"text"))
}

fn decode_component(raw:&str) -> Option<String> {
    let mut bytes = raw.bytes();
    let mut out = Vec::new();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let a = char::from(bytes.next()?).to_digit(16)?;
            let b = char::from(bytes.next()?).to_digit(16)?;
            let c = (a*16+b) as u8;
            if matches!(c,b'/'|b'\\'|0) { return None; }
            out.push(c);
        } else { out.push(b); }
    }
    String::from_utf8(out).ok()
}

fn safe(part:&str) -> bool {
    !part.is_empty() && !part.starts_with('.') && part.encode_utf16().count() <= 255
        && !part.contains(['/', '\\']) && windows_output_name(OsStr::new(part)).is_ok()
}

fn normalized(source:&[String], raw:&str, wiki:bool) -> Option<String> {
    if raw.starts_with("//") || raw.contains(['\\',':']) || (!wiki && raw.starts_with('/')) { return None; }
    let raw = if wiki {raw.strip_prefix('/').unwrap_or(raw)} else {raw};
    let relative = !wiki || raw.starts_with("./") || raw.starts_with("../");
    let mut parts = if relative { source[..source.len().checked_sub(1)?].to_vec() } else { Vec::new() };
    for chunk in raw.split('/') {
        let part = if wiki { chunk.to_string() } else { decode_component(chunk)? };
        match part.as_str() {
            "." => {}
            ".." => { parts.pop()?; }
            _ if safe(&part) => parts.push(part),
            _ => return None,
        }
    }
    (parts.len() <= 64 && parts.iter().all(|p|safe(p))).then(||parts.join("/"))
}

pub(crate) fn source_relative(source:&[String],raw:&str) -> Option<String> {
    normalized(source,raw,false)
}

fn resolve(source:&[String], raw:&str, wiki:bool, paths:&HashSet<&str>, names:&HashMap<&str,Option<&str>>) -> Option<String> {
    let path = normalized(source,raw,wiki)?;
    if wiki && !raw.contains('/') { return names.get(path.as_str()).copied().flatten().map(str::to_owned); }
    paths.contains(path.as_str()).then_some(path)
}

pub(crate) fn referenced_paths(body:&str)->impl Iterator<Item=(String,bool)> + '_ {
    Parser::new_ext(body,Options::ENABLE_WIKILINKS).filter_map(|event| {
        let (url,ty) = match event {
            Event::Start(Tag::Link {dest_url,link_type,..}|Tag::Image {dest_url,link_type,..}) => (dest_url,link_type),
            _ => return None,
        };
        let wiki=matches!(ty,LinkType::WikiLink{..});
        let raw=url.split('#').next()?;
        if raw.contains(':') || raw.starts_with("//") { return None; }
        let last=raw.rsplit('/').next()?;
        let name=if wiki {last.to_owned()} else {decode_component(last).unwrap_or_else(||last.to_owned())};
        supported(Path::new(&name)).then(||(raw.to_owned(),wiki))
    })
}

fn read_bytes(root:&Path, relative:&str, remaining:usize) -> Result<Vec<u8>> {
    if !root.is_absolute() || root.components().any(|p|p == std::path::Component::ParentDir) { return Err("需要完整的筆記庫路徑。".into()); }
    let mut handles: Vec<File> = Vec::new();
    let leaf = root.join(relative);
    let mut ancestors:Vec<_> = leaf.ancestors().collect();
    ancestors.reverse();
    let mut file = None;
    for location in ancestors {
        #[cfg(target_os="linux")]
        let path = if let (Some(parent),Some(name)) = (handles.last(),location.file_name()) {
            use std::os::fd::AsRawFd;
            std::path::PathBuf::from(format!("/proc/self/fd/{}",parent.as_raw_fd())).join(name)
        } else { location.to_owned() };
        #[cfg(not(target_os="linux"))]
        let path = location;
        let handle = open_entry(path.as_ref(),location != leaf)?;
        if location.starts_with(root) && location.file_name().is_some_and(|name|
            handle.metadata().map_or(true,|meta|hidden(name,&meta))) {
            return Err("附件路徑含隱藏項目。".into());
        }
        if location == leaf { file = Some(handle); } else { handles.push(handle); }
    }
    let file=file.ok_or("附件路徑無效。")?;
    let limit = remaining.min(MAX_FILE_BYTES);
    if hidden(leaf.file_name().ok_or("無效的附件名稱。")?,&file.metadata()?) ||
        file.metadata()?.len() > limit as u64 { return Err("附件已隱藏或超過匯入容量上限。".into()); }
    let mut data=Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut data)?;
    if data.len() > limit || !has_expected_signature(file_kind(&leaf)?,&data) { return Err("附件內容與格式不符或超過容量上限。".into()); }
    Ok(data)
}

struct MediaPaths<'a> {
    paths: HashSet<&'a str>,
    names: HashMap<&'a str, Option<&'a str>>,
}

impl<'a> MediaPaths<'a> {
    fn new(vault: &'a Vault) -> Self {
        let paths: HashSet<_> = vault.media_paths.iter().map(String::as_str).collect();
        let mut names = HashMap::new();
        for &path in &paths {
            if let Some(name) = path.rsplit('/').next() {
                names.entry(name).and_modify(|found| *found = None).or_insert(Some(path));
            }
        }
        Self { paths, names }
    }
}

pub(crate) fn plan(root:&Path,vault:&Vault) -> Result<MediaPlan> {
    let MediaPaths { paths, names } = MediaPaths::new(vault);
    let mut result=MediaPlan::default();
    let mut total=0usize;
    for page in &vault.pages {
        let mut found=BTreeSet::new();
        let mut missing=HashSet::new();
        for (raw,wiki) in referenced_paths(&page.body) {
            if let Some(path)=resolve(&page.parts,&raw,wiki,&paths,&names) { found.insert(path); }
            else if missing.insert(raw.clone()) {
                let why = if normalized(&page.parts,&raw,wiki).is_none() { "無效或超出來源範圍的路徑" }
                    else if wiki && !raw.contains('/') && names.get(raw.as_str()) == Some(&None) { "多個附件同名" }
                    else { "找不到可匯入的來源檔案" };
                result.issue(&page.parts.join("/"),&raw,"附件",why);
            }
            if missing.len()+found.len()>crate::archive::MAX_ENTRIES { return Err("筆記內的附件連結超過上限。".into()); }
        }
        result.unresolved += missing.len();
        if found.len()>MAX_PER_NOTE as usize { return Err("每篇筆記最多可有 16 個附件。".into()); }
        for path in &found {
            if !result.files.contains_key(path) {
                let bytes=read_bytes(root,path,MAX_STORAGE_BYTES as usize-total)?;
                result.files.insert(path.clone(),bytes);
            }
            total+=result.files[path].len();
            if total>MAX_STORAGE_BYTES as usize {return Err("附件總容量超過 128 MiB。".into());}
        }
        if !found.is_empty() { result.bindings.insert(page.parts.join("/"),found); }
    }
    let notes = vault.pages.iter().map(|p|(p,p.parts.join("/"))).collect::<Vec<_>>();
    result.check_notes(&notes,true);
    result.bytes = total;
    Ok(result)
}

impl MediaPlan {
    pub(crate) fn issue(&mut self,source:&str,raw:&str,category:&str,reason:&str) {
        self.issues+=1;
        if self.details.len()>=100 { return; }
        fn field(s:&str)->String {
            let mut chars=s.chars();
            let mut text:String=chars.by_ref().take(160).map(|c|if c.is_control(){' '}else{c}).collect();
            if chars.next().is_some() {text.push('…');}
            text
        }
        self.details.push(format!("{}　{}　{}　{}",field(source),category,field(raw),reason));
    }

    /// Check source targets without creating app links or changing Markdown.
    pub(crate) fn check_notes(&mut self,notes:&[(&crate::archive::ImportedPage,String)],vault:bool) {
        let paths:HashSet<_>=notes.iter().map(|(_,s)|s.as_str()).collect();
        let indexed:Vec<_>=notes.iter().enumerate().map(|(i,(_,s))|(i as i64,s.clone())).collect();
        let wiki_index=crate::vault::WikiIndex::new(&indexed);
        let mut basenames=HashMap::<&str,usize>::new();
        for path in &paths {
            let file=path.rsplit('/').next().unwrap_or(path);
            *basenames.entry(file).or_default()+=1;
            if let Some((stem,_))=file.rsplit_once('.') {*basenames.entry(stem).or_default()+=1;}
        }
        for (page,source) in notes {
            let parts=source.split('/').map(str::to_owned).collect::<Vec<_>>();
            for event in Parser::new_ext(&page.body,Options::ENABLE_WIKILINKS) {
                let Event::Start(Tag::Link{dest_url,link_type,..})=event else {continue};
                let target=dest_url.split('#').next().unwrap_or("");
                if target.is_empty() || target.contains(':') || target.starts_with("//") {continue;}
                let wiki=matches!(link_type,LinkType::WikiLink{..});
                let leaf=target.rsplit('/').next().unwrap_or("");
                let decoded=if wiki {leaf.to_owned()} else {decode_component(leaf).unwrap_or_else(||leaf.into())};
                let md=Path::new(&decoded).extension().is_some_and(|e|e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
                if wiki {
                    if !vault || leaf.parse::<i64>().is_ok() || (!md && Path::new(leaf).extension().is_some()) {continue;}
                } else if !md {continue;}
                let norm=normalized(&parts,target,wiki);
                let matched=if wiki {wiki_index.resolve(source,target).is_some()} else {norm.as_ref().is_some_and(|n|paths.contains(n.as_str()))};
                if matched {self.note_matches+=1;continue;}
                self.note_misses+=1;
                let why=if norm.is_none() {"無效或超出來源範圍的路徑"}
                    else if wiki && !target.contains('/') && basenames.get(target).copied().unwrap_or(0)>1 {"多篇筆記同名"}
                    else {"找不到可匯入的來源筆記"};
                self.issue(source,&dest_url,"筆記",why);
            }
        }
    }

    pub(crate) fn report(&self,notes:usize,skipped:usize)->String {
        let mut text=format!("匯入 {notes} 篇筆記；略過 {skipped} 個來源項目。\r\n附件綁定 {} 筆，未解析 {} 筆。\r\n本機筆記連結按出現次數核對來源檔案：相符 {} 筆，未解析 {} 筆。未核對標題錨點或外部網址。\r\n附件上限：單檔 8 MiB、每篇 16 個，工作區共 128 MiB。",self.count(),self.unresolved,self.note_matches,self.note_misses);
        if !self.details.is_empty() {
            text.push_str("\r\n\r\n來源筆記　類別　參照　原因\r\n");
            text.push_str(&self.details.join("\r\n"));
            if self.issues>self.details.len() {text.push_str(&format!("\r\n另有 {} 筆未列出。",self.issues-self.details.len()));}
        }
        text
    }

    pub(crate) fn add_bytes(&mut self,source:&str,relative:&str,bytes:&[u8])->Result<()> {
        if bytes.is_empty() || bytes.len()>MAX_FILE_BYTES || !relative.split('/').all(safe) ||
            !supported(Path::new(relative)) || !has_expected_signature(file_kind(Path::new(relative))?,bytes) {
            return Err("附件路徑、大小或格式不符。".into());
        }
        if self.files.get(relative).is_some_and(|existing|existing.as_slice()!=bytes) {
            return Err("來源路徑相同的附件含有不同內容。".into());
        }
        let targets = self.bindings.entry(source.to_owned()).or_default();
        if targets.contains(relative) { return Ok(()); }
        if targets.len()>=MAX_PER_NOTE as usize || self.bytes + bytes.len()>MAX_STORAGE_BYTES as usize {
            return Err("附件超過每篇 16 個或合計 128 MiB 的上限。".into());
        }
        targets.insert(relative.to_owned());
        self.files.entry(relative.to_owned()).or_insert_with(||bytes.to_vec());
        self.bytes+=bytes.len();
        Ok(())
    }

    pub(crate) fn file_count(&self)->usize {self.files.len()}
    pub(crate) fn count(&self)->usize {self.bindings.values().map(BTreeSet::len).sum()}
    pub(crate) fn attach(&self,tx:&Transaction<'_>,note:i64,source:&str)->Result<()> {
        let Some(paths)=self.bindings.get(source) else {return Ok(());};
        let used:i64=tx.query_row("SELECT coalesce(sum(length(content)),0) FROM attachments",[],|r|r.get(0))?;
        let added:usize=paths.iter().map(|p|self.files[p].len()).sum();
        if used>MAX_STORAGE_BYTES-added as i64 {return Err("工作區附件容量上限為 128 MiB。".into());}
        for path in paths {
            let name=path.rsplit('/').next().ok_or("附件名稱無效。")?;
            tx.execute("INSERT INTO attachments(note_id,name,kind,content) VALUES(?1,?2,?3,?4)",
                       params![note,name,file_kind(Path::new(name))?,self.files[path]])?;
            tx.execute("INSERT INTO attachment_origins(attachment_id,relative_path) VALUES(?1,?2)",
                       params![tx.last_insert_rowid(),path])?;
        }
        Ok(())
    }
}

impl super::Notebook {
    pub(crate) fn attachment_origin(&self,id:i64)->Result<Option<String>> {
        Ok(self.connection.query_row("SELECT relative_path FROM attachment_origins WHERE attachment_id=?1",
                                     [id],|r|r.get(0)).optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_media_preserves_exact_sources_and_skips_ambiguous_names() -> Result<()> {
        let root=std::env::temp_dir().join(format!("paperroom-migration-{}-{}",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        for folder in ["notes","a","b",".hidden"] { std::fs::create_dir_all(root.join(folder))?; }
        for (name,bytes) in [("a/id.txt","甲"),("b/id.txt","乙"),("a/筆 記.txt","同行"),(".hidden/off.txt","隱藏")] {
            std::fs::write(root.join(name),bytes)?;
        }
        let body="[甲](../a/id.txt) ![[b/id.txt]] [[id.txt]] [再](../a/id.txt)\n\n[字](../a/%E7%AD%86%20%E8%A8%98.txt) [隱](../.hidden/off.txt)\n\n`[code](../b/not.txt)`";
        std::fs::write(root.join("notes/one.md"),body)?;
        std::fs::write(root.join("two.md"),"![[/a/id.txt]]")?;
        let vault=crate::vault::read(&root)?;
        let media=plan(&root,&vault)?;
        assert_eq!((media.count(),media.file_count(),media.unresolved),(4,3,2));
        let report=media.report(vault.pages.len(),vault.skipped-media.file_count());
        assert!(report.contains("notes/one.md　附件　id.txt　多個附件同名"));
        assert!(report.contains("../.hidden/off.txt　無效或超出來源範圍的路徑"));
        assert!(!report.contains("not.txt"));
        let mut db=super::super::Notebook::open(Path::new(":memory:"),false)?;
        let import=db.import_vault_with_media("匯入",vault.pages,media)?;
        let note:i64=db.connection.query_row("SELECT note_id FROM vault_paths WHERE import_root=?1 AND relative_path='notes/one.md'",[import],|r|r.get(0))?;
        assert_eq!(db.note(note)?.unwrap().body,body);
        let attachments=db.list_attachments(note)?;
        assert_eq!(attachments.len(),3);
        let mut values=HashMap::new();
        for attachment in attachments {
            let path=db.attachment_origin(attachment.id)?.unwrap();
            let bytes:Vec<u8>=db.connection.query_row("SELECT content FROM attachments WHERE id=?1",[attachment.id],|r|r.get(0))?;
            values.insert(path,String::from_utf8(bytes)?);
        }
        assert_eq!(values["a/id.txt"],"甲");
        assert_eq!(values["b/id.txt"],"乙");
        assert_eq!(values["a/筆 記.txt"],"同行");
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn source_note_report_checks_links_in_context_and_caps_details() {
        let page = |name:&str,body:&str|crate::archive::ImportedPage{parts:vec![name.into()],title:name.into(),body:body.into()};
        let source=page("src","[[Same]] [[a/Same]] [valid](a/Same.md#section) [bad](../outside.md) [absent](b/Other.md) [remote](https://example.org/N.md) `[[Code]]`");
        let first=page("a",""); let second=page("b","");
        let notes=vec![(&source,"src.md".into()),(&first,"a/Same.md".into()),(&second,"b/Same.md".into())];
        let mut plan=MediaPlan::default();
        plan.check_notes(&notes,true);
        assert_eq!((plan.note_matches,plan.note_misses),(2,3));
        let r=plan.report(3,0);
        assert!(r.contains("src.md　筆記　Same　多篇筆記同名"));
        assert!(r.contains("b/Other.md") && !r.contains("example.org") && !r.contains("Code"));
        for _ in 0..110 { plan.issue("輸入","abc","附件","找不到"); }
        assert_eq!(plan.details.len(),100);
        assert!(plan.report(3,0).contains("另有 13 筆"));
    }

    #[test]
    fn source_relative_is_confined_and_encoded_separators_are_rejected() {
        let page=vec!["notes".into(),"one".into()];
        assert_eq!(source_relative(&page,"../assets/a%20b.png").as_deref(),Some("assets/a b.png"));
        for path in ["../../out.png","//server/a.pdf","C:/a.pdf","a%2Fb.png","../.hidden/a.png","../CON.pdf"] {
            assert_eq!(source_relative(&page,path),None);
        }
    }
}
