use super::*;
use crate::canvas::Document;
use crate::dialog::{prompt, prompt_multiline};
use crate::store::canvas_edit::{CanvasEdit, EdgeStyleField};
use serde_json::Value;

fn content_field(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "text" => ("text", "文字內容"),
        "file" => ("file", "Vault 內的檔案路徑"),
        "link" => ("url", "網址文字"),
        _ => ("label", "群組名稱"),
    }
}

fn menu_text(value: &str) -> String {
    value.chars().take(55).map(|ch| if ch.is_control() { ' ' } else { ch }).collect()
}

fn item_label(item: &Value) -> String {
    let id = menu_text(item["id"].as_str().unwrap_or_default());
    match item["type"].as_str() {
        Some(kind) => {
            let value = item[content_field(kind).0].as_str().unwrap_or_default();
            format!("{}　〔{id}〕", menu_text(value))
        }
        None => {
            let from = menu_text(item["fromNode"].as_str().unwrap_or_default());
            let to = menu_text(item["toNode"].as_str().unwrap_or_default());
            format!("{from} → {to}　〔{id}〕")
        }
    }
}

// Long documents are legal; construct only one native menu page at a time.
unsafe fn pick_item<'a>(
    hwnd: HWND, anchor: HWND, doc: &'a Document, collection: &str, title: &str,
) -> Result<Option<&'a Value>> {
    let items = doc.json()[collection].as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return Err(format!("{title}：畫布沒有可選項目。").into());
    }
    const PAGE: usize = 25;
    let mut start = 0;
    loop {
        let end = (start + PAGE).min(items.len());
        let mut labels = vec![(0, format!("{title}　{}–{} / {}", start + 1, end, items.len()))];
        labels.extend(items[start..end].iter().enumerate().map(|(n, item)| {
            (n + 1, format!("{}　{}", start + n + 1, item_label(item)))
        }));
        if start > 0 { labels.push((PAGE + 1, "上一頁".into())); }
        if end < items.len() { labels.push((PAGE + 2, "下一頁".into())); }
        let rows = labels.iter().map(|(id, label)| (*id, label.as_str())).collect::<Vec<_>>();
        let choice = unsafe { popup(hwnd, anchor, None, &rows) };
        match choice {
            0 => return Ok(None),
            n if n == PAGE + 1 => start -= PAGE,
            n if n == PAGE + 2 => start += PAGE,
            n => return Ok(items.get(start + n - 1)),
        }
    }
}

unsafe fn geometry(hwnd: HWND, initial: &str) -> Result<Option<[i64; 4]>> {
    let Some(text) = (unsafe { prompt(hwnd, "畫布位置與尺寸", "依序輸入 X、Y、寬、高，以空格分隔", initial, 128)? })
    else { return Ok(None); };
    let numbers = text.split_whitespace().map(str::parse::<i64>).collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "請以空格分隔四個整數。")?;
    let values: [i64; 4] = numbers.try_into().map_err(|_| "請輸入 X、Y、寬、高四個整數。")?;
    if values[2] <= 0 || values[3] <= 0 { return Err("寬及高必須大於零。".into()); }
    Ok(Some(values))
}

unsafe fn content(hwnd: HWND, kind: &str, initial: &str) -> Result<Option<String>> {
    let hint = content_field(kind).1;
    unsafe {
        if kind == "text" {
            prompt_multiline(hwnd, "畫布節點", hint, initial, 1_048_576)
        } else {
            prompt(hwnd, "畫布節點", hint, initial, 8192)
        }
    }
}

