//! JSON Canvas 1.0 interchange. The original UTF-8 is the stored representation.
use crate::store::Result;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ELEMENTS: usize = 15_000;
const MAX_CONTAINERS_AND_SEPARATORS: usize = 250_000;
const MAX_DEPTH: usize = 64;

pub struct Document {
    raw: String,
    tree: Value,
    extensions: bool,
    pub nodes: usize,
    pub edges: usize,
}

impl Document {
    pub fn json(&self) -> &Value {
        &self.tree
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub(crate) fn reparse(&self, raw: String) -> Result<Self> {
        parse_document(raw, self.extensions)
    }
}

pub fn read(path: &Path) -> Result<Document> {
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err("請選擇 8 MiB 以內的 JSON Canvas 檔案。".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err("JSON Canvas 超過 8 MiB。".into());
    }
    parse(String::from_utf8(bytes)?)
}

pub fn export(path: &Path, document: &Document) -> Result<()> {
    crate::output::publish_new(path, |file, _| {
        file.write_all(document.raw.as_bytes())?;
        Ok(())
    })
}

// Bound allocations before building the JSON value tree. Reject repeated keys
// (including escaped equivalents) so two readers cannot select different fields.
// serde_json remains responsible for the JSON grammar and string decoding.
fn checked_projection(source: &str) -> Result<Cow<'_, str>> {
    let mut escaped = String::new();
    let mut copied = 0;
    let bytes = source.as_bytes();
    let mut objects = Vec::<HashSet<String>>::new();
    let mut depth = 0usize;
    let mut budget = MAX_CONTAINERS_AND_SEPARATORS;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        if matches!(byte, b'{' | b'[' | b',') {
            budget = budget.checked_sub(1).ok_or("JSON Canvas 結構超過處理上限。")?;
        }
        match byte {
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEPTH { return Err("JSON Canvas 巢狀結構超過 64 層。".into()); }
                if byte == b'{' { objects.push(HashSet::new()); }
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1).ok_or("JSON Canvas 括號未配對。")?;
                if byte == b'}' { objects.pop().ok_or("JSON Canvas 物件未配對。")?; }
            }
            b'"' => {
                let start = i - 1;
                loop {
                    match bytes.get(i) {
                        Some(b'\\') => i += 2,
                        Some(b'"') => { i += 1; break; }
                        Some(_) => i += 1,
                        None => return Err("JSON Canvas 字串未結束。".into()),
                    }
                }
                let end = i;
                while bytes.get(i).is_some_and(u8::is_ascii_whitespace) { i += 1; }
                if bytes.get(i) == Some(&b':') {
                    let key: String = serde_json::from_str(&source[start..end])?;
                    // Arbitrary-precision serde_json uses a private "$..." key to
                    // transport numbers internally. Keep every user key literal.
                    if key.starts_with('$') {
                        escaped.push_str(&source[copied..start + 1]);
                        escaped.push('$');
                        copied = start + 1;
                    }
                    if !objects.last_mut().ok_or("JSON Canvas 欄位缺少物件。")?.insert(key) {
                        return Err("JSON Canvas 的同一物件有重複欄位。".into());
                    }
                }
            }
            _ => {}
        }
    }
    if copied == 0 {
        Ok(Cow::Borrowed(source))
    } else {
        escaped.push_str(&source[copied..]);
        Ok(Cow::Owned(escaped))
    }
}

fn restore_keys(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(restore_keys),
        Value::Object(map) => {
            for value in map.values_mut() { restore_keys(value); }
            let keys: Vec<_> = map.keys().filter(|k| k.starts_with("$$")).cloned().collect();
            // Remove all old keys first: "$x" and "$$x" may both be present.
            let renamed: Vec<_> = keys.into_iter()
                .filter_map(|k| map.remove(&k).map(|v| (k[1..].to_owned(), v)))
                .collect();
            map.extend(renamed);
        }
        _ => {}
    }
}

type Object = Map<String, Value>;

fn string_field<'a>(item: &'a Object, field: &str) -> Result<&'a str> {
    item.get(field).and_then(Value::as_str)
        .ok_or_else(|| format!("JSON Canvas 欄位 {field} 必須是字串。").into())
}

fn optional_string<'a>(item: &'a Object, field: &str) -> Result<Option<&'a str>> {
    item.get(field).map(|_| string_field(item, field)).transpose()
}

fn choice(item: &Object, field: &str, choices: &[&str]) -> Result<()> {
    if let Some(value) = optional_string(item, field)?
        && !choices.contains(&value)
    {
        return Err(format!("JSON Canvas 欄位 {field} 的值不符合 1.0 規格。").into());
    }
    Ok(())
}

