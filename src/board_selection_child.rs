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

impl Notebook {
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
