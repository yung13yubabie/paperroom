//! Read-only subset of Obsidian Bases, projected onto imported Markdown file notes.
use super::obsidian_base_typed::Test;
use std::collections::{BTreeMap, BTreeSet};
use yaml_rust2::{Yaml, parser::{Event, Parser}, scanner::TScalarStyle};

type R<T> = Result<T, String>;
pub(crate) const MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Layout { Table, List }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FolderRule { Path(String), Exact(String), Subtree(String) }

#[derive(Debug)]
pub(crate) struct View {
    pub name: String,
    pub layout: Layout,
    /// Canonical identifiers, in display order; empty means no explicit order.
    pub order: Vec<String>,
    pub folders: Vec<FolderRule>,
    pub typed_filters: Vec<Test>,
    pub group: Option<String>,
    pub sorts: Vec<(String,bool)>,
}

impl View {
    /// Only pass imported active Markdown file paths from the selected Vault.
    pub fn matches_path(&self, relative: &str) -> bool {
        let parent = relative.rsplit_once('/').map_or("", |(dir, _)| dir);
        self.folders.iter().all(|rule| match rule {
            FolderRule::Path(path) => relative==path,
            FolderRule::Exact(dir) => parent == dir,
            FolderRule::Subtree(dir) => parent == dir || parent.strip_prefix(dir.as_str())
                .is_some_and(|tail| tail.starts_with('/')),
        })
    }
}

#[derive(Debug)]
pub(crate) struct Base {
    pub raw: Vec<u8>,
    pub views: Vec<View>,
    pub types: BTreeMap<String,&'static str>,
    pub formulas: BTreeMap<String, FileFormula>,
    pub display_names: BTreeMap<String, String>,
}

impl Base {
    pub(crate) fn has_live_status(&self)->bool {
        self.views.iter().flat_map(|view|view.order.iter()).any(|key|key=="note.status"
            || self.formulas.get(key).and_then(FileFormula::status_literal).is_some()) || !self.types.is_empty()
    }
}

#[derive(Clone, Debug)]
enum FileText { Folder, Path, Extension, Status, Typed(Test) }
#[derive(Clone, Debug)]
enum StringTest { Contains, StartsWith, EndsWith }

#[derive(Clone, Debug)]
pub(crate) struct FileFormula { field: FileText, test: StringTest, literal: String }

impl FileFormula {
    fn parse(expression: &str) -> R<Self> {
        if let Some((lhs,rhs))=expression.split_once("==") && matches!(lhs.trim(),"status"|"note.status") {
            let literal:String=serde_json::from_str(rhs.trim()).map_err(|_|"status 等值公式需要一個雙引號字串。")?;
            if literal.len()>512 || literal.chars().any(char::is_control) {return Err("公式文字過長或含控制字元。".into());}
            return Ok(Self{field:FileText::Status,test:StringTest::Contains,literal});
        }
        if let Some(test)=Test::parse(expression){return Ok(Self{field:FileText::Typed(test),test:StringTest::Contains,literal:String::new()})}
        let (call, argument) = expression.trim().split_once('(').ok_or("公式需要單一檔案文字函式。")?;
        let (receiver, method) = call.trim_end().rsplit_once('.').ok_or("公式缺少檔案欄位。")?;
        let field = match receiver {
            "file.folder" => FileText::Folder,
            "file.path" => FileText::Path, "file.ext" => FileText::Extension,
            _ => return Err("公式只支援資料夾、路徑和副檔名。".into()),
        };
        let test = match method {
            "contains" => StringTest::Contains, "startsWith" => StringTest::StartsWith,
            "endsWith" => StringTest::EndsWith,
            _ => return Err("公式只支援 contains、startsWith 和 endsWith。".into()),
        };
        let argument = argument.strip_suffix(')').ok_or("公式缺少結尾括號。")?;
        let literal: String = serde_json::from_str(argument.trim()).map_err(|_|"公式參數需要一個雙引號字串。")?;
        if literal.len()>512 || literal.chars().any(char::is_control) { return Err("公式字串超過 512 位元組或含控制字元。".into()); }
        Ok(Self { field, test, literal })
    }

    pub(super) fn typed(&self)->Option<&Test>{if let FileText::Typed(test)=&self.field{Some(test)}else{None}}
    pub(super) fn status_literal(&self)->Option<&str>{matches!(self.field,FileText::Status).then_some(&self.literal)}
    pub(super) fn display(&self, path: &str, folder: &str, name: &str) -> &'static str {
        let text = match self.field {
            FileText::Status | FileText::Typed(_) => unreachable!("status formulas use their source column"),
            FileText::Folder => folder, FileText::Path => path,
            FileText::Extension => name.rsplit_once('.').map_or("", |(_, ext)| ext),
        };
        let matches = match self.test {
            StringTest::Contains => text.contains(&self.literal),
            StringTest::StartsWith => text.starts_with(&self.literal),
            StringTest::EndsWith => text.ends_with(&self.literal),
        };
        if matches { "true" } else { "false" }
    }
}