fn color(item: &Object) -> Result<()> {
    if let Some(value) = optional_string(item, "color")? {
        let hex = value.strip_prefix('#').is_some_and(|digits|
            matches!(digits.len(), 3 | 4 | 6 | 8) && digits.bytes().all(|b| b.is_ascii_hexdigit())
        );
        if !hex && !matches!(value, "1" | "2" | "3" | "4" | "5" | "6") {
            return Err("JSON Canvas 色彩必須是十六進位色碼或預設色 1–6。".into());
        }
    }
    Ok(())
}

// Test mathematical integrality without rounding through f64 or converting a
// coordinate to a machine-sized integer. Geometry is preserved, never executed.
pub(crate) fn integer(value: Option<&Value>) -> bool {
    let Some(number) = value.and_then(Value::as_number) else { return false };
    let spelling = number.as_str();
    let (mantissa, exponent) = spelling.split_once(['e', 'E']).unwrap_or((spelling, "0"));
    let fraction = mantissa.split_once('.').map_or(0, |(_, part)| part.len());
    let mut zeros = 0i64;
    for digit in mantissa.bytes().rev() {
        match digit {
            b'0' => zeros += 1,
            b'.' | b'-' => {}
            _ => {
                let exponent = exponent.parse::<i64>().unwrap_or_else(|_| {
                    if exponent.starts_with('-') { i64::MIN } else { i64::MAX }
                });
                return exponent >= fraction as i64 - zeros;
            }
        }
    }
    true
}

fn elements<'a>(root: &'a Object, field: &str) -> Result<&'a [Value]> {
    match root.get(field) {
        None => Ok(&[]),
        Some(Value::Array(array)) => Ok(array),
        _ => Err(format!("JSON Canvas 欄位 {field} 必須是陣列。").into()),
    }
}

pub fn parse(raw: String) -> Result<Document> {
    parse_document(raw, false)
}

pub(crate) fn parse_bundle(raw: String) -> Result<Document> {
    parse_document(raw, true)
}

fn parse_document(raw: String, extensions: bool) -> Result<Document> {
    if raw.len() > MAX_BYTES { return Err("JSON Canvas 超過 8 MiB。".into()); }
    let source = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let projection = checked_projection(source)?;
    let mut tree: Value = serde_json::from_str(&projection)?;
    if matches!(projection, Cow::Owned(_)) { restore_keys(&mut tree); }
    drop(projection);
    let root = tree.as_object().ok_or("JSON Canvas 根節點必須是物件。")?;
    let nodes = elements(root, "nodes")?;
    let edges = elements(root, "edges")?;
    if nodes.len() + edges.len() > MAX_ELEMENTS {
        return Err("JSON Canvas 節點與連線合計超過 15,000 筆。".into());
    }
    let mut node_ids = HashSet::new();
    for node in nodes {
        let node = node.as_object().ok_or("JSON Canvas 節點必須是物件。")?;
        if !node_ids.insert(string_field(node, "id")?) { return Err("JSON Canvas 節點 ID 重複。".into()); }
        let kind = string_field(node, "type")?;
        if extensions && !matches!(kind, "text" | "link" | "file" | "group") {
            if kind.trim().is_empty() || kind.chars().any(char::is_control) {
                return Err("JSON Canvas 節點類型無效。".into());
            }
            // Extension nodes may use different geometry. They remain inert
            // metadata and the preview only draws bounded standard geometry.
            continue;
        }
        for key in ["x", "y", "width", "height"] {
            if !integer(node.get(key)) {
                return Err(format!("JSON Canvas 節點的 {key} 必須是整數。").into());
            }
        }
        color(node)?;
        match kind {
            "text" => { string_field(node, "text")?; }
            "link" => { string_field(node, "url")?; }
            "file" => {
                string_field(node, "file")?;
                if optional_string(node, "subpath")?.is_some_and(|path| !path.starts_with('#')) {
                    return Err("JSON Canvas 檔案節點的 subpath 必須以 # 開頭。".into());
                }
            }
            "group" => {
                optional_string(node, "label")?;
                optional_string(node, "background")?;
                choice(node, "backgroundStyle", &["cover", "ratio", "repeat"])?;
            }
            _ => return Err("JSON Canvas 包含不支援的節點類型。".into()),
        }
    }
    let mut edge_ids = HashSet::new();
    for edge in edges {
        let edge = edge.as_object().ok_or("JSON Canvas 連線必須是物件。")?;
        if !edge_ids.insert(string_field(edge, "id")?) { return Err("JSON Canvas 連線 ID 重複。".into()); }
        for field in ["fromNode", "toNode"] {
            if !node_ids.contains(string_field(edge, field)?) {
                return Err(format!("JSON Canvas 的 {field} 找不到對應節點。").into());
            }
        }
        for field in ["fromSide", "toSide"] {
            choice(edge, field, &["top", "right", "bottom", "left"])?;
        }
        for field in ["fromEnd", "toEnd"] { choice(edge, field, &["none", "arrow"])?; }
        optional_string(edge, "label")?;
        color(edge)?;
    }
    let counts = (nodes.len(), edges.len());
    Ok(Document { raw, tree, extensions, nodes: counts.0, edges: counts.1 })
}

