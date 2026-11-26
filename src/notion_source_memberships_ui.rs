use super::*;
use crate::store::notion_page_links::PageIdentity;

impl App {
    pub(in crate::window) unsafe fn show_notion_source_rows(&mut self,page:&PageIdentity)->Result<()> {
        unsafe {
            let hwnd=self.hwnd;
            let anchor=self.widgets.links;
            let mut offset=0;
            loop {
                let (rows,more,ambiguous)=self.notebook.notion_membership_page(page,offset)?;
                if ambiguous>0 {
                    crate::dialog::show_text(hwnd,"Notion 來源關聯待確認",&format!(
                        "{ambiguous} 篇集合筆記有多個同名來源關聯，未列入。請先在集合的關聯管理確認。"))?;
                }
                if rows.is_empty() && offset==0 {
                    label(self.widgets.status,"這個 Notion 來源頁沒有可開啟的集合資料列。");
                    return Ok(());
                }
                let captions=rows.iter().map(|row|row.label()).collect::<Vec<_>>();
                let mut items=captions.iter().enumerate().map(|(i,s)|(i+1,s.as_str())).collect::<Vec<_>>();
                if offset>0 {items.push((41,"上一頁"));}
                if more {items.push((42,"下一頁"));}
                let command=popup(hwnd,anchor,None,&items);
                match command {
                    41 if offset>0=>{offset-=40;continue;}
                    42 if more=>{offset+=40;continue;}
                    _ if command==0 || command>rows.len()=>return Ok(()),
                    _=>{}
                }
                let hit=&rows[command-1];
                let action=popup(hwnd,anchor,None,&[(1,"開啟資料列筆記"),(2,"在集合看板定位")]);
                if !matches!(action,1|2){return Ok(());}
                if action==1 {
                    let note=self.notebook.note_from_source_membership(page,hit)?;
                    self.view=View::All;
                    self.editing=false;
                    label(self.widgets.search,"");
                    self.set_current_note(Some(note));
                    self.rebuild();
                    label(self.widgets.status,"已開啟 Notion 來源頁的資料列。");
                }else{
                    let board=self.notebook.read_source_membership(page,hit,||
                        self.build_collection_layout(hit.collection,hit.collection_title.clone(),None))?;
                    self.reveal_collection_row(board,hit.row)?;
                }
                return Ok(())
            }
        }
    }
}
