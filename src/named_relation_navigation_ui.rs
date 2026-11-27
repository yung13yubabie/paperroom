use super::*;
use crate::store::named_relation_navigation::{Direction, PAGE_SIZE};

impl App {
    pub(super) unsafe fn show_named_relation_navigation(&mut self, note: i64) -> Result<()> {
        let origin = self.notebook.named_relation_origin(note)?;
        let anchor = self.widgets.links;
        let mut actions=vec![(1,"指向此筆記的具名關聯…"),(2,"此筆記指向的具名關聯…"),(3,"編輯集合關聯屬性…")];
        let imported=self.notebook.notion_source_identity(note)?;
        if imported.is_some() {actions.push((4,"查看 Notion 來源頁的集合資料列…"));}
        let direction = match unsafe { popup(self.hwnd, anchor, None, &actions) } {
            4 => {if let Some(page)=imported {unsafe {self.show_notion_source_rows(&page)?;}} return Ok(())},
            1 => Direction::Incoming,
            2 => Direction::Outgoing,
            3 => return unsafe { self.edit_relation_fields(note) },
            _ => return Ok(()),
        };
        let mut page = 0;
        loop {
            let (hits, more) = self.notebook.named_relation_page(&origin, direction, page)?;
            if page == 0 && hits.is_empty() {
                unsafe { label(self.widgets.status, "沒有可開啟的具名關聯。"); }
                return Ok(());
            }
            let labels: Vec<_> = hits.iter().map(|hit| {
                let location = hit.location.as_deref().unwrap_or("工作區根層");
                format!("{}　{}　｜ {}　[#{id}]", hit.name, hit.title, location, id = hit.note)
            }).collect();
            let mut items: Vec<_> = labels.iter().enumerate()
                .map(|(i, label)| (i + 1, label.as_str())).collect();
            let previous = PAGE_SIZE + 1;
            let next = PAGE_SIZE + 2;
            if page > 0 { items.push((previous, "上一頁")); }
            if more { items.push((next, "下一頁")); }
            match unsafe { popup(self.hwnd, anchor, None, &items) } {
                command if command == previous => page -= 1,
                command if command == next => page += 1,
                command if (1..=hits.len()).contains(&command) => {
                    let target = self.notebook.note_from_named_relation(&origin, direction, &hits[command - 1])?;
                    unsafe {
                        self.view = View::All;
                        self.editing = false;
                        label(self.widgets.search, "");
                        self.set_current_note(Some(target));
                        self.rebuild();
                        label(self.widgets.status, "已開啟具名關聯的筆記。");
                    }
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
    }
}

impl App {
    pub(super) unsafe fn bind_notion_source_page(&mut self, row: i64) -> Result<()> {
        let token = self.notebook.notion_row_token(row)?.ok_or("此筆記已經不在集合中。")?;
        let previous = self.notebook.bound_notion_source(row)?;
        let batches = self.notebook.notion_source_batches()?;
        if batches.is_empty() { return Err("找不到含 Markdown 來源頁的 Notion 匯入批次。".into()); }
        let anchor = self.widgets.links;
        let Some(index) = (unsafe { navigation_ui::choose_navigation_page(self.hwnd, anchor, batches.len(),
            |i| format!("{}  (#{})", batches[i].title,batches[i].id)) }) else { return Ok(()); };
        let batch = &batches[index];
        let pages = self.notebook.notion_source_pages(batch,row)?;
        if pages.is_empty() { return Err("此批次沒有其他可連結的來源頁。".into()); }
        let Some(index) = (unsafe { navigation_ui::choose_navigation_page(self.hwnd, anchor, pages.len(),
            |i| pages[i].label()) }) else { return Ok(()); };
        let page = &pages[index];
        let (caption, prompt, status) = if let Some(old) = previous.as_ref() {
            if old.matches(page) {
                unsafe { label(self.widgets.status,"已經連結到這個 Notion Markdown 來源頁。"); }
                return Ok(());
            }
            ("更換 Notion 來源頁",
                format!("更換集合資料列 [#{row}] 的來源？\r\n原來源：{}\r\n新來源：{}  (#{})\r\n{}",
                    old.label(), batch.title, batch.id, page.label()),
                "已更換 Notion Markdown 來源頁。")
        } else {
            ("連結 Notion 來源頁",
                format!("將集合資料列 [#{row}] 連到您選取的來源頁？\r\n{}  (#{})\r\n{}\r\n這會建立本機具名關聯。",batch.title,batch.id,page.label()),
                "已連結 Notion Markdown 來源頁。")
        };
        if unsafe { message(self.hwnd,caption,&prompt,MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) } == IDYES {
            self.notebook.bind_notion_source(row,&token,batch,page,previous.as_ref())?;
            unsafe { label(self.widgets.status,status); }
        }
        Ok(())
    }
}

impl App {
    pub(super) unsafe fn edit_relation_fields(&mut self, note: i64) -> Result<()> {
        use super::navigation_ui::choose_navigation_page;
        unsafe {
            let anchor = self.widgets.links;
            let memberships = self.notebook.relation_collections(note)?;
            if memberships.is_empty() { return Err("請先將筆記加入可編輯的集合。".into()); }
            let fields = self.notebook.editable_relation_fields(note)?;
            let Some(index) = choose_navigation_page(self.hwnd,anchor,fields.len()+1,|i|{
                if i==0 { return "新增關聯屬性…".to_owned(); }
                let field=&fields[i-1].0;
                let direction=if field.reverse {"反向"}else{"正向"};
                format!("{direction}　{}　｜ {}　[#{}]",field.name,field.other,field.id)
            }) else {return Ok(())};
            if index==0 {
                let Some(s)=choose_navigation_page(self.hwnd,anchor,memberships.len(),|i|
                    format!("{}　[#{}]",memberships[i].1,memberships[i].0)) else {return Ok(())};
                let targets=self.notebook.relation_target_collections()?;
                let Some(t)=choose_navigation_page(self.hwnd,anchor,targets.len(),|i|
                    format!("{}　[#{}]",targets[i].1,targets[i].0)) else {return Ok(())};
                let Some(name)=crate::dialog::prompt(self.hwnd,"新增集合關聯屬性","關聯屬性名稱","",128)? else {return Ok(())};
                let (id,title,origin)=&memberships[s];
                let (target,other)=&targets[t];
                self.notebook.create_relation_field(origin,(*id,title),(*target,other),&name)?;
                label(self.widgets.status,"已新增集合關聯屬性。");
                return Ok(());
            }
            let (field,origin)=&fields[index-1];
            let action=popup(self.hwnd,anchor,None,&[(1,"加入關聯筆記…"),(2,"移除關聯筆記…"),(3,"開啟關聯筆記…"),
                (4,"儲存關聯計數公式…"),(5,"儲存數值加總公式…")]);
            if action == 4 || action == 5 {
                let number = if action == 5 {
                    let keys = self.notebook.property_definitions()?.into_iter()
                        .filter_map(|(key,kind)| (kind == crate::store::typed_properties::PropertyType::Number).then_some(key))
                        .collect::<Vec<_>>();
                    if keys.is_empty() {return Err("請先定義可加總的數字屬性。".into());}
                    let Some(i) = choose_navigation_page(self.hwnd,anchor,keys.len(),|i| keys[i].as_str()) else {return Ok(())};
                    Some(keys[i].clone())
                } else {None};
                let Some(name) = crate::dialog::prompt(self.hwnd,"儲存集合彙總公式","公式名稱","",128)? else {return Ok(())};
                self.notebook.save_field_rollup(field,origin,number.as_deref(),&name)?;
                label(self.widgets.status,"已儲存集合彙總公式。");
                return Ok(());
            }
            if !(1..=3).contains(&action) {return Ok(())}
            let choices=self.notebook.field_candidates(field,origin,action!=1)?;
            if choices.is_empty() {
                label(self.widgets.status,if action==1 {"沒有可加入的關聯筆記。"}else{"這個關聯屬性尚無可操作的筆記。"});
                return Ok(());
            }
            let Some(i)=choose_navigation_page(self.hwnd,anchor,choices.len(),|i|
                format!("{}　[#{}]",choices[i].title,choices[i].id)) else {return Ok(())};
            let target=&choices[i];
            match action {
                3=>{
                    let target=self.notebook.open_field_value(field,origin,target)?;
                    self.view=View::All;
                    self.editing=false;
                    label(self.widgets.search,"");
                    self.set_current_note(Some(target));
                    self.rebuild();
                    label(self.widgets.status,"已開啟集合關聯筆記。");
                }
                1|2=>{
                    if action==2 {
                        let prompt=format!("移除「{}」中 {} [#{}] 與 {} [#{}] 的關聯？",field.name,origin.title,origin.id,target.title,target.id);
                        if message(self.hwnd,"移除集合關聯",&prompt,MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2)!=IDYES {return Ok(())}
                    }
                    self.notebook.edit_field_value(field,origin,target,action==1)?;
                    label(self.widgets.status,if action==1{"已加入集合關聯筆記。"}else{"已移除集合關聯筆記。"});
                }
                _=>unreachable!(),
            }
            Ok(())
        }
    }
}
