//! Preview-only Notion relation migration. Pure, offline, no persistent writes.
//! Consume a ZIP reader's verified part/path registry and an explicitly confirmed
//! source-ID and relation column mapping. Notion CSVs do not universally supply this schema.
//! No plan may be committed without rechecking its archive hash, mappings and active local IDs.

use std::collections::{BTreeMap, BTreeSet};

const MAX_ORIGINS: usize = 15_000;
const MAX_CELL_BYTES: usize = 4096;
const MAX_VALUES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Part<'a> { Outer, Member(&'a str) }

#[derive(Clone, Copy, Debug)]
pub struct VerifiedPage<'a> {
    pub import_root: i64,
    pub part: Part<'a>,
    pub archive_path: &'a [&'a str], // validated logical path, not a string prefix
    pub page_id: &'a str, // only a verified page identity, never the CSV filename ID
    pub local_note: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    pub import_root: i64,
    pub allowed_parts: &'a [Part<'a>],
    pub ancestor: &'a [&'a str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lookup {
    Found(i64), InvalidId, Missing, OutOfScope, Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellEncoding { Single, CommaSpaceSeparated }

/// Construct only after the caller has explicit column mapping evidence.
#[derive(Clone, Copy, Debug)]
pub struct ExplicitColumn<'a> {
    pub relation_name: &'a str,
    pub encoding: CellEncoding,
    pub source: Scope<'a>,
    pub target: Scope<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Resolved(i64), Unresolved(Lookup), UnsupportedValue, DuplicateValue, SelfTarget,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Token<'a> { pub original: &'a str, pub outcome: Outcome }

#[derive(Debug, Eq, PartialEq)]
pub struct Preview<'a> {
    pub original_cell: &'a str,
    pub source: Lookup,
    pub tokens: Vec<Token<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanError { TooManyOrigins, InvalidOrigin, ConflictingOrigin, InvalidMapping, InvalidRowOrigin, OversizedCell, TooManyValues }

/// Snapshot coordinates, not a relation inferred from the CSV member name.
/// The adapter must compute both digests over already-read immutable bytes.
/// This module neither verifies hashes nor reads input paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CsvMemberIdentity<'a> {
    import_root: i64,
    archive_sha256: [u8; 32],
    part: Part<'a>,
    member_path: &'a [&'a str],
    member_sha256: [u8; 32],
}
impl<'a> CsvMemberIdentity<'a> {
    pub fn from_verified_snapshot(import_root:i64, archive_sha256:[u8;32], part:Part<'a>,
        member_path:&'a [&'a str], member_sha256:[u8;32]) -> Result<Self,PlanError> {
        if import_root<=0 || !part_ok(part) || !path_ok(member_path)
            || !member_path.last().is_some_and(|s|s.rsplit_once('.').is_some_and(|(_,e)|e.eq_ignore_ascii_case("csv"))) {
            return Err(PlanError::InvalidRowOrigin);
        }
        Ok(Self{import_root,archive_sha256,part,member_path,member_sha256})
    }
}

/// The immutable record is supplied by the parser of the member snapshot, never
/// constructed from a displayed/sorted row or mutable edit. Ordinal is 1-based
/// among data records, excluding header; quoted multi-line records count once.
#[derive(Debug)]
pub struct VerifiedCsvRow<'a> {
    member: CsvMemberIdentity<'a>,
    data_record_ordinal: usize,
    decoded_fields: &'a [&'a str],
}
impl<'a> VerifiedCsvRow<'a> {
    pub fn from_verified_record(member:CsvMemberIdentity<'a>, data_record_ordinal:usize,
        decoded_fields:&'a [&'a str]) -> Result<Self,PlanError> {
        if !(1..=5000).contains(&data_record_ordinal) || decoded_fields.is_empty() || decoded_fields.len()>128 {
            return Err(PlanError::InvalidRowOrigin);
        }
        Ok(Self{member,data_record_ordinal,decoded_fields})
    }
}

/// Bind the user's approved column mapping to a specific archive and CSV member.
/// Columns are zero-based original CSV indexes; titles or header-name matches
/// cannot transfer a mapping to a different member or import root.
#[derive(Clone, Copy, Debug)]
pub struct BoundCsvMapping<'a> {
    pub member: CsvMemberIdentity<'a>,
    pub source_id_column: usize,
    pub relation_column: usize,
    pub column: ExplicitColumn<'a>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct VerifiedRowPreview<'a> {
    pub member: CsvMemberIdentity<'a>,
    pub data_record_ordinal: usize,
    pub source_id_column: usize,
    pub relation_column: usize,
    pub original_source_field: &'a str,
    pub preview: Preview<'a>,
}