enum Node { Text(String), Map(BTreeMap<String, Node>), Seq(Vec<Node>) }

impl Node {
    fn map(self) -> R<BTreeMap<String, Node>> {
        if let Self::Map(m) = self { Ok(m) } else { Err("Bases 需要 YAML 對應表。".into()) }
    }
    fn text(self) -> R<String> {
        if let Self::Text(s) = self { Ok(s) } else { Err("Bases 需要純文字值。".into()) }
    }
    fn seq(self) -> R<Vec<Node>> {
        if let Self::Seq(v) = self { Ok(v) } else { Err("Bases 需要 YAML 清單。".into()) }
    }
}

fn node(events: &mut std::iter::Peekable<std::vec::IntoIter<Event>>, depth: usize) -> R<Node> {
    if depth > 12 { return Err("Bases YAML 超過 12 層。".into()); }
    match events.next() {
        Some(Event::Scalar(s, style, 0, None)) if !s.is_empty() && s.len() <= 1024
            && !s.chars().any(char::is_control)
            && (style != TScalarStyle::Plain || matches!(Yaml::from_str(&s), Yaml::String(_))) => Ok(Node::Text(s)),
        Some(Event::MappingStart(0, None)) => {
            let mut result = BTreeMap::new();
            while events.peek() != Some(&Event::MappingEnd) {
                let key = node(events, depth+1)?.text()?;
                if result.insert(key.clone(), node(events, depth+1)?).is_some() {
                    return Err(format!("Bases 欄位重複：{key}"));
                }
            }
            events.next();
            Ok(Node::Map(result))
        }
        Some(Event::SequenceStart(0, None)) => {
            let mut result = Vec::new();
            while events.peek() != Some(&Event::SequenceEnd) { result.push(node(events, depth+1)?); }
            events.next();
            Ok(Node::Seq(result))
        }
        _ => Err("不支援的 Bases YAML 值、錨點、別名或標籤。".into()),
    }
}

/// Only a bounded prefix of the stored Markdown is needed.
pub(super) const FRONT_LIMIT: usize = 65536;

fn front_key(event: Event) -> R<String> {
    match event {
        Event::Scalar(s,TScalarStyle::Plain,0,None) if s=="<<"=>Err("不支援頁首 YAML 合併鍵。".into()),
        Event::Scalar(s,_,0,None) if !s.is_empty() && s.len()<=128 && !s.chars().any(char::is_control)=>Ok(s),
        _=>Err("頁首含不支援的鍵名、錨點或標籤。".into()),
    }
}

fn skip_front_value(event: Event, next: &mut impl FnMut()->R<Event>, depth: usize) -> R<()> {
    if depth>12 {return Err("頁首 YAML 超過 12 層。".into())}
    match event {
        Event::Scalar(_,_,0,None)=>Ok(()),
        Event::SequenceStart(0,None)=>{
            loop {
                let child=next()?;
                if child==Event::SequenceEnd {break}
                skip_front_value(child,next,depth+1)?;
            }
            Ok(())
        }
        Event::MappingStart(0,None)=>{
            let mut keys=BTreeSet::new();
            loop {
                let key=next()?;
                if key==Event::MappingEnd {break}
                if keys.len()>=256 || !keys.insert(front_key(key)?) {return Err("頁首欄位重複或超過 256 個。".into())}
                skip_front_value(next()?,next,depth+1)?;
            }
            Ok(())
        }
        _=>Err("頁首含不支援的錨點、別名或標籤。".into()),
    }
}

