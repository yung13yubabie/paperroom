use super::{HWND, Result, file_dialog};
use crate::dialog::{prompt, show_text};
use crate::store::notion_preview::{
    CellEncoding, CsvChoice, Member, NotionPreviewSession, PageChoice, Part, Report, Resolution, Scope,
    Selection,
};
use std::collections::BTreeSet;
use std::fmt::Write;

const MAX_DIALOG_BYTES: usize = 28_000;

fn member_label(member: &Member) -> String {
    match &member.part {
        Some(part) => format!("部件 {part:?} / 內部路徑 {:?}", member.path),
        None => format!("外層 ZIP / 內部路徑 {:?}", member.path),
    }
}

fn list_text<'a>(intro: &str, items: impl IntoIterator<Item = &'a String>) -> Result<String> {
    let mut text = format!("{intro}\n\n");
    for (i, item) in items.into_iter().enumerate() {
        writeln!(text, "{}. {item}", i + 1)?;
        if text.len() > MAX_DIALOG_BYTES {
            return Err("清單過長，無法完整顯示精確來源。請改用範圍較小的原始匯出檔。".into());
        }
    }
    Ok(text)
}

fn numbers(text: &str, limit: usize) -> Result<Vec<usize>> {
    let mut seen = BTreeSet::new();
    let mut picked = Vec::new();
    for segment in text.split(',') {
        let index = segment.trim().parse::<usize>().ok()
            .filter(|n| (1..=limit).contains(n))
            .ok_or("只接受清單內的序號，請用半形逗號分隔。")? - 1;
        if !seen.insert(index) {
            return Err("同一個序號不可重複選取。".into());
        }
        picked.push(index);
    }
    Ok(picked)
}

unsafe fn select(hwnd: HWND, title: &str, text: &str, limit: usize, exactly_one: bool)
    -> Result<Option<Vec<usize>>>
{
    unsafe {
        show_text(hwnd, title, text)?;
        let hint = if exactly_one { "輸入一個剛才清單上的序號；取消可離開。" }
            else { "輸入已核對的序號，以半形逗號分隔；取消可離開。" };
        let Some(value) = prompt(hwnd, title, hint, "", if exactly_one { 12 } else { 6_000 })?
            else { return Ok(None); };
        let picked = numbers(&value, limit)?;
        if exactly_one && picked.len() != 1 {
            return Err("請只選一個序號。".into());
        }
        Ok(Some(picked))
    }
}

fn scope_part(page: &PageChoice) -> Part<'_> {
    page.member.part.as_deref().map_or(Part::Outer, Part::Member)
}

fn outcome(status: &Resolution, selected: &[&PageChoice]) -> String {
    match status {
        // A shared identity registry lets the adapter detect duplicates without
        // choosing a winner. The UI must also withhold candidates on the wrong side.
        Resolution::Candidate(member) if selected.iter().any(|page| page.member == *member) =>
            format!("候選：{}", member_label(member)),
        Resolution::Candidate(_) | Resolution::OutOfScope => "不在你核准的此側範圍".into(),
        Resolution::InvalidId => "不是可核對的 ID".into(),
        Resolution::Missing => "無已核准的頁面".into(),
        Resolution::Ambiguous => "多個頁面使用同一 ID；未配對".into(),
        Resolution::UnsupportedValue => "值不支援；未配對".into(),
        Resolution::DuplicateValue => "欄位內的重複值".into(),
        Resolution::SelfTarget => "指向同一頁面；未配對".into(),
    }
}

fn report_text(report: &Report, source: &[&PageChoice], target: &[&PageChoice])
    -> Result<String>
{
    let mut text = format!(
        "只有候選，沒有匯入筆記、還原關聯或變更檔案。\n{}\n來源 ID 欄 {} / 關聯欄 {}\n共 {} 筆 CSV 資料列。\n\n",
        member_label(&report.csv.member), report.source_id_column + 1,
        report.relation_column + 1, report.rows.len()
    );
    if report.rows.is_empty() {
        text.push_str("CSV 有欄名，沒有資料列。");
    }
    for row in &report.rows {
        let mut chunk = format!("第 {} 筆 / ID {:?} / 原始關聯 {:?}\n來源：{}\n",
            row.data_record_ordinal, row.original_source_field, row.original_relation_field,
            outcome(&row.source, source));
        let source_ok = matches!(&row.source,
            Resolution::Candidate(member) if source.iter().any(|page| page.member == *member));
        for (value, status) in &row.tokens {
            let detail = if source_ok { outcome(status, target) }
                else { "來源未核實，沒有候選關聯".into() };
            writeln!(chunk, "  {value:?} → {detail}")?;
        }
        if text.len() + chunk.len() > MAX_DIALOG_BYTES {
            writeln!(text, "顯示上限：從第 {} 筆起未顯示；未顯示不代表無關聯。", row.data_record_ordinal)?;
            break;
        }
        text.push_str(&chunk);
        text.push('\n');
    }
    Ok(text)
}

unsafe fn show_original(hwnd: HWND, preview: &NotionPreviewSession, csv: &CsvChoice)
    -> Result<()>
{
    unsafe {
        let original = preview.original_text(csv)?;
        let pages = original.page_count();
        let mut number = 1;
        loop {
            let page = original.page(number).ok_or("找不到原始 CSV 的這一頁。")?;
            let heading = format!(
                "{}\n第 {number} / {pages} 頁 · 原檔位元組 {} 起，顯示 {} 位元組\n\\r、\\t 與不可見符號以跳脫形式顯示。此處未解析、匯入或配對資料。\n\n",
                member_label(&csv.member), page.bytes.start, page.bytes.len()
            );
            show_text(hwnd, "Notion CSV 原文（唯讀）", &(heading + &page.display_text()))?;
            if pages == 1 {
                return Ok(());
            }
            let next = if number < pages { (number + 1).to_string() } else { String::new() };
            let Some(entered) = prompt(
                hwnd, "Notion CSV 原文", "輸入頁號以繼續；取消可關閉。", &next, 12,
            )? else { return Ok(()); };
            let number_text = numbers(&entered, pages)?;
            if number_text.len() != 1 {
                return Err("每次請輸入一個頁號。".into());
            }
            number = number_text[0] + 1;
        }
    }
}