fn uuid(input: &str) -> Option<String> {
    let mut out = String::with_capacity(32);
    for (i,b) in input.bytes().enumerate() {
        if input.len()==36 && matches!(i,8|13|18|23) {
            if b!=b'-' { return None; }
        } else {
            if !b.is_ascii_hexdigit() { return None; }
            out.push(b.to_ascii_lowercase() as char);
        }
    }
    (out.len()==32).then_some(out)
}

fn path_ok(path: &[&str]) -> bool {
    path.len() <= 64 && path.iter().all(|p| !matches!(*p,""|"."|"..")
        && !p.bytes().any(|b| b.is_ascii_control() || matches!(b,b'/'|b'\\'|b':')))
}
fn part_ok(part: Part<'_>) -> bool {
    match part { Part::Outer => true, Part::Member(name) => path_ok(&[name]) }
}

/// Accept only exact canonical direct page URLs. Deliberately do not infer IDs from
/// redirects, arbitrary slugs, percent encoding, query strings, fragments or other hosts.
fn href_id(value: &str) -> Option<String> {
    let id=value.strip_prefix("https://www.notion.so/")
        .or_else(||value.strip_prefix("https://notion.so/"))?;
    uuid(id)
}

pub struct IdentityIndex<'a> {
    pages: BTreeMap<String, Vec<VerifiedPage<'a>>>,
    collisions: BTreeSet<i64>,
}

impl<'a> IdentityIndex<'a> {
    pub fn new(origins: &[VerifiedPage<'a>]) -> Result<Self,PlanError> {
        if origins.len()>MAX_ORIGINS { return Err(PlanError::TooManyOrigins); }
        let mut pages: BTreeMap<String,Vec<_>>=BTreeMap::new();
        let mut exact_origins=BTreeSet::new();
        let mut seen=BTreeSet::new();
        let mut collisions=BTreeSet::new();
        for p in origins {
            let id=uuid(p.page_id).ok_or(PlanError::InvalidOrigin)?;
            if p.import_root<=0 || p.local_note<=0 || p.archive_path.is_empty()
                || !path_ok(p.archive_path) || !part_ok(p.part) {
                return Err(PlanError::InvalidOrigin);
            }
            if !exact_origins.insert((p.import_root,p.part,p.archive_path)) {
                // The same physical archive member cannot attest two page identities.
                return Err(PlanError::ConflictingOrigin);
            }
            if !seen.insert(p.local_note) { collisions.insert(p.local_note); }
            pages.entry(id).or_default().push(*p);
        }
        Ok(Self{pages,collisions})
    }

    fn lookup(&self, id: &str, scope: Scope<'_>) -> Lookup {
        let Some(entries)=self.pages.get(id) else { return Lookup::Missing; };
        let mut accepted=entries.iter().filter(|p|p.import_root==scope.import_root
            && scope.allowed_parts.contains(&p.part) && p.archive_path.starts_with(scope.ancestor));
        let Some(page)=accepted.next() else { return Lookup::OutOfScope; };
        if self.collisions.contains(&page.local_note) || accepted.next().is_some() {
            Lookup::Ambiguous
        } else { Lookup::Found(page.local_note) }
    }

    /// Read-only integration entrypoint. Preserve original ZIP/CSV bytes in the
    /// attachment store before converting anything. A rejected preview does not
    /// change or remove those bytes. A future commit must rerun after any remap.
    pub fn preview_verified_row<'v>(&self, mapping:BoundCsvMapping<'_>, row:&VerifiedCsvRow<'v>)
        -> Result<VerifiedRowPreview<'v>,PlanError> {
        if row.member!=mapping.member || row.member.import_root!=mapping.column.source.import_root
            || mapping.source_id_column==mapping.relation_column {
            return Err(PlanError::InvalidMapping);
        }
        let source=*row.decoded_fields.get(mapping.source_id_column).ok_or(PlanError::InvalidMapping)?;
        let raw=*row.decoded_fields.get(mapping.relation_column).ok_or(PlanError::InvalidMapping)?;
        Ok(VerifiedRowPreview{member:row.member,data_record_ordinal:row.data_record_ordinal,
            source_id_column:mapping.source_id_column,relation_column:mapping.relation_column,
            original_source_field:source,preview:self.preview(mapping.column,source,raw)?})
    }

