//! Fresh, local aggregates over explicit directed note relations.
use super::{Notebook, Result};
use rusqlite::params;

const MAX_RELATIONS: usize = 1000;
fn label(value: &str) -> Result<&str> {
    if value.contains(['\0','\r','\n','\t']) { return Err("名稱不能包含控制字元。".into()); }
    let value = value.trim();
    if value.is_empty() || value.len()>256 { return Err("名稱需有內容且最多 256 位元組。".into()); }
    Ok(value)
}
impl Notebook {
    /// Count active targets in this explicitly named relation. Source must be active.
    pub fn rollup_count(&self, source_id: i64, relation: &str) -> Result<usize> {
        let relation=label(relation)?;
        let mut s=self.connection.prepare_cached(
            "SELECT count(n.id) FROM notes source
             LEFT JOIN note_relations r ON r.source_id=source.id AND r.name=?2
             LEFT JOIN notes n ON n.id=r.target_id AND n.trash_root IS NULL
             WHERE source.id=?1 AND source.trash_root IS NULL GROUP BY source.id")?;
        let mut rows=s.query(params![source_id,relation])?;
        let count: i64=rows.next()?.ok_or("找不到可使用的來源筆記。")?.get(0)?;
        if count<0 || count as usize>MAX_RELATIONS { return Err("關聯超過計算上限。".into()); }
        Ok(count as usize)
    }
    /// Sum text properties parsed as finite f64. Missing or nonnumeric values are errors.
    /// All targets and properties are read from one SQLite statement snapshot.
    pub fn rollup_sum(&self, source_id: i64, relation: &str, property: &str) -> Result<f64> {
        let relation=label(relation)?; let property=label(property)?;
        let mut s=self.connection.prepare_cached(
            "SELECT n.id,p.value FROM notes source
             LEFT JOIN note_relations r ON r.source_id=source.id AND r.name=?2
             LEFT JOIN notes n ON n.id=r.target_id AND n.trash_root IS NULL
             LEFT JOIN note_properties p ON p.note_id=n.id AND p.key=?3
             WHERE source.id=?1 AND source.trash_root IS NULL
             ORDER BY n.id LIMIT 1002")?;
        let mut rows=s.query(params![source_id,relation,property])?;
        let mut saw_source=false; let mut count=0; let mut sum=0.0f64;
        while let Some(row)=rows.next()? {
            saw_source=true;
            let id: Option<i64>=row.get(0)?;
            let Some(id) = id else { continue; };
            count+=1;
            if count>MAX_RELATIONS { return Err("關聯超過計算上限。".into()); }
            let value: Option<String>=row.get(1)?;
            let value=value.ok_or_else(||format!("關聯「{relation}」的筆記 #{id} 缺少屬性「{property}」。"))?;
            let n: f64=value.trim().parse().map_err(|_|format!("關聯「{relation}」的筆記 #{id} 的「{property}」需要數字。"))?;
            if !n.is_finite() { return Err(format!("筆記 #{id} 的「{property}」超出公式數值範圍。").into()); }
            sum+=n;
            if !sum.is_finite() { return Err("加總結果超出範圍。".into()); }
        }
        if !saw_source { return Err("找不到可使用的來源筆記。".into()); }
        Ok(sum)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime,UNIX_EPOCH};
    #[test]
    fn section_time_is_live_with_trash_and_backup() -> Result<()> {
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir=std::env::temp_dir().join(format!("paperroom-rollup-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&dir)?;
        let mut db=Notebook::open(&dir.join("notes.sqlite3"),false)?;
        let project=db.add(None,"段落整理","")?;
        let first=db.add(None,"第一節","")?;
        let second=db.add(None,"第二節","")?;
        db.set_property(first,"分鐘",Some("5"))?;
        db.set_property(second,"分鐘",Some("7"))?;
        db.set_relation(project,"章節",first,true)?;
        db.set_relation(project,"章節",second,true)?;
        assert_eq!(db.rollup_count(project,"章節")?,2);
        assert_eq!(db.rollup_sum(project,"章節","分鐘")?,12.0);
        db.set_property(second,"分鐘",Some("13"))?;
        assert_eq!(db.rollup_sum(project,"章節","分鐘")?,18.0);
        db.trash(second)?;
        assert_eq!(db.rollup_count(project,"章節")?,1);
        assert_eq!(db.rollup_sum(project,"章節","分鐘")?,5.0);
        db.restore(second)?;
        assert_eq!(db.rollup_sum(project,"章節","分鐘")?,18.0);
        let backup=dir.join("backup.sqlite3"); db.backup(&backup)?;
        let restored=Notebook::open(&backup,false)?;
        assert_eq!(restored.rollup_sum(project,"章節","分鐘")?,18.0);
        db.set_property(first,"分鐘",Some("不是數字"))?;
        assert!(db.rollup_sum(project,"章節","分鐘").is_err());
        db.set_property(first,"分鐘",None)?;
        assert!(db.rollup_sum(project,"章節","分鐘").is_err());
        assert_eq!(db.rollup_count(project,"無此關聯")?,0);
        assert_eq!(db.rollup_sum(project,"無此關聯","分鐘")?,0.0);
        db.trash(project)?;
        assert!(db.rollup_count(project,"章節").is_err());
        assert!(db.rollup_sum(project,"章節","分鐘").is_err());
        drop(restored); drop(db); std::fs::remove_dir_all(dir)?; Ok(())
    }
}
