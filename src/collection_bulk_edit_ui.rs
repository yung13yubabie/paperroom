use super::*;

impl App {
    pub(in crate::window) unsafe fn bulk_edit_collection(&mut self,id:i64,title:String)->Result<()> {
        use std::collections::BTreeSet;
        use crate::window::navigation_ui::choose_navigation_page;
        unsafe {
            let hwnd=self.hwnd;
            let anchor=self.widgets.links;
            let fields=self.notebook.bulk_field_choices(id)?;
            if fields.is_empty(){return Err("請先定義數字、核取方塊或日期屬性。".into());}
            let Some(f)=choose_navigation_page(hwnd,anchor,fields.len(),|i|fields[i].0.as_str()) else{return Ok(())};
            let (key,kind)=&fields[f];
            let view=self.active_collection_view(id);
            let edit=self.notebook.begin_bulk_field_edit(id,view.as_ref(),key,*kind)?;
            let visible=self.visible_collection_rows(id)?.into_iter().map(|(r,_)|r.id).collect::<BTreeSet<_>>();
            let rows=edit.rows.iter().filter(|r|visible.contains(&r.id)).collect::<Vec<_>>();
            if rows.len()<2 {return Err("目前檢視至少需要兩筆可見資料列。".into());}
            let mut selected=BTreeSet::new();
            loop {
                let Some(i)=choose_navigation_page(hwnd,anchor,rows.len()+1,|i|{
                    if i==0{return format!("繼續（已選 {} 筆）",selected.len());}
                    let row=rows[i-1];let mark=if selected.contains(&row.id){"■"}else{"□"};
                    format!("{mark} {}　[#{}]",row.title,row.id)
                }) else{return Ok(())};
                if i==0 {
                    if selected.len()>=2 {break;}
                    crate::dialog::show_text(hwnd,"選取資料列","請選取至少兩筆不同的資料列。")?;
                }else{
                    let row=rows[i-1].id;
                    if !selected.remove(&row) {
                        if selected.len()>=100 {crate::dialog::show_text(hwnd,"選取資料列","一次最多可選 100 筆。")?;}
                        else {selected.insert(row);}
                    }
                }
            }
            let next=if *kind==PropertyType::Checkbox {
                match popup(hwnd,anchor,None,&[(1,"勾選"),(2,"取消勾選"),(3,"清除欄位")]){
                    1=>Some("true".to_owned()),2=>Some("false".to_owned()),3=>None,_=>return Ok(()),
                }
            }else{
                let hint=format!("「{key}」需要{}；留白會清除所選資料列的欄位。",kind.hint());
                let mut draft=String::new();
                loop {
                    let Some(value)=crate::dialog::prompt(hwnd,"批次設定屬性",&hint,&draft,MAX_VALUE_BYTES)? else{return Ok(())};
                    if value.trim().is_empty(){break None;}
                    if value.len()<=MAX_VALUE_BYTES && kind.normalized(&value).is_some(){break Some(value);}
                    crate::dialog::show_text(hwnd,"請修正欄位值",&hint)?;draft=value;
                }
            };
            let mut text=format!("屬性：{key}\r\n已選 {} 筆。\r\n",selected.len());
            for row in rows.iter().filter(|r|selected.contains(&r.id)){
                text.push_str(&format!("\r\n{}　[#{}]　{} → {}",row.title,row.id,
                    row.previous.as_deref().unwrap_or("未設定"),next.as_deref().unwrap_or("清除")));
            }
            if !crate::dialog::preview_decision(hwnd,"確認批次編輯",&text,"套用","取消")? {return Ok(())}
            let ids=selected.into_iter().collect::<Vec<_>>();
            let changed=self.notebook.commit_bulk_field_edit(&edit,&ids,next.as_deref())?;
            if let Some(view)=view {self.reopen_collection_view(id,title,view.id)?;}
            else {self.open_collection_layout(id,title,None)?;}
            label(self.widgets.status,&format!("已選 {} 筆，實際更新 {changed} 筆。",ids.len()));
            Ok(())
        }
    }
}