    pub fn preview<'v>(&self, column: ExplicitColumn<'_>, source_id: &str,
        raw: &'v str) -> Result<Preview<'v>,PlanError> {
        let valid=|s:Scope<'_>|s.import_root>0 && !s.allowed_parts.is_empty()
            && s.allowed_parts.iter().copied().all(part_ok) && path_ok(s.ancestor);
        if column.relation_name.trim().is_empty() || column.relation_name.len()>256
            || column.relation_name.chars().any(char::is_control)
            || !valid(column.source) || !valid(column.target)
            || column.source.import_root!=column.target.import_root {
            return Err(PlanError::InvalidMapping);
        }
        if raw.len()>MAX_CELL_BYTES { return Err(PlanError::OversizedCell); }
        let source=uuid(source_id).map_or(Lookup::InvalidId, |id|self.lookup(&id,column.source));
        let mut tokens=Vec::new();
        let mut seen=BTreeSet::new();
        // This separator is an explicit caller choice, not assumed by CSV format.
        // The CSV quoting split has already happened in the caller.
        let delimiter=match column.encoding {
            CellEncoding::Single => "\0", CellEncoding::CommaSpaceSeparated => ", ",
        };
        if !raw.is_empty() {
            let fragments=raw.split(delimiter);
            // If the input contains NUL, do not let a valid URL segment become a link.
            if raw.contains('\0') {
                tokens.push(Token { original:raw, outcome:Outcome::UnsupportedValue });
            } else {
                for original in fragments {
                    if tokens.len()>=MAX_VALUES { return Err(PlanError::TooManyValues); }
                    let outcome=match (href_id(original.trim()),source) {
                        (None,_)=>Outcome::UnsupportedValue,
                        (Some(id), Lookup::Found(from))=>match self.lookup(&id,column.target) {
                            Lookup::Found(to) if from==to=>Outcome::SelfTarget,
                            Lookup::Found(to) if !seen.insert(to)=>Outcome::DuplicateValue,
                            Lookup::Found(to)=>Outcome::Resolved(to),
                            unresolved=>Outcome::Unresolved(unresolved),
                        },
                        (Some(_),unresolved)=>Outcome::Unresolved(unresolved),
                    };
                    tokens.push(Token{original,outcome});
                }
            }
        }
        Ok(Preview{original_cell:raw,source,tokens})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A:&str="11111111111111111111111111111111";
    const B:&str="22222222222222222222222222222222";
    const C:&str="33333333333333333333333333333333";
    const SRC:&str="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const P:Part<'_>=Part::Member("Part-1.zip");
    const Q:Part<'_>=Part::Member("Part-2.zip");
    const SCOPED:Scope<'_>=Scope{import_root:10,allowed_parts:&[P],ancestor:&["頁面"]};
    const BOTH:Scope<'_>=Scope{import_root:10,allowed_parts:&[P,Q],ancestor:&["頁面"]};
    fn row<'a>(page_id:&'a str, note:i64, import_root:i64, part:Part<'a>,
        archive_path:&'a [&'a str])->VerifiedPage<'a>{
        VerifiedPage {page_id,local_note:note,import_root,part,archive_path}
    }
    fn col(target:Scope<'static>)->ExplicitColumn<'static>{
        ExplicitColumn{relation_name:"明確指定關聯",encoding:CellEncoding::CommaSpaceSeparated,
            source:SCOPED,target}
    }
    fn url(id:&str)->String{format!("https://www.notion.so/{id}")}
    fn states(p:&Preview<'_>)->Vec<Outcome>{p.tokens.iter().map(|t|t.outcome).collect()}

    #[test]
    fn same_title_mixed_values_keep_raw_and_do_not_join_off_scope() {
        let idx=IdentityIndex::new(&[
            row(SRC,1,10,P,&["頁面","來源"]),
            row(A,2,10,P,&["頁面","同名 11111111111111111111111111111111"]),
            row(B,3,10,P,&["頁面","同名 22222222222222222222222222222222"]),
            row(B,4,99,P,&["頁面","其他匯入"]),
        ]).unwrap();
        let bad=format!("https://www.notion.so.invalid/{A}");
        let value=format!(" {}, {}, {}, {bad}, {} ",url(B),url(A),url(C),url(B));
        let p=idx.preview(col(SCOPED),SRC,&value).unwrap();
        assert_eq!(p.original_cell,value);
        assert_eq!(p.tokens[0].original,format!(" {}",url(B)));
        assert_eq!(p.source,Lookup::Found(1));
        assert_eq!(states(&p),vec![Outcome::Resolved(3),Outcome::Resolved(2),
            Outcome::Unresolved(Lookup::Missing),Outcome::UnsupportedValue,Outcome::DuplicateValue]);
    }

    #[test]
    fn parts_and_segment_ancestors_distinguish_ambiguous_and_outside() {
        let idx=IdentityIndex::new(&[
            row(SRC,1,10,P,&["頁面","來源"]),
            row(A,2,10,P,&["頁面","甲"]),
            row(A,3,10,Q,&["頁面","複本"]),
            row(B,4,10,P,&["頁面外","乙"]),
            row(C,5,99,P,&["頁面","舊批次"]),
        ]).unwrap();
        let value=format!("{}, {}, {}, {}",url(A),url(B),url(C),url(SRC));
        assert_eq!(states(&idx.preview(col(SCOPED),SRC,&value).unwrap()),
            vec![Outcome::Resolved(2),Outcome::Unresolved(Lookup::OutOfScope),
                 Outcome::Unresolved(Lookup::OutOfScope),Outcome::SelfTarget]);
        assert_eq!(idx.preview(col(BOTH),SRC,&value).unwrap().tokens[0].outcome,
            Outcome::Unresolved(Lookup::Ambiguous));
    }

    #[test]
    fn invalid_or_ambiguous_source_never_suggests_edges() {
        let idx=IdentityIndex::new(&[row(SRC,1,10,P,&["頁面","來源"]),
            row(SRC,2,10,P,&["頁面","來源複本"]),row(A,3,10,P,&["頁面","目標"])]).unwrap();
        assert_eq!(idx.preview(col(SCOPED),SRC,&url(A)).unwrap().tokens[0].outcome,
            Outcome::Unresolved(Lookup::Ambiguous));
        assert_eq!(idx.preview(col(SCOPED),"同名",&url(A)).unwrap().source, Lookup::InvalidId);
        assert!(idx.preview(col(SCOPED),SRC,&"a".repeat(4097)).is_err());
        assert!(IdentityIndex::new(&[row(A,3,10,P,&["頁面",".."])]).is_err());
    }

    #[test]
    fn explicit_single_encoding_does_not_split_and_uuid_hyphens_are_strict() {
        let idx=IdentityIndex::new(&[row(SRC,1,10,P,&["頁面","來源"]),
            row(A,2,10,P,&["頁面","目標"])]).unwrap();
        let one=ExplicitColumn{encoding:CellEncoding::Single,..col(SCOPED)};
        let plus=format!("{}, {}",url(A),url(B));
        assert_eq!(states(&idx.preview(one,SRC,&plus).unwrap()),vec![Outcome::UnsupportedValue]);
        assert_eq!(states(&idx.preview(one,SRC,&url("11111111-1111-1111-1111-111111111111")).unwrap()),
            vec![Outcome::Resolved(2)]);
        assert_eq!(states(&idx.preview(one,SRC,&url("11111111_1111-1111-1111-111111111111")).unwrap()),
            vec![Outcome::UnsupportedValue]);
    }

    #[test]
    fn cannot_attest_two_page_ids_from_one_exact_archive_member() {
        let path=&["頁面","同一檔案.md"];
        assert!(matches!(IdentityIndex::new(&[row(A,1,10,P,path),row(B,2,10,P,path)]),
            Err(PlanError::ConflictingOrigin)));
        assert!(matches!(IdentityIndex::new(&[row(A,1,10,P,path),row(A,1,10,P,path)]),
            Err(PlanError::ConflictingOrigin)));
        // Archive path text alone is not identity: separate inner parts remain separate.
        assert!(IdentityIndex::new(&[row(A,1,10,P,path),row(B,2,10,Q,path)]).is_ok());
    }

    #[test]
    fn csv_preview_is_bound_to_snapshot_part_path_and_parsed_record() {
        let idx=IdentityIndex::new(&[row(SRC,1,10,P,&["頁面","來源"]),
            row(A,2,10,P,&["頁面","目標"])]).unwrap();
        let path=&["匯出","資料庫.csv"];
        let identity=|archive,part,member,path|CsvMemberIdentity::from_verified_snapshot(
            10,[archive;32],part,path,[member;32]).unwrap();
        let member=identity(1,P,2,path);
        let raw=format!(" {}, ordinary URL ",url(A));
        let fields=&["同名",SRC,raw.as_str(),"非關聯"];
        let row=VerifiedCsvRow::from_verified_record(member,17,fields).unwrap();
        let mapping=BoundCsvMapping{member,source_id_column:1,relation_column:2,column:col(SCOPED)};
        let out=idx.preview_verified_row(mapping,&row).unwrap();
        assert_eq!((out.member,out.data_record_ordinal,out.relation_column),(member,17,2));
        assert_eq!(out.original_source_field,SRC);
        assert_eq!(out.preview.original_cell,raw);
        assert_eq!(states(&out.preview),vec![Outcome::Resolved(2),Outcome::UnsupportedValue]);
        for unapproved in [identity(2,P,2,path),identity(1,Q,2,path),
            identity(1,P,3,path),identity(1,P,2,&["另一匯出","資料庫.csv"])] {
            assert_eq!(idx.preview_verified_row(BoundCsvMapping{member:unapproved,..mapping},&row),
                Err(PlanError::InvalidMapping));
        }
        assert!(VerifiedCsvRow::from_verified_record(member,0,fields).is_err());
        assert_eq!(idx.preview_verified_row(BoundCsvMapping{source_id_column:2,..mapping},&row),
            Err(PlanError::InvalidMapping));
    }
}