// Import preserves the format's integer geometry. A read-only screen
// projection can omit nodes that do not have drawable positive dimensions.
pub fn geometry_for_preview(node: &serde_json::Value) -> Option<(f64, f64, f64, f64)> {
    let coordinate = |key: &str| node[key].as_f64().filter(|v| v.is_finite() && v.abs() <= 1e15);
    let (x, y, width, height) = (coordinate("x")?, coordinate("y")?,
                                  coordinate("width")?, coordinate("height")?);
    (width > 0.0 && height > 0.0).then_some((x, y, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_omits_undrawable_geometry_without_rewriting_it() {
        for node in [
            serde_json::json!({"x":0,"y":0,"width":-100,"height":20}),
            serde_json::json!({"x":0,"y":0,"width":20,"height":0}),
            serde_json::json!({"x":1e20,"y":0,"width":40,"height":20}),
        ] {
            assert!(geometry_for_preview(&node).is_none());
        }
        let valid = serde_json::json!({"x":-150,"y":-30,"width":40,"height":20});
        assert_eq!(geometry_for_preview(&valid), Some((-150.0, -30.0, 40.0, 20.0)));
    }

    #[test]
    fn all_canvas_types_round_trip_byte_for_byte_without_opening_references() -> Result<()> {
        let json = r##"{
 "nodes":[
  {"id":"g","type":"group","x":-20,"y":10,"width":400,"height":300,"label":"群組","background":"missing.png","backgroundStyle":"ratio","color":"#abcdef"},
  {"id":"t","type":"text","x":0,"y":100e-2,"width":70,"height":20,"text":"# 文字","extra":{"unknown":[1e500]}},
  {"id":"f","type":"file","x":20,"y":10,"width":200,"height":20,"file":"筆記/缺頁.md","subpath":"#^block"},
  {"id":"l","type":"link","x":10,"y":30,"width":10,"height":72,"url":"https://example.invalid/"}
 ],
 "edges":[{"id":"e","fromNode":"f","toNode":"t","fromSide":"right","toSide":"left","fromEnd":"none","toEnd":"arrow","label":"關聯","color":"6"}],
 "vendor":{"original":true}
}
"##;
        let raw = format!("\u{feff}{}", json.replace('\n', "\r\n"));
        let document = parse(raw.clone())?;
        assert_eq!((document.nodes, document.edges), (4, 1));
        let dir = std::env::temp_dir().join(format!("paperroom-canvas-test-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("交換.canvas");
        export(&path, &document)?;
        assert!(export(&path, &parse("{}".to_owned())?).is_err());
        let result = read(&path)?;
        assert_eq!(result.as_str().as_bytes(), raw.as_bytes());
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn extension_objects_and_large_geometry_keep_their_json_types() -> Result<()> {
        let source = r#"{"nodes":[{"id":"n","type":"text","text":"","x":1e500,"y":-0.00,"width":20,"height":20}],"vendor":{"\u0024serde_json::private::Number":"literal","$x":1,"$$x":2}}"#;
        let doc = parse(source.to_owned())?;
        let vendor = &doc.json()["vendor"];
        assert_eq!(vendor["$serde_json::private::Number"], "literal");
        assert_eq!(vendor["$x"], 1);
        assert_eq!(vendor["$$x"], 2);
        assert!(doc.json()["nodes"][0]["x"].is_number());
        assert!(doc.json()["nodes"][0]["x"].as_i64().is_none());
        assert_eq!(doc.as_str(), source);
        Ok(())
    }

    #[test]
    fn refuses_ambiguous_invalid_or_excessive_canvas_input() {
        for raw in [
            r#"{"nodes":[],"\u006eodes":[]}"#,
            r#"{"edges":[{"id":"e","fromNode":"missing","toNode":"missing"}]}"#,
            r#"{"nodes":[{"id":"t","type":"text","x":0.1,"y":0,"width":1,"height":1,"text":"ok"}]}"#,
            r#"{"nodes":[{"id":"t","type":"file","x":0,"y":0,"width":1,"height":1,"file":"a.md","subpath":"invalid"}]}"#,
            r#"{"edges":null}"#,
        ] { assert!(parse(raw.to_owned()).is_err(), "{raw}"); }
        assert!(parse(format!("{{\"extra\":{}0{}}}", "[".repeat(65), "]".repeat(65))).is_err());
        assert!(parse(" ".repeat(MAX_BYTES + 1)).is_err());
    }
}
