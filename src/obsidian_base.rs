//! Read-only subset of Obsidian Bases, projected onto imported Markdown file notes.
use std::collections::{BTreeMap, BTreeSet};
use yaml_rust2::{Yaml, parser::{Event, Parser}, scanner::TScalarStyle};

type R<T> = Result<T, String>;
pub(crate) const MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Layout { Table, List }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FolderRule { Exact(String), Subtree(String) }

#[derive(Debug)]
pub(crate) struct View {
    pub name: String,
    pub layout: Layout,
    /// Canonical identifiers, in display order; empty means no explicit order.
    pub order: Vec<String>,
    pub folders: Vec<FolderRule>,
}

impl View {
    /// Only pass imported active Markdown file paths from the selected Vault.
    pub fn matches_path(&self, relative: &str) -> bool {
        let parent = relative.rsplit_once('/').map_or("", |(dir, _)| dir);
        self.folders.iter().all(|rule| match rule {
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
    pub display_names: BTreeMap<String, String>,
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
    if matches!(s.as_str(), "file.name" | "file.folder" | "file.path" | "file.ext" | "file.basename") {
        return Ok(s);
    }
    let key = s.strip_prefix("note.").unwrap_or(&s);
    if key.is_empty() || key.len() > 128 || !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("不支援的 Bases 屬性：{s}"));
    }
    Ok(format!("note.{key}"))
}

fn predicate(s: &str) -> R<FolderRule> {
    let s = s.trim();
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

fn filters(value: Node, rules: &mut Vec<FolderRule>) -> R<()> {
    match value {
        Node::Text(s) if rules.len() < 32 => { rules.push(predicate(&s)?); Ok(()) }
        Node::Map(mut m) if m.len() == 1 && m.contains_key("and") => {
            let terms = m.remove("and").unwrap().seq()?;
            if terms.is_empty() { return Err("不支援空的 and 條件。".into()); }
            for term in terms { filters(term, rules)?; }
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
    let mut display_names = BTreeMap::new();
    if let Some(properties) = root.remove("properties") {
        for (key, config) in properties.map()? {
            let mut config = config.map()?;
            let display = label(config.remove("displayName").ok_or("屬性缺少 displayName。")?.text()?)?;
            supported(config)?;
            if display_names.insert(property(key)?,display).is_some() { return Err("屬性名稱重複。".into()); }
        }
    }
    let mut global = Vec::new();
    if let Some(f) = root.remove("filters") { filters(f,&mut global)?; }
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
        let mut folders = global.clone();
        if let Some(f) = row.remove("filters") { filters(f,&mut folders)?; }
        let mut order = Vec::new();
        if let Some(columns) = row.remove("order") {
            for col in columns.seq()? {
                let key = property(col.text()?)?;
                if order.len() >= 64 || order.contains(&key) { return Err("欄位重複或超過 64 個。".into()); }
                order.push(key);
            }
        }
        supported(row)?;
        views.push(View {name, layout, order, folders});
    }
    Ok(Base { raw: raw.to_vec(), views, display_names })
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