pub(super) fn front_fields(prefix: &[u8], selected: &BTreeSet<&str>) -> R<BTreeMap<String,String>> {
    parse_front(prefix,selected,false)
}
pub(super) fn front_fields_with_null(prefix:&[u8],selected:&BTreeSet<&str>)->R<BTreeMap<String,String>>{parse_front(prefix,selected,true)}
fn parse_front(prefix:&[u8],selected:&BTreeSet<&str>,keep_null:bool)->R<BTreeMap<String,String>> {
    let bytes = prefix.strip_prefix(&[0xef,0xbb,0xbf]).unwrap_or(prefix);
    let mut lines = bytes.split_inclusive(|&b| b == b'\n');
    fn line(bytes: &[u8]) -> &[u8] {
        let s=bytes.strip_suffix(b"\n").unwrap_or(bytes);
        s.strip_suffix(b"\r").unwrap_or(s)
    }
    let mut fields=BTreeMap::new();
    let Some(first)=lines.next() else {return Ok(fields)};
    if line(first)!=b"---" {return Ok(fields)}
    let mut end=first.len();
    for chunk in lines {
        if end+chunk.len()>FRONT_LIMIT {break}
        if line(chunk)!=b"---" {end+=chunk.len();continue}
        let header=std::str::from_utf8(&bytes[first.len()..end]).map_err(|_|"YAML 頁首不是 UTF-8。")?;
        if header.trim().is_empty() {return Ok(fields)}
        let mut parser=Parser::new_from_str(header);
        let mut events=0;
        let mut next=|| -> R<Event> {
            events+=1;
            if events>4096 {return Err("頁首超過 4096 個 YAML 事件。".into())}
            parser.next_token().map(|(e,_)|e).map_err(|_|"YAML 頁首格式錯誤。".into())
        };
        if next()?!=Event::StreamStart || next()?!=Event::DocumentStart || next()?!=Event::MappingStart(0,None) {
            return Err("頁首必須是單層 YAML 欄位表。".into());
        }
        let mut seen=BTreeSet::new();
        loop {
            let event=next()?;
            if event==Event::MappingEnd {break}
            let key=front_key(event)?;
            if seen.len()>=256 || !seen.insert(key.clone()) {return Err("頁首欄位重複或超過 256 個。".into())}
            let value=next()?;
            if !selected.contains(key.as_str()) {
                skip_front_value(value,&mut next,1)?;
                continue;
            }
            match value {
                Event::Scalar(s,style,0,None) if matches!(style,TScalarStyle::Plain|TScalarStyle::SingleQuoted|TScalarStyle::DoubleQuoted)
                    && s.len()<=1024 && !s.chars().any(char::is_control) => {
                        if keep_null || style!=TScalarStyle::Plain || !(s.is_empty() || matches!(Yaml::from_str(&s),Yaml::Null)) {
                            fields.insert(key,s);
                        }
                    }
                _=>return Err(format!("選取的頁首欄位「{key}」需要單行純量。")),
            }
        }
        if next()?!=Event::DocumentEnd || next()?!=Event::StreamEnd {return Err("頁首只能包含一份 YAML 文件。".into())}
        return Ok(fields);
    }
    Err("頁首缺少結尾分隔線，或超過 64 KiB。".into())
}

fn supported(map: BTreeMap<String, Node>) -> R<()> {
    match map.into_keys().next() {
        Some(k) => Err(format!("不支援的 Bases 設定：{k}")),
        None => Ok(()),
    }
}

fn label(s: String) -> R<String> {
    if s.trim().is_empty() || s.chars().count() > 160 { Err("Bases 名稱超過限制或為空。".into()) }
    else { Ok(s) }
}

fn property(s: String) -> R<String> {
    if matches!(s.as_str(), "file.name" | "file.folder" | "file.path" | "file.ext") {
        return Ok(s);
    }
    let (namespace,key) = s.strip_prefix("formula.").map_or_else(|| ("note",s.strip_prefix("note.").unwrap_or(&s)),|key|("formula",key));
    if key.is_empty() || key.len() > 128 || !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("不支援的 Bases 屬性：{s}"));
    }
    Ok(format!("{namespace}.{key}"))
}

fn predicate(s: &str) -> R<FolderRule> {
    let s = s.trim();
    if let Some(arg)=s.strip_prefix("file.path").and_then(|tail|tail.trim_start().strip_prefix("==")){
        let path:String=serde_json::from_str(arg.trim()).map_err(|_|"檔案路徑需要雙引號字串。")?;
        if path.len()>512 || !super::notion_handoff::windows_zip_path(&path){return Err("檔案條件須為有效的 Vault 相對路徑。".into());}
        return Ok(FolderRule::Path(path));
    }
    let (exact, arg) = if let Some(arg) = s.strip_prefix("file.folder").and_then(|v| v.trim_start().strip_prefix("==")) {
        (true,arg.trim())
    } else if let Some(arg) = s.strip_prefix("file.inFolder(").and_then(|v| v.strip_suffix(')')) {
        (false,arg.trim())
    } else { return Err("目前只支援 file.folder 等值或 file.inFolder 資料夾篩選。".into()); };
    let path: String = serde_json::from_str(arg).map_err(|_| "資料夾條件需要雙引號字串。")?;
    if path.is_empty() || path.len() > 512 || path.chars().any(char::is_control)
        || path.contains('\\') || path.split('/').any(|part| matches!(part, ""|"."|"..")) {
        return Err("資料夾須為 Vault 內清楚的相對路徑。".into());
    }
    Ok(if exact { FolderRule::Exact(path) } else { FolderRule::Subtree(path) })
}

