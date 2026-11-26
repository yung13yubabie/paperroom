//! Replace a complete card selection on its parent with a single child-board tile.
use super::*;
use std::collections::BTreeSet;

#[derive(PartialEq,Eq)]
struct CardStamp {
    token:String, width:i64, height:i64, group:Option<i64>, title:Option<String>, annotation:Option<i64>,
}
#[derive(PartialEq,Eq)]
pub(crate) struct SelectionChild {
    parent:i64, positions:Vec<BoardPosition>, cards:Vec<CardStamp>, edges:BTreeSet<(i64,i64)>,
}

fn snapshot(db:&Connection,parent:i64,positions:&[BoardPosition])->Result<SelectionChild> {
    if positions.is_empty() || positions.len()>128 {return Err("請選取 1 至 128 張卡片。".into());}
    let mut notes=BTreeSet::new();
    let mut cards=Vec::new();
    let mut stmt=db.prepare("SELECT i.token,c.width,c.height,g.id,g.title,s.annotation_id
        FROM board_card_geometry c JOIN notes n ON n.id=c.note_id JOIN note_link_identities i ON i.note_id=n.id
        LEFT JOIN board_group_cards m ON m.board_id=c.board_id AND m.note_id=n.id
        LEFT JOIN board_groups g ON g.board_id=m.board_id AND g.id=m.group_id
        LEFT JOIN board_group_sources s ON s.board_id=g.board_id AND s.group_id=g.id
        WHERE c.board_id=?1 AND c.note_id=?2 AND c.x=?3 AND c.y=?4 AND n.trash_root IS NULL")?;
    for position in positions {
        let BoardItem::Card(note)=position.item else{return Err("此操作只接受筆記、圖片與 PDF 摘錄卡。".into());};
        if !notes.insert(note){return Err("卡片選取重複。".into());}
        cards.push(stmt.query_row(params![parent,note,position.x,position.y],|r|Ok(CardStamp{token:r.get(0)?,width:r.get(1)?,height:r.get(2)?,
            group:r.get(3)?,title:r.get(4)?,annotation:r.get(5)?})).optional()?.ok_or("來源卡片已變更，請重新選取。")?);
    }
    let groups=cards.iter().filter_map(|s|s.group).collect::<BTreeSet<_>>();
    let mut members=db.prepare("SELECT note_id FROM board_group_cards WHERE board_id=?1 AND group_id=?2")?;
    for group in groups{
        for member in members.query_map(params![parent,group],|r|r.get::<_,i64>(0))? {
            if !notes.contains(&member?){return Err("請選取整個分區後再收進子白板。".into());}
        }
    }
    let mut edges=BTreeSet::new();
    let mut links=db.prepare("SELECT from_note,to_note FROM board_edges WHERE board_id=?1")?;
    for pair in links.query_map([parent],|r|Ok((r.get(0)?,r.get(1)?)))? {
        let (from,to)=pair?;
        if notes.contains(&from)!=notes.contains(&to){return Err("請選取連線兩端，再收進子白板。".into());}
        if notes.contains(&from){edges.insert((from,to));}
    }
    Ok(SelectionChild{parent,positions:positions.to_vec(),cards,edges})
}

/// Describes the active child placements and the exact tile visible at preview time.
#[derive(PartialEq, Eq)]
pub(crate) struct ChildUnfold {
    selection: SelectionChild,
    parent: i64,
    tile: i64,
    version: i64,
    tile_origin: (i32, i32),
    tile_destination: (i32, i32),
}
impl ChildUnfold {
    pub(crate) fn count(&self) -> usize { self.selection.positions.len() }
}

fn child_unfold(db: &Connection, child: i64) -> Result<ChildUnfold> {
    let (parent, tile, version, x, y): (i64, i64, i64, i32, i32) = db.query_row(
        "SELECT board_id,id,version,x,y FROM board_objects WHERE child_id=?1", [child],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).optional()?.ok_or("請先開啟有上層入口的子白板。")?;
    let mut cards = db.prepare("SELECT c.note_id,c.x,c.y FROM board_cards c JOIN notes n
        ON n.id=c.note_id WHERE c.board_id=?1 AND n.trash_root IS NULL ORDER BY c.note_id")?;
    let positions = cards.query_map([child], |r| Ok(BoardPosition {
        item: BoardItem::Card(r.get(0)?), x: r.get(1)?, y: r.get(2)?, version: None,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if positions.is_empty() { return Err("子白板目前沒有可展回的卡片。".into()); }
    let selection = snapshot(db, child, &positions)?;
    // Keep the original tile if possible. Otherwise find space around the future
    // placements before touching any rows, preserving the tile and its identity.
    let destination = db.query_row("WITH RECURSIVE grid(n,x,y) AS (
        VALUES(0,16,16) UNION ALL SELECT n+1,16+((n+1)%2)*260,16+((n+1)/2)*135 FROM grid WHERE n<737
    ), slots(n,x,y) AS (SELECT -1,?4,?5 UNION ALL SELECT * FROM grid),
    occupied(x,y,w,h) AS (
        SELECT x,y,width,height FROM board_card_geometry WHERE board_id=?1
        UNION ALL SELECT c.x,c.y,c.width,c.height FROM board_card_geometry c JOIN notes n ON n.id=c.note_id
            WHERE c.board_id=?2 AND n.trash_root IS NULL
        UNION ALL SELECT x,y,232,114 FROM board_objects WHERE board_id=?1 AND id<>?3
    ) SELECT x,y FROM slots s WHERE NOT EXISTS(SELECT 1 FROM occupied o
        WHERE o.x<s.x+232 AND o.x+o.w>s.x AND o.y<s.y+114 AND o.y+o.h>s.y)
        ORDER BY n LIMIT 1", params![parent,child,tile,x,y], |r| Ok((r.get(0)?,r.get(1)?)))
        .optional()?.ok_or("上層白板沒有空位可保留子白板入口。")?;
    Ok(ChildUnfold {selection, parent, tile, version, tile_origin:(x,y), tile_destination:destination})
}

impl Notebook {
    pub(crate) fn plan_child_unfold(&self, child: i64) -> Result<ChildUnfold> {
        let tx = self.connection.unchecked_transaction()?;
        let plan = child_unfold(&tx, child)?;
        tx.commit()?;
        Ok(plan)
    }

    pub(crate) fn unfold_child(&self, plan: &ChildUnfold) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let child = plan.selection.parent;
        if child_unfold(&tx,child)? != *plan {
            return Err("卡片、連線、分區或上層入口已變更，請重新預覽。".into());
        }
        if plan.tile_origin != plan.tile_destination {
            let (x,y) = plan.tile_destination;
            if tx.execute("UPDATE board_objects SET x=?1,y=?2,version=version+1
                WHERE id=?3 AND board_id=?4 AND child_id=?5 AND version=?6",
                params![x,y,plan.tile,plan.parent,child,plan.version])? != 1 {
                return Err("上層入口已變更，請重新預覽。".into());
            }
        }
        Self::move_cards_on(&tx, child, plan.parent, &plan.selection.positions, 0, 0)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn plan_selection_child(&self,parent:i64,positions:&[BoardPosition])->Result<SelectionChild> {
        let tx=self.connection.unchecked_transaction()?;
        let plan=snapshot(&tx,parent,positions)?;
        tx.commit()?;
        Ok(plan)
    }

    pub(crate) fn commit_selection_child(&self,plan:&SelectionChild,title:&str)->Result<i64>{
        let tx=Transaction::new_unchecked(&self.connection,TransactionBehavior::Immediate)?;
        if snapshot(&tx,plan.parent,&plan.positions)?!=*plan{return Err("卡片、分區或連線已變更，請重新選取。".into());}
        if board_depth(&tx,plan.parent,None)? >= MAX_DEPTH{return Err("白板階層最多 16 層。".into());}
        let child=super::super::boards::create_board_on(&tx,title)?;
        Self::move_cards_on(&tx,plan.parent,child,&plan.positions,0,0)?;
        let first=plan.positions.iter().min_by_key(|p|(p.y,p.x)).ok_or("請先選取卡片。")?;
        let (x,y)=position(&tx,plan.parent,Some((first.x,first.y)))?;
        tx.execute("INSERT INTO board_objects(board_id,child_id,x,y) VALUES(?1,?2,?3,?4)",params![plan.parent,child,x,y])?;
        tx.commit()?;
        Ok(child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unfolding_restores_canonical_cards_and_keeps_the_tile_and_grandchild() -> Result<()> {
        let dir = super::super::super::tests::data_dir();
        let file = dir.join("original.db");
        let backup = dir.join("after-unfold.db");
        let pdf = dir.join("分鏡.pdf");
        let bytes = include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf");
        std::fs::write(&pdf,bytes)?;
        let mut db = Notebook::open(&file,false)?;
        let parent = db.create_board("上層")?;
        let other = db.create_board("並行專案")?;
        let a = db.add(None,"同名","甲")?;
        let b = db.add(None,"同名","乙")?;
        let attachment = db.import_attachment(a,&pdf)?;
        let annotation = db.add_pdf_annotation(attachment,1,"摘錄","")?;
        for (note,x) in [(a,16),(b,276)] {
            db.add_card_on(parent,note)?;
            db.move_card_on(parent,note,x,16)?;
        }
        db.add_card_on(other,a)?;
        db.add_edge_on(parent,a,b)?;
        let group = db.create_board_group(parent,"場景")?;
        db.set_board_group(parent,a,None,Some(group))?;
        db.set_board_group(parent,b,None,Some(group))?;
        db.set_board_group_source(parent,group,None,Some(annotation))?;
        let positions = [(a,16),(b,276)].map(|(n,x)|BoardPosition{item:BoardItem::Card(n),x,y:16,version:None});
        let child = db.commit_selection_child(&db.plan_selection_child(parent,&positions)?,"子層")?;
        let grandchild = db.new_child_board(child,"孫層")?;
        let text = db.add_board_text(child,"子板自己的文字")?;
        let tile = db.parent_board_tile(child)?.unwrap().1;
        let child_tile = db.parent_board_tile(grandchild)?.unwrap().1;
        let mut writer = Notebook::open(&file,false)?;
        let stale = db.plan_child_unfold(child)?;
        writer.remove_edge_on(child,a,b)?;
        assert!(db.unfold_child(&stale).is_err());
        writer.add_edge_on(child,a,b)?;

        // A shared placement in the destination is never overwritten or merged.
        writer.add_card_on(parent,a)?;
        let conflict = db.plan_child_unfold(child)?;
        let before = db.objects_on(parent)?.remove(0);
        assert!(db.unfold_child(&conflict).is_err());
        let after = db.objects_on(parent)?.remove(0);
        assert_eq!((before.id,before.x,before.y,before.version),(after.id,after.x,after.y,after.version));
        assert_eq!(db.cards_on(child)?.len(),2);
        writer.remove_card_on(parent,a)?;
        let plan = db.plan_child_unfold(child)?;
        assert_eq!(plan.count(),2);
        db.unfold_child(&plan)?;
        assert!(db.unfold_child(&plan).is_err());
        db.backup(&backup)?;
        drop(writer); drop(db);
        for copy in [&file,&backup] {
            let db = Notebook::open(copy,false)?;
            let positions = db.cards_on(parent)?.into_iter().map(|v|(v.id,v.x,v.y)).collect::<Vec<_>>();
            assert_eq!(positions,[(a,16,16),(b,276,16)]);
            assert!(db.cards_on(child)?.is_empty());
            assert_eq!(db.cards_on(other)?[0].id,a);
            let entrance = db.objects_on(parent)?.remove(0);
            assert_eq!((entrance.id,entrance.child,entrance.x,entrance.y,entrance.version),
                       (tile,Some(child),16,151,before.version+1));
            assert_eq!(db.child_board_from_tile(child,child_tile,grandchild)?.id,grandchild);
            assert!(db.objects_on(child)?.iter().any(|v|v.id==text && v.text=="子板自己的文字"));
            assert_eq!(db.edges_on(parent)?.len(),1);
            assert_eq!(db.live_board_group_source(parent,group,annotation)?.unwrap().attachment_id,attachment);
            assert_eq!(db.read_pdf_attachment(attachment)?.1,bytes.as_slice());
            assert!(!db.connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",[],|r|r.get::<_,bool>(0))?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn wrapping_selection_is_atomic_and_restores_the_same_placements_from_backup()->Result<()> {
        let dir=super::super::super::tests::data_dir();
        let file=dir.join("live.db");
        let backup=dir.join("copy.db");
        let pdf=dir.join("來源.pdf");
        std::fs::write(&pdf,include_bytes!("../tests/fixtures/storyboard-zh/剪輯範圍.pdf"))?;
        let mut db=Notebook::open(&file,false)?;
        let board=db.create_board("分鏡")?;
        let other=db.create_board("另一板")?;
        let a=db.add(None,"同名","甲正文")?;
        let b=db.add(None,"同名","乙正文")?;
        let decoy=db.add(None,"同名","其他")?;
        let attachment=db.import_attachment(a,&pdf)?;
        let annotation=db.add_pdf_annotation(attachment,1,"引文","附註")?;
        for (note,x,y) in [(a,400,100),(b,660,100),(decoy,16,16)] {
            db.add_card_on(board,note)?;db.move_card_on(board,note,x,y)?;
        }
        db.add_card_on(other,a)?;
        let positions=[(a,400),(b,660)].map(|(n,x)|BoardPosition{item:BoardItem::Card(n),x,y:100,version:None});
        let group=db.create_board_group(board,"共用分區")?;
        db.set_board_group(board,a,None,Some(group))?;
        db.set_board_group(board,b,None,Some(group))?;
        db.set_board_group_source(board,group,None,Some(annotation))?;
        db.add_edge_on(board,a,b)?;
        assert!(db.plan_selection_child(board,&positions[..1]).is_err());
        let plan=db.plan_selection_child(board,&positions)?;
        let before=db.list_boards()?.len();
        let writer=Notebook::open(&file,false)?;
        writer.remove_edge_on(board,a,b)?;
        assert!(db.commit_selection_child(&plan,"失效").is_err());
        assert_eq!(db.list_boards()?.len(),before);
        writer.add_edge_on(board,a,b)?;
        writer.add_edge_on(board,a,decoy)?;
        assert!(db.commit_selection_child(&plan,"跨界").is_err());
        writer.remove_edge_on(board,a,decoy)?;
        let plan=db.plan_selection_child(board,&positions)?;
        assert!(db.commit_selection_child(&plan,"另一板").is_err());
        writer.connection.execute_batch("CREATE TRIGGER simulate_tile_failure BEFORE INSERT ON board_objects
            WHEN new.child_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'synthetic tile failure'); END;")?;
        assert!(db.commit_selection_child(&plan,"不可殘留").is_err());
        assert_eq!(db.cards_on(board)?.len(),3);
        assert_eq!(db.list_boards()?.len(),before);
        assert_eq!(db.edges_on(board)?.len(),1);
        assert_eq!(db.groups_on(board)?[0].id,group);
        writer.connection.execute_batch("DROP TRIGGER simulate_tile_failure")?;
        let child=db.commit_selection_child(&plan,"新子板")?;
        db.backup(&backup)?;
        drop(db);drop(writer);
        for path in [&file,&backup]{
            let db=Notebook::open(path,false)?;
            assert_eq!(db.cards_on(board)?.iter().map(|c|c.id).collect::<Vec<_>>(),[decoy]);
            assert_eq!(db.cards_on(other)?[0].id,a);
            assert_eq!(db.cards_on(child)?.iter().map(|c|(c.id,c.x,c.y)).collect::<Vec<_>>(),[(a,400,100),(b,660,100)]);
            let tile=db.objects_on(board)?.into_iter().find(|o|o.child==Some(child)).unwrap();
            assert_eq!((tile.x,tile.y),(400,100));
            assert_eq!(db.child_board_from_tile(board,tile.id,child)?.id,child);
            assert_eq!(db.parent_board_tile(child)?.unwrap().1,tile.id);
            assert_eq!(db.groups_on(child)?[0].id,group);
            assert!(db.groups_on(board)?.is_empty());
            assert_eq!(db.edges_on(child)?.len(),1);
            assert_eq!(db.live_board_group_source(child,group,annotation)?.unwrap().attachment_id,attachment);
            assert_eq!(db.list_attachments(a)?[0].id,attachment);
            assert_eq!(db.read_pdf_attachment(attachment)?.1,std::fs::read(&pdf)?);
            assert_eq!(db.note(a)?.unwrap().body,"甲正文");
            assert!(!db.connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",[],|r|r.get::<_,bool>(0))?);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
}
