//! Read-only, byte-bound Notion ZIP relation preview. Does not authenticate Notion,
//! edit any workspace, or suggest edges without explicitly approved page/column scope.

use super::csv_table::parse_csv_preview;
use super::notion_handoff::{CsvSources, windows_zip_path};
use super::notion_relation_resolver as relation;
pub(crate) use super::notion_relation_resolver::{CellEncoding, Part};
use super::snapshot_sha256::sha256;
use super::Result;
use crate::archive::{bounded_zip_snapshot, read_archive_snapshot};
use std::{collections::{BTreeMap, BTreeSet}, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub(crate) struct Member {
    pub(crate) part: Option<String>,
    pub(crate) path: String,
}
impl Member {
    fn new(part: Option<&str>, path: &str) -> Self {
        Self { part: part.map(str::to_owned), path: path.to_owned() }
    }
    fn part(&self) -> relation::Part<'_> {
        self.part.as_deref().map_or(relation::Part::Outer, relation::Part::Member)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CsvChoice {
    pub(crate) member: Member,
    archive_sha256: [u8; 32],
    csv_sha256: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageChoice {
    pub(crate) member: Member,
    pub(crate) page_id_candidate: String,
    archive_sha256: [u8; 32],
}

#[derive(Clone, Copy)]
pub(crate) struct Scope<'a> {
    pub(crate) parts: &'a [Part<'a>],
    pub(crate) ancestor: &'a [&'a str],
}
impl<'a> Scope<'a> {
    fn for_preview(self) -> relation::Scope<'a> {
        relation::Scope { import_root: PREVIEW_ONLY_ROOT, allowed_parts: self.parts, ancestor: self.ancestor }
    }
}
const PREVIEW_ONLY_ROOT: i64 = 1;
const MAX_REPORT_TOKENS: usize = 50_000;

pub(crate) struct Selection<'a> {
    pub(crate) csv: &'a CsvChoice,
    // The caller must explicitly approve exact archive members and their
    // candidate IDs; archive text is never proof that an ID belongs to Notion.
    pub(crate) approved_pages: &'a [PageChoice],
    pub(crate) source_id_column: usize,
    pub(crate) relation_column: usize,
    pub(crate) encoding: CellEncoding,
    pub(crate) source: Scope<'a>,
    pub(crate) target: Scope<'a>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Candidate(Member),
    InvalidId,
    Missing,
    OutOfScope,
    Ambiguous,
    UnsupportedValue,
    DuplicateValue,
    SelfTarget,
}
#[derive(Debug)]
pub(crate) struct Row {
    pub(crate) data_record_ordinal: usize,
    pub(crate) original_source_field: String,
    pub(crate) original_relation_field: String,
    pub(crate) source: Resolution,
    pub(crate) tokens: Vec<(String, Resolution)>,
}
#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) csv: CsvChoice,
    pub(crate) source_id_column: usize,
    pub(crate) relation_column: usize,
    pub(crate) rows: Vec<Row>,
}

const RAW_PAGE_BYTES: usize = 4096;

pub(crate) struct CsvText<'a> {
    original: &'a str,
    pages: Vec<std::ops::Range<usize>>,
}

pub(crate) struct CsvTextPage<'a> {
    pub(crate) bytes: std::ops::Range<usize>,
    pub(crate) text: &'a str,
}

impl<'a> CsvText<'a> {
    fn new(original: &'a str) -> Self {
        let mut pages = Vec::new();
        let mut start = 0;
        while start < original.len() {
            let mut end = (start + RAW_PAGE_BYTES).min(original.len());
            while !original.is_char_boundary(end) {
                end -= 1;
            }
            if end < original.len() && &original.as_bytes()[end - 1..=end] == b"\r\n" {
                end -= 1;
            }
            pages.push(start..end);
            start = end;
        }
        Self { original, pages }
    }

    pub(crate) fn page_count(&self) -> usize { self.pages.len() }

    pub(crate) fn page(&self, one_based: usize) -> Option<CsvTextPage<'a>> {
        let bytes = self.pages.get(one_based.checked_sub(1)?)?.clone();
        Some(CsvTextPage { text: &self.original[bytes.clone()], bytes })
    }
}

impl CsvTextPage<'_> {
    pub(crate) fn display_text(&self) -> String {
        let mut rendered = String::with_capacity(self.text.len());
        for c in self.text.chars() {
            match c {
                '\n' | '"' | '\'' => rendered.push(c),
                _ => rendered.extend(c.escape_debug()),
            }
        }
        rendered
    }
}