fn filters(value: Node, rules: &mut Vec<FolderRule>, typed: &mut Vec<Test>) -> R<()> {
    match value {
        Node::Text(s) if rules.len()+typed.len() < 32 => { if let Some(test)=Test::parse(&s){typed.push(test);}else{rules.push(predicate(&s)?);} Ok(()) }
        Node::Map(mut m) if m.len() == 1 && m.contains_key("and") => {
            let terms = m.remove("and").unwrap().seq()?;
            if terms.is_empty() { return Err("不支援空的 and 條件。".into()); }
            for term in terms { filters(term, rules,typed)?; }
            Ok(())
        }
        _ => Err("不支援這項 Bases 篩選組合，或條件超過 32 個。".into()),
    }
}

pub(crate) fn parse(raw: &[u8]) -> R<Base> {
    if raw.len() > MAX_BYTES { return Err("Bases 超過 256 KiB。".into()); }
    let text = std::str::from_utf8(raw).map_err(|_| "Bases 須為 UTF-8。")?;
    let mut parser = Parser::new_from_str(text.strip_prefix('\u{feff}').unwrap_or(text));
    let mut events = Vec::new();
    loop {
        let (event, _) = parser.next_token().map_err(|e| format!("無效的 Bases YAML：{e}"))?;
        if event == Event::StreamEnd { break; }
        if events.len() >= 4096 { return Err("Bases 超過 4096 個 YAML 事件。".into()); }
        events.push(event);
    }
    let mut events = events.into_iter().peekable();
    if events.next() != Some(Event::StreamStart) || events.next() != Some(Event::DocumentStart) {
        return Err("找不到 Bases 文件。".into());
    }
    let mut root = node(&mut events,0)?.map()?;
    if events.next() != Some(Event::DocumentEnd) || events.next().is_some() {
        return Err("每個 Bases 檔案只能有一份 YAML 文件。".into());
    }
    let mut formulas = BTreeMap::new();
    if let Some(definitions) = root.remove("formulas") {
        let definitions=definitions.map()?;
        if definitions.len()>32 { return Err("每份 Bases 最多支援 32 條公式。".into()); }
        for (name, value) in definitions {
            let key=property(format!("formula.{name}"))?;
            let formula=FileFormula::parse(&value.text()?).map_err(|err|format!("公式「{name}」：{err}"))?;
            formulas.insert(key, formula);
        }
    }
    let mut display_names = BTreeMap::new();
    if let Some(properties) = root.remove("properties") {
        for (key, config) in properties.map()? {
            let mut config = config.map()?;
            let display = label(config.remove("displayName").ok_or("屬性缺少 displayName。")?.text()?)?;
            supported(config)?;
            if display_names.insert(property(key)?,display).is_some() { return Err("屬性名稱重複。".into()); }
        }
    }
    let mut global = Vec::new(); let mut global_typed=Vec::new();
    if let Some(f) = root.remove("filters") { filters(f,&mut global,&mut global_typed)?; }
    let rows = root.remove("views").ok_or("Bases 缺少 views。")?.seq()?;
    supported(root)?;
    if rows.is_empty() || rows.len() > 32 { return Err("需要 1 到 32 個檢視。".into()); }
    let mut views = Vec::new();
    let mut names = BTreeSet::new();
    for row in rows {
        let mut row = row.map()?;
        let layout = match row.remove("type").ok_or("檢視缺少 type。")?.text()?.as_str() {
            "table" => Layout::Table, "list" => Layout::List, _ => return Err("只支援表格與清單檢視。".into())
        };
        let name = label(row.remove("name").ok_or("檢視缺少 name。")?.text()?)?;
        if !names.insert(name.clone()) { return Err("檢視名稱重複。".into()); }
        let mut folders = global.clone();let mut typed_filters=global_typed.clone();
        if let Some(f) = row.remove("filters") { filters(f,&mut folders,&mut typed_filters)?; }
        let mut order = Vec::new();
        if let Some(columns) = row.remove("order") {
            for col in columns.seq()? {
                let key = property(col.text()?)?;
                if order.len() >= 64 || order.contains(&key) { return Err("欄位重複或超過 64 個。".into()); }
                order.push(key);
            }
        }
        let mut sorts=Vec::new();
        if let Some(config)=row.remove("sort"){
            let entries=config.seq()?;
            if entries.is_empty() || entries.len()>4 {return Err("動態檢視需要 1 至 4 個排序鍵。".into())}
            for entry in entries{
                let mut sort=entry.map()?;
                let key=property(sort.remove("property").ok_or("排序缺少 property。")?.text()?)?;
                let key=key.strip_prefix("note.").ok_or("排序只支援布林或數值來源欄位。")?.to_string();
                let desc=match sort.remove("direction").ok_or("排序缺少方向。")?.text()?.as_str(){"ASC"=>false,"DESC"=>true,_=>return Err("排序方向須為 ASC 或 DESC。".into())};
                supported(sort)?;
                if sorts.iter().any(|(k,_)|k==&key){return Err("排序鍵重複。".into())}
                sorts.push((key,desc));
            }
        }
        let group=if let Some(config)=row.remove("groupBy"){
            let mut config=config.map()?;
            let key=property(config.remove("property").ok_or("群組缺少來源屬性。")?.text()?)?;
            if config.remove("direction").ok_or("群組缺少 ASC 方向。")?.text()? != "ASC"{
                return Err("目前只支援 ASC 遞增型別群組。".into());
            }
            supported(config)?;
            if !order.contains(&key){return Err("型別群組的來源欄必須出現在 order。".into())}
            Some(key)
        }else{None};
        supported(row)?;
        views.push(View {name, layout, order, folders,typed_filters,group,sorts});
    }
    for key in display_names.keys().chain(views.iter().flat_map(|view|view.order.iter())) {
        if key.starts_with("formula.") && !formulas.contains_key(key) { return Err(format!("找不到公式：{key}")); }
    }
    let mut types=BTreeMap::new();
    for test in formulas.values().filter_map(FileFormula::typed).chain(views.iter().flat_map(|v|&v.typed_filters)) {
        if types.insert(test.key.clone(),test.kind).is_some_and(|prev|prev!=test.kind){return Err(format!("欄位「{}」有互相衝突的型別。",test.key))}
    }
    for view in &views {
        if view.sorts.iter().any(|(key,_)|!types.contains_key(key)){return Err("排序欄位須有明確布林或數值比較。".into());}
        if let Some(group)=&view.group && !group.strip_prefix("note.").is_some_and(|key|types.contains_key(key)){
            return Err("群組來源必須是已明確比較的布林或數值筆記欄位。".into())
        }
    }
    Ok(Base { types,raw: raw.to_vec(), views, display_names, formulas })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_style_views_preserve_raw_names_order_and_membership() {
        let raw = "\u{feff}filters:\r\n  and:\r\n    - file.inFolder(\"甲\")\r\nproperties:\n  status:\n    displayName: '進度'\nviews:\n  - type: table\n    name: 清單甲\n    filters: 'file.folder == \"甲/一\"'\n    order: [file.folder, note.status]\n  - type: list\n    name: 所有甲頁\n";
        let b = parse(raw.as_bytes()).unwrap();
        assert_eq!(b.raw,raw.as_bytes());
        assert_eq!(b.views[0].name,"清單甲");
        assert_eq!(b.views[0].layout,Layout::Table);
        assert_eq!(b.views[1].layout,Layout::List);
        assert_eq!(b.views[0].order,["file.folder","note.status"]);
        assert_eq!(b.display_names["note.status"],"進度");
        assert!(b.views[0].matches_path("甲/一/鏡頭.md"));
        assert!(!b.views[0].matches_path("甲/一/二/鏡頭.md"));
        assert!(b.views[1].matches_path("甲/一/二/鏡頭.md"));
        assert!(!b.views[1].matches_path("甲乙/一/鏡頭.md"));
    }
    #[test]
    fn refuses_unrepresented_features() {
        for body in ["views: [{type: cards, name: X}]", "views: [{type: table, name: X, sort: []}]",
            "views: [{type: list, name: X, name: Y}]", "formulas: {}\nviews: []",
            "views: [{type: list, name: X, order: [formula.total]}]",
            "views: &a [{type: list, name: X}]", "views: [{type: list, name: X}]\n---\nviews: []",
            "filters: 'file.inFolder(\"../甲\")'\nviews: [{type: list, name: X}]",
            "filters: {or: [true]}\nviews: [{type: list, name: X}]",
            "views: [{type: list, name: true}]"] {
            assert!(parse(body.as_bytes()).is_err(),"{body}");
        }
        assert!(parse(&vec![b' ';MAX_BYTES+1]).is_err());
    }
}