pub(super) unsafe fn run(hwnd: HWND) -> Result<()> {
    unsafe {
        let Some(path) = file_dialog(hwnd, "Notion Markdown ZIP\0*.zip\0\0", "zip", None)?
            else { return Ok(()); };
        let preview = NotionPreviewSession::open(&path)?;
        let files: Vec<_> = preview.csv_choices().collect();
        if files.is_empty() {
            return show_text(hwnd, "Notion 關聯預覽", "封裝裡沒有可選的 CSV；沒有可預覽的關聯。");
        }
        let names: Vec<_> = files.iter().map(|choice| member_label(&choice.member)).collect();
        let text = list_text("自行辨認要預覽的 CSV；不依同名、_all 或排序自動選取。", &names)?;
        let Some(file) = select(hwnd, "1 / 選擇原始 CSV", &text, names.len(), true)?
            else { return Ok(()); };
        let csv = files[file[0]];
        let Some(mode) = prompt(hwnd, "Notion CSV（唯讀）",
            "1 = 檢視原始 CSV；2 = 預覽關聯（需有來源頁面 ID）。", "1", 3)?
            else { return Ok(()); };
        match mode.trim() {
            "1" => return show_original(hwnd, &preview, csv),
            "2" => {}
            _ => return Err("請輸入 1（原始 CSV）或 2（關聯預覽）。".into()),
        }
        let (columns, rows) = preview.csv_header_and_count(csv)?;
        let names: Vec<_> = columns.iter().map(|name| format!("{name:?}")).collect();
        if rows == 0 {
            let summary = format!("{}\nCSV 有欄名，沒有資料列；未建立候選關聯。", member_label(&csv.member));
            return show_text(hwnd, "Notion 關聯預覽（唯讀）", &list_text(&summary, &names)?);
        }
        if names.len() < 2 {
            return show_text(hwnd, "Notion 關聯預覽", "至少要有兩個不同欄位才能指定來源 ID 和關聯。");
        }
        let text = list_text(
            "核對欄位後按順序輸入「來源頁面 ID 欄、關聯欄、關聯格式」的序號。\n來源欄須為實際頁面 ID；關聯欄須為支援的 Notion 頁面連結。不用標題補猜。\n最後一個序號代表格式：1 = 單值；2 = 以逗號加空格分隔多值。",
            &names,
        )?;
        show_text(hwnd, "2 / 核對欄位與格式", &text)?;
        let Some(map) = prompt(hwnd, "2 / 指定欄位與格式", "輸入 3 個數字，使用半形逗號分隔。", "", 24)?
            else { return Ok(()); };
        let input: Vec<_> = map.split(',').collect();
        if input.len() != 3 { return Err("請輸入來源欄、關聯欄和格式三個序號。".into()); }
        let column_indices = numbers(&format!("{},{}", input[0], input[1]), names.len())?;
        let encoding = match input[2].trim() {
            "1" => CellEncoding::Single,
            "2" => CellEncoding::CommaSpaceSeparated,
            _ => return Err("關聯格式只能用 1（單值）或 2（逗號加空格分隔）。".into()),
        };

        let pages: Vec<_> = preview.page_choices().collect();
        if pages.is_empty() {
            return show_text(hwnd, "Notion 關聯預覽", "此封裝沒有可核對 ID 的 Markdown 成員；未建立候選關聯。");
        }
        let names: Vec<_> = pages.iter().map(|page| {
            format!("{} · ID 候選 {:?}", member_label(&page.member), page.page_id_candidate)
        }).collect();
        let source_text = list_text("只選你已核對為來源的精確部件、路徑與 ID。檔名出現 ID 本身不是證明。", &names)?;
        let Some(source_pick) = select(hwnd, "3 / 核准來源頁面", &source_text, pages.len(), false)?
            else { return Ok(()); };
        let target_text = list_text("只選你已核對為關聯目標的精確部件、路徑與 ID；可與來源重疊。", &names)?;
        let Some(target_pick) = select(hwnd, "4 / 核准目標頁面", &target_text, pages.len(), false)?
            else { return Ok(()); };
        let source: Vec<_> = source_pick.iter().map(|&i| pages[i]).collect();
        let target: Vec<_> = target_pick.iter().map(|&i| pages[i]).collect();
        let approved: Vec<_> = source_pick.into_iter().chain(target_pick).collect::<BTreeSet<_>>()
            .into_iter().map(|i| (*pages[i]).clone()).collect();
        let source_parts: Vec<_> = source.iter().map(|p| scope_part(p)).collect();
        let target_parts: Vec<_> = target.iter().map(|p| scope_part(p)).collect();
        let report = preview.preview(Selection {
            csv, approved_pages: &approved, source_id_column: column_indices[0],
            relation_column: column_indices[1], encoding,
            source: Scope { parts: &source_parts, ancestor: &[] },
            target: Scope { parts: &target_parts, ancestor: &[] },
        })?;
        show_text(hwnd, "Notion 關聯預覽（唯讀）", &report_text(&report, &source, &target)?)
    }
}