pub(crate) struct NotionPreviewSession {
    archive_sha256: [u8; 32],
    sources: CsvSources,
    pages: BTreeMap<Member, PageChoice>,
    csv: BTreeMap<Member, CsvChoice>,
}

fn page_candidate(path: &str) -> Option<&str> {
    let file = path.rsplit('/').next()?;
    let (stem, ext) = file.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case("md") && !ext.eq_ignore_ascii_case("markdown") { return None; }
    let (_, id) = stem.rsplit_once(' ')?;
    (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}
fn relation_error(err: relation::PlanError) -> String {
    format!("Notion 關聯預覽未完成：{err:?}。原始 CSV 未變更。")
}

impl NotionPreviewSession {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let snapshot = bounded_zip_snapshot(path)?;
        Self::from_snapshot(&snapshot)
    }

    fn from_snapshot(snapshot: &[u8]) -> Result<Self> {
        // Read and identify only one bounded snapshot, even if the user replaces
        // the filename on disk while the dialog is open.
        let mut sources = CsvSources::default();
        let imported = read_archive_snapshot(snapshot, None, Some(&mut sources))?;
        let archive_sha256 = sha256(snapshot);
        let mut pages = BTreeMap::new();
        for (part, path) in &imported.notion_members {
            if !windows_zip_path(path) { continue; }
            let Some(id) = page_candidate(path) else { continue; };
            let member = Member::new(part.as_deref(), path);
            if pages.insert(member.clone(),PageChoice { member, page_id_candidate:id.into(),archive_sha256 }).is_some() {
                return Err("關聯預覽遇到重複頁面來源。".into());
            }
        }
        let csv = sources.preview_members().map(|(part,path,bytes)| {
            let member=Member::new(part,path);
            let choice=CsvChoice { member:member.clone(),archive_sha256,csv_sha256:sha256(bytes) };
            (member,choice)
        }).collect();
        Ok(Self{archive_sha256,sources,pages,csv})
    }

    pub(crate) fn page_choices(&self) -> impl Iterator<Item=&PageChoice> { self.pages.values() }
    pub(crate) fn csv_choices(&self) -> impl Iterator<Item=&CsvChoice> { self.csv.values() }
    pub(crate) fn original_csv<'a>(&'a self, pick:&CsvChoice) -> Result<&'a [u8]> {
        if pick.archive_sha256!=self.archive_sha256 || self.csv.get(&pick.member)!=Some(pick) {
            return Err("CSV 選擇屬於不同或已變更的封裝。請重新選取。".into());
        }
        self.sources.preview_members().find(|(part,path,_)| *part==pick.member.part.as_deref()
            && *path==pick.member.path).map(|(_,_,bytes)|bytes)
            .ok_or_else(||"找不到已選取的原始 CSV。".into())
    }

    pub(crate) fn original_text<'a>(&'a self, pick: &CsvChoice) -> Result<CsvText<'a>> {
        let text = std::str::from_utf8(self.original_csv(pick)?)?;
        Ok(CsvText::new(text))
    }

    pub(crate) fn columns(&self, pick:&CsvChoice) -> Result<Vec<String>> {
        self.csv_header_and_count(pick).map(|(header, _)| header)
    }

    pub(crate) fn csv_header_and_count(&self, pick: &CsvChoice) -> Result<(Vec<String>, usize)> {
        let mut parsed = parse_csv_preview(self.original_csv(pick)?)?.into_iter();
        let header = parsed.next().ok_or("找不到 CSV 標題列。")?;
        Ok((header, parsed.len()))
    }

    pub(crate) fn preview(&self, request: Selection<'_>) -> Result<Report> {
        let bytes = self.original_csv(request.csv)?;
        let decoded = parse_csv_preview(bytes)?;
        let header = &decoded[0];
        let relation_name = header.get(request.relation_column).ok_or("無效的關聯欄位。")?;
        if request.source_id_column>=header.len() || request.source_id_column==request.relation_column {
            return Err("來源 ID 與關聯欄位必須明確且不同。".into());
        }
        if request.approved_pages.len() > crate::archive::MAX_ENTRIES {
            return Err("核對頁面過多。".into());
        }
        let mut approved = BTreeSet::new();
        for supplied in request.approved_pages {
            if self.pages.get(&supplied.member)!=Some(supplied) || !approved.insert(&supplied.member) {
                return Err("核對頁面並非本次的唯一封裝成員。請重新選取。".into());
            }
        }
        let paths: Vec<Vec<&str>> = request.approved_pages.iter()
            .map(|p|p.member.path.split('/').collect()).collect();
        let origins: Vec<_> = request.approved_pages.iter().zip(&paths).enumerate().map(|(i,(p,path))|
            relation::VerifiedPage{import_root:PREVIEW_ONLY_ROOT,part:p.member.part(),archive_path:path,
                page_id:&p.page_id_candidate,local_note:(i+1) as i64}).collect();
        let identities=relation::IdentityIndex::new(&origins).map_err(relation_error)?;
        let member_path: Vec<_> = request.csv.member.path.split('/').collect();
        let member=relation::CsvMemberIdentity::from_verified_snapshot(PREVIEW_ONLY_ROOT,
            self.archive_sha256,request.csv.member.part(),&member_path,request.csv.csv_sha256)
            .map_err(relation_error)?;
        let mapping=relation::BoundCsvMapping { member,source_id_column:request.source_id_column,
            relation_column:request.relation_column, column:relation::ExplicitColumn {
                relation_name, encoding:request.encoding,source:request.source.for_preview(),target:request.target.for_preview()
            } };
        // Convert synthetic resolver indices to exact archive members. Preview IDs
        // never escape as local note IDs and cannot be passed to DB mutators.
        let classify=|found|->Result<Resolution>{
            use relation::Lookup::*;
            Ok(match found {
                Found(index)=>Resolution::Candidate(request.approved_pages.get((index-1) as usize)
                    .ok_or("內部預覽頁面索引不存在。")?.member.clone()),
                Missing=>Resolution::Missing, InvalidId=>Resolution::InvalidId,
                OutOfScope=>Resolution::OutOfScope, Ambiguous=>Resolution::Ambiguous,
            })
        };
        let mut rows=Vec::new();
        let mut token_count=0;
        for (i, record) in decoded[1..].iter().enumerate() {
            let fields: Vec<_> = record.iter().map(String::as_str).collect();
            let verified=relation::VerifiedCsvRow::from_verified_record(member,i+1,&fields).map_err(relation_error)?;
            let result=identities.preview_verified_row(mapping,&verified).map_err(relation_error)?;
            token_count+=result.preview.tokens.len();
            if token_count>MAX_REPORT_TOKENS { return Err("Notion 關聯預覽項目超過 50,000。請分批預覽。".into()); }
            let mut tokens=Vec::new();
            for token in result.preview.tokens {
                let mapped=match token.outcome {
                    relation::Outcome::Resolved(i)=>classify(relation::Lookup::Found(i))?,
                    relation::Outcome::Unresolved(reason)=>classify(reason)?,
                    relation::Outcome::UnsupportedValue=>Resolution::UnsupportedValue,
                    relation::Outcome::DuplicateValue=>Resolution::DuplicateValue,
                    relation::Outcome::SelfTarget=>Resolution::SelfTarget,
                };
                tokens.push((token.original.to_owned(),mapped));
            }
            rows.push(Row { data_record_ordinal:result.data_record_ordinal,
                original_source_field:result.original_source_field.to_owned(),
                original_relation_field:result.preview.original_cell.to_owned(),
                source:classify(result.preview.source)?,tokens });
        }
        Ok(Report{csv:request.csv.clone(),source_id_column:request.source_id_column,
            relation_column:request.relation_column,rows})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/notion-relation-preview");
    const PART: relation::Part<'_> = relation::Part::Member("Part-1.zip");

    fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b|format!("{b:02x}")).collect() }
    fn expect() -> serde_json::Value {
        serde_json::from_slice(include_bytes!("../tests/fixtures/notion-relation-preview/expected.json")).unwrap()
    }
    fn selection<'a>(csv:&'a CsvChoice,pages:&'a [PageChoice], scope:Scope<'a>) -> Selection<'a> {
        Selection{ csv,approved_pages:pages,source_id_column:1,relation_column:2,
            encoding:relation::CellEncoding::CommaSpaceSeparated,source:scope,target:scope }
    }
    fn id_at_report_member(session:&NotionPreviewSession, resolution:&Resolution) -> Option<String> {
        match resolution {
            Resolution::Candidate(m)=>Some(session.pages[m].page_id_candidate.clone()),
            _=>None,
        }
    }

    #[test]
    fn preview_actual_nested_zip_member_and_original_cell_match_fixture_manifest() {
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let manifest=expect();
        let scope=Scope{parts:&[PART],ancestor:&[]};
        for variant in ["default","filtered"] {
            let expected=&manifest["variants"][variant];
            let path=Path::new(FIXTURE).join(expected["archive"].as_str().unwrap());
            let session=NotionPreviewSession::open(&path).unwrap();
            assert_eq!(hex(&session.archive_sha256),expected["archive_sha256"]);
            let csv=session.csv_choices().next().unwrap();
            assert_eq!(hex(&csv.csv_sha256),expected["member_sha256"][&csv.member.path]);
            assert_eq!(session.columns(csv).unwrap(),["名稱","外部頁面 ID","關聯","一般網址"]);
            let original=session.original_csv(csv).unwrap().to_vec();
            let pages:Vec<_>=session.page_choices().cloned().collect();
            let report=session.preview(selection(csv,&pages,scope)).unwrap();
            assert_eq!(report.rows.len(),expected["csv_rows"]);
            assert_eq!(report.csv,*csv);
            let first=&report.rows[0];
            assert_eq!(first.data_record_ordinal,1);
            assert_eq!(first.original_relation_field,manifest["original_relation_text"]);
            assert_eq!(Some(first.original_source_field.clone()),id_at_report_member(&session,&first.source));
            let resolved:Vec<_>=first.tokens.iter().filter_map(|(_,r)|id_at_report_member(&session,r)).collect();
            let expected_ids:Vec<_>=expected["expected_relations_from_source"].as_array().unwrap()
                .iter().map(|v|v.as_str().unwrap()).collect();
            assert_eq!(resolved,expected_ids);
            assert_eq!(first.tokens.iter().filter(|(_,r)|*r==Resolution::Missing).count(),
                expected["unresolved_missing_in_this_archive"].as_array().unwrap().len());
            assert_eq!(first.tokens.last().unwrap().1,Resolution::UnsupportedValue);
            assert_eq!(original,session.original_csv(csv).unwrap());
        }
    }

    #[test]
    fn headers_only_csv_is_empty_preview_but_cannot_create_notes() {
        use std::io::{Cursor, Write};
        let bytes = b"Name,Page ID,Relation\r\n";
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("filtered.csv", options).unwrap();
        zip.write_all(bytes).unwrap();
        zip.start_file("Page 11111111111111111111111111111111.md", options).unwrap();
        zip.write_all(b"# Page\n").unwrap();
        let snapshot = zip.finish().unwrap().into_inner();
        let session = NotionPreviewSession::from_snapshot(&snapshot).unwrap();
        let csv = session.csv_choices().next().unwrap();
        let approved: Vec<_> = session.page_choices().cloned().collect();
        let scope = Scope { parts: &[Part::Outer], ancestor: &[] };
        let (header, count) = session.csv_header_and_count(csv).unwrap();
        assert_eq!(header, ["Name", "Page ID", "Relation"]);
        assert_eq!(count, 0);
        assert!(session.preview(selection(csv, &approved, scope)).unwrap().rows.is_empty());
        assert_eq!(session.original_csv(csv).unwrap(), bytes);
        assert!(super::super::csv_table::parse_csv(bytes).is_err());
    }

    #[test]
    fn preview_keeps_blank_first_field_without_treating_it_as_a_note_title() {
        use std::io::{Cursor, Write};
        let bytes = b"Name,Page ID,Relation\n,11111111111111111111111111111111,\"https://www.notion.so/22222222222222222222222222222222, 22222222222222222222222222222222\"\n";
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, contents) in [
            ("filtered.csv", bytes.as_slice()),
            ("Same 11111111111111111111111111111111.md", b"# Same\n".as_slice()),
            ("Same 22222222222222222222222222222222.md", b"# Same\n".as_slice()),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(contents).unwrap();
        }
        let snapshot = zip.finish().unwrap().into_inner();
        let session = NotionPreviewSession::from_snapshot(&snapshot).unwrap();
        let csv = session.csv_choices().next().unwrap();
        let approved: Vec<_> = session.page_choices().cloned().collect();
        let scope = Scope { parts: &[Part::Outer], ancestor: &[] };
        let report = session.preview(selection(csv, &approved, scope)).unwrap();
        assert_eq!(id_at_report_member(&session, &report.rows[0].source).as_deref(),
            Some("11111111111111111111111111111111"));
        assert_eq!(id_at_report_member(&session, &report.rows[0].tokens[0].1).as_deref(),
            Some("22222222222222222222222222222222"));
        assert_eq!(report.rows[0].tokens[1].1, Resolution::UnsupportedValue);
        assert_eq!(session.original_csv(csv).unwrap(), bytes);
        assert!(super::super::csv_table::parse_csv(bytes).is_err());
    }

    #[test]
    fn original_text_reads_exact_zip_parts_without_inventing_a_relation_schema() {
        use std::io::{Cursor, Write};

        fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (path, data) in entries {
                writer.start_file(*path, zip::write::SimpleFileOptions::default()).unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap().into_inner()
        }

        let default = "\u{feff}名稱,關聯\r\n甲,同名\r\n";
        let filtered = "名稱,關聯\n甲,\"無法解析，但原文仍能檢視";
        let first = zip(&[("研究_all.csv", default.as_bytes())]);
        let second = zip(&[("研究_all.csv", filtered.as_bytes())]);
        let outer = zip(&[("part-A.zip", &first), ("part-B.zip", &second)]);
        let session = NotionPreviewSession::from_snapshot(&outer).unwrap();
        assert!(session.page_choices().next().is_none());
        let picks: Vec<_> = session.csv_choices().collect();
        assert_eq!(picks.len(), 2);
        assert_eq!(picks[0].member.part.as_deref(), Some("part-A.zip"));
        assert_eq!(picks[1].member.part.as_deref(), Some("part-B.zip"));
        assert_eq!(picks[0].member.path, picks[1].member.path);
        for (pick, expected) in picks.iter().zip([default, filtered]) {
            let original = session.original_text(pick).unwrap();
            assert_eq!(original.page_count(), 1);
            let page = original.page(1).unwrap();
            assert_eq!(page.text, expected);
            assert_eq!(page.bytes, 0..expected.len());
            assert_eq!(session.original_csv(pick).unwrap(), expected.as_bytes());
        }
        assert_eq!(session.csv_header_and_count(picks[0]).unwrap(), (vec!["名稱".into(), "關聯".into()], 1));
        assert!(session.csv_header_and_count(picks[1]).is_err());
        let replacement = zip(&[("part-A.zip", &second), ("part-B.zip", &first)]);
        assert!(NotionPreviewSession::from_snapshot(&replacement).unwrap()
            .original_text(picks[0]).is_err());
    }

    #[test]
    fn original_text_pages_preserve_utf8_crlf_and_escape_invisible_characters() {
        let mut input = "a".repeat(RAW_PAGE_BYTES - 1);
        input.push_str("泥");
        input.push_str(&"b".repeat(RAW_PAGE_BYTES - 4));
        input.push_str("\r\n\u{202e}\t\\原文\n");
        let original = CsvText::new(&input);
        assert_eq!(original.page_count(), 3);
        assert_eq!(original.page(1).unwrap().bytes, 0..RAW_PAGE_BYTES - 1);
        assert_eq!(original.page(2).unwrap().text, format!("泥{}", "b".repeat(RAW_PAGE_BYTES - 4)));
        let last = original.page(3).unwrap();
        assert_eq!(last.text, "\r\n\u{202e}\t\\原文\n");
        assert_eq!(last.display_text(), "\\r\n\\u{202e}\\t\\\\原文\n");
        assert!(original.page(0).is_none());
        assert!(original.page(4).is_none());
        let complete: String = (1..=original.page_count()).map(|i| original.page(i).unwrap().text).collect();
        assert_eq!(complete, input);
    }

    #[test]
    fn stale_selection_scope_and_explicit_mapping_fail_closed_without_editing_original() {
        let a=NotionPreviewSession::open(&Path::new(FIXTURE).join("notion-synthetic-default.zip")).unwrap();
        let b=NotionPreviewSession::open(&Path::new(FIXTURE).join("notion-synthetic-filtered.zip")).unwrap();
        let a_csv=a.csv_choices().next().unwrap();
        let b_csv=b.csv_choices().next().unwrap();
        let pages:Vec<_>=a.page_choices().cloned().collect();
        let scope=Scope{parts:&[PART],ancestor:&[]};
        let original=a.original_csv(a_csv).unwrap().to_vec();
        assert!(b.preview(selection(a_csv,&[],scope)).is_err());
        assert!(b.preview(selection(b_csv,&pages,scope)).is_err());
        assert!(a.preview(Selection{relation_column:1,..selection(a_csv,&pages,scope)}).is_err());
        let duplicated=vec![pages[0].clone(),pages[0].clone()];
        assert!(a.preview(selection(a_csv,&duplicated,scope)).is_err());
        let off_scope=Scope{parts:&[relation::Part::Outer],ancestor:&[]};
        let report=a.preview(Selection{target:off_scope,..selection(a_csv,&pages,scope)}).unwrap();
        assert_eq!(report.rows[0].tokens[0].1,Resolution::OutOfScope);
        let report=a.preview(selection(a_csv,&[],scope)).unwrap();
        assert_eq!(report.rows[0].source,Resolution::Missing);
        assert!(report.rows[0].tokens.iter().all(|(_,r)|!matches!(r,Resolution::Candidate(_))));
        assert_eq!(original,a.original_csv(a_csv).unwrap());
    }
}