impl App {
    pub(super) unsafe fn canvas_edit_menu(&mut self, canvas_id: i64) -> Result<()> {
        unsafe {
            let Some(WorkspaceView::Canvas(view)) = self.extras.workspace.as_mut() else { return Ok(()); };
            if view.id != canvas_id { return Err("目前畫布已切換，請重新開啟選單。".into()); }
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let action = popup(hwnd, anchor, None, &[
                (1, "新增文字節點…"), (2, "新增檔案參照…"), (3, "新增網址節點…"), (4, "新增群組…"),
                (5, "移動或調整尺寸…"), (6, "編輯節點內容…"), (7, "修改節點標籤…"),
                (8, "移到最上層…"), (9, "移到最下層…"), (10, "刪除節點及相連線段…"),
                (11, "新增連線…"), (12, "修改連線標籤…"), (13, "刪除連線…"),
                (14, "修改連線端點與顏色…"),
            ]);
            let entered: String;
            let edit = match action {
                1..=4 => {
                    let kind = ["text", "file", "link", "group"][action - 1];
                    let Some(value) = content(hwnd, kind, "")? else { return Ok(()); };
                    let Some(rect) = geometry(hwnd, "0 0 320 180")? else { return Ok(()); };
                    entered = value;
                    CanvasEdit::AddNode { kind, content: &entered, geometry: rect }
                }
                5..=10 => {
                    let Some(node) = pick_item(hwnd, anchor, &view.doc, "nodes", "選取節點")? else { return Ok(()); };
                    let id = node["id"].as_str().ok_or("節點缺少識別碼。")?;
                    let kind = node["type"].as_str().ok_or("節點缺少類型。")?;
                    match action {
                        5 => {
                            let initial = ["x", "y", "width", "height"].map(|k| node[k].to_string()).join(" ");
                            let Some(rect) = geometry(hwnd, &initial)? else { return Ok(()); };
                            CanvasEdit::Geometry { id, geometry: rect }
                        }
                        6 => {
                            let initial = node[content_field(kind).0].as_str().unwrap_or_default();
                            let Some(value) = content(hwnd, kind, initial)? else { return Ok(()); };
                            entered = value;
                            CanvasEdit::Content { id, value: &entered }
                        }
                        7 => {
                            let initial = node["label"].as_str().unwrap_or_default();
                            let Some(label) = prompt(hwnd, "節點標籤", "標籤文字", initial, 8192)? else { return Ok(()); };
                            entered = label;
                            CanvasEdit::NodeLabel { id, label: (!entered.is_empty()).then_some(entered.as_str()) }
                        }
                        8 | 9 => CanvasEdit::ZOrder { id, front: action == 8 },
                        _ => {
                            if message(hwnd, "刪除節點", &format!("刪除「{}」及所有相連線段？", item_label(node)),
                                MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING) != IDYES { return Ok(()); }
                            CanvasEdit::DeleteNode { id }
                        }
                    }
                }
                11 => {
                    let Some(start) = pick_item(hwnd, anchor, &view.doc, "nodes", "選取連線起點")? else { return Ok(()); };
                    let Some(end) = pick_item(hwnd, anchor, &view.doc, "nodes", "選取連線終點")? else { return Ok(()); };
                    let Some(label) = prompt(hwnd, "新增連線", "連線標籤（可留白）", "", 8192)? else { return Ok(()); };
                    entered = label;
                    CanvasEdit::AddEdge {
                        from: start["id"].as_str().ok_or("起點缺少識別碼。")?,
                        to: end["id"].as_str().ok_or("終點缺少識別碼。")?,
                        label: (!entered.is_empty()).then_some(entered.as_str()),
                    }
                }
                12 | 13 => {
                    let Some(edge) = pick_item(hwnd, anchor, &view.doc, "edges", "選取連線")? else { return Ok(()); };
                    let id = edge["id"].as_str().ok_or("連線缺少識別碼。")?;
                    if action == 12 {
                        let initial = edge["label"].as_str().unwrap_or_default();
                        let Some(label) = prompt(hwnd, "連線標籤", "標籤文字", initial, 8192)? else { return Ok(()); };
                        entered = label;
                        CanvasEdit::EdgeLabel { id, label: (!entered.is_empty()).then_some(entered.as_str()) }
                    } else {
                        if message(hwnd, "刪除連線", &format!("刪除這條連線？\n{}", item_label(edge)),
                            MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING) != IDYES { return Ok(()); }
                        CanvasEdit::DeleteEdge { id }
                    }
                }
                14 => {
                    let Some(edge) = pick_item(hwnd, anchor, &view.doc, "edges", "選取連線")? else { return Ok(()); };
                    let id = edge["id"].as_str().ok_or("連線缺少識別碼。")?;
                    let field = match popup(hwnd, anchor, None, &[
                        (1,"起點位置"), (2,"終點位置"), (3,"起點箭頭"), (4,"終點箭頭"), (5,"線條顏色"),
                    ]) {
                        1 => EdgeStyleField::FromSide,
                        2 => EdgeStyleField::ToSide,
                        3 => EdgeStyleField::FromEnd,
                        4 => EdgeStyleField::ToEnd,
                        5 => EdgeStyleField::Color,
                        _ => return Ok(()),
                    };
                    let current = edge[field.key()].as_str();
                    let value = match field {
                        EdgeStyleField::Color => {
                            let Some(color) = prompt(hwnd, "線條顏色",
                                "輸入預設色 1–6 或十六進位色碼（例如 #70A9EF），留白使用預設。",
                                current.unwrap_or_default(), 9)? else { return Ok(()); };
                            entered = color.trim().to_owned();
                            (!entered.is_empty()).then_some(entered.as_str())
                        }
                        _ => {
                            let choices: &[(Option<&str>, &str)] = match field {
                                EdgeStyleField::FromSide | EdgeStyleField::ToSide =>
                                    &[(None,"自動"),(Some("top"),"上"),(Some("right"),"右"),
                                      (Some("bottom"),"下"),(Some("left"),"左")],
                                EdgeStyleField::FromEnd =>
                                    &[(None,"預設（無箭頭）"),(Some("none"),"無箭頭"),(Some("arrow"),"箭頭")],
                                _ => &[(None,"預設（箭頭）"),(Some("none"),"無箭頭"),(Some("arrow"),"箭頭")],
                            };
                            let captions: Vec<_> = choices.iter().map(|(choice, title)| {
                                if *choice == current { format!("✓ {title}") } else { (*title).to_owned() }
                            }).collect();
                            let items: Vec<_> = captions.iter().enumerate()
                                .map(|(i, name)| (i+1, name.as_str())).collect();
                            let picked = popup(hwnd, anchor, None, &items);
                            let Some(index) = picked.checked_sub(1) else { return Ok(()); };
                            choices[index].0
                        }
                    };
                    CanvasEdit::EdgeStyle { id, field, value }
                }
                _ => return Ok(()),
            };
            view.doc = self.notebook.edit_canvas(canvas_id, &view.doc, edit)?;
            if view.bundle.is_some() {
                view.bundle = self.notebook.canvas_bundle_view(canvas_id)?;
            }
            self.present();
            label(self.widgets.status, "已儲存畫布修改。");
            Ok(())
        }
    }
}
