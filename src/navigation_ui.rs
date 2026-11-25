use super::*;
use crate::markdown::block_anchors::{BlockLookup, resolve_block};
use crate::store::navigation::{BlockNoteSnapshot, BlockNoteState, editor_source_selection};

const BLOCK_RETURN_LIMIT: usize = 16;
const RICH_EDIT_LINE_FROM_CHAR: u32 = WM_USER + 54; // EM_EXLINEFROMCHAR

struct NotePosition {
    editing: bool,
    start: usize,
    end: usize,
    first_char: Option<usize>,
}

pub(super) struct BlockReturn {
    note: BlockNoteSnapshot,
    position: NotePosition,
}

pub(super) unsafe fn choose_navigation(
    owner: HWND,
    anchor: HWND,
    labels: &[impl AsRef<str>],
) -> Option<usize> {
    unsafe { choose_navigation_page(owner, anchor, labels.len(), |index| labels[index].as_ref()) }
}

pub(super) unsafe fn choose_navigation_page<T: AsRef<str>>(
    owner: HWND,
    anchor: HWND,
    count: usize,
    name: impl Fn(usize) -> T,
) -> Option<usize> {
    const PAGE_SIZE: usize = 40;
    let mut start = 0;
    loop {
        let end = (start + PAGE_SIZE).min(count);
        let labels = (start..end).map(&name).collect::<Vec<_>>();
        let mut entries = labels.iter().enumerate()
            .map(|(i, label)| (i + 1, label.as_ref())).collect::<Vec<_>>();
        if start > 0 { entries.push((41, "上一頁")); }
        if end < count { entries.push((42, "下一頁")); }
        match unsafe { popup(owner, anchor, None, &entries) } {
            41 => start -= PAGE_SIZE,
            42 => start += PAGE_SIZE,
            index if (1..=labels.len()).contains(&index) => return Some(start + index - 1),
            _ => return None,
        }
    }
}

impl App {
    pub(super) unsafe fn navigation_shortcut(&mut self,msg:&MSG)->bool {
        unsafe {
            if msg.message!=WM_KEYDOWN || (msg.hwnd!=self.hwnd && IsChild(self.hwnd,msg.hwnd)==0)
                || [VK_CONTROL,VK_MENU,VK_SHIFT,VK_LWIN,VK_RWIN].iter().any(|k|GetKeyState(*k as i32)<0) {
                return false;
            }
            let key=msg.wParam as u16;
            if !matches!(key,VK_F6|VK_F7|VK_F8|VK_F9){return false;}
            if (msg.lParam & (1<<30))==0 {
                if key == VK_F8 {
                    self.show_note_outline();
                } else {
                    self.mutate(|app|match key {VK_F6=>app.command_palette(),VK_F7=>app.quick_switcher(),_=>app.return_from_block_link()});
                }
            }
            true
        }
    }

    pub(super) unsafe fn command_palette(&mut self)->Result<()> {
        unsafe {
            let Some(query)=crate::dialog::prompt(self.hwnd,"命令面板","輸入命令關鍵字，留白列出全部。","",128)? else{return Ok(())};
            let commands=["快速切換筆記 (F7)","本篇標題大綱 (F8)","搜尋 Vault 來源","開啟本篇筆記關聯圖","在全部筆記中搜尋","瀏覽 Vault 標籤","瀏覽 Vault 來源資料夾","核對 Vault 外部改名或搬移","返回區塊連結來源 (F9)","搜尋區塊並複製連結","檢查 Vault 傳入區塊連結","檢查 Vault 未連結提及","跨頁待辦總覽","日誌月曆","插入同步區塊","管理本篇同步區塊","查看同步區塊引用"];
            let found:Vec<_>=commands.iter().enumerate().filter(|(_,s)|s.to_lowercase().contains(&query.trim().to_lowercase())).collect();
            let labels:Vec<_>=found.iter().map(|(_,s)|s.to_string()).collect();
            if found.is_empty(){label(self.widgets.status,"找不到符合的命令。");return Ok(());}
            let Some(index)=choose_navigation(self.hwnd,self.widgets.files,&labels) else{return Ok(())};
            match found[index].0 {
                0=>self.quick_switcher(),1=>self.note_outline(),2=>self.vault_source_picker(),
                3=>{
                    let id=self.current.as_ref().filter(|n|n.row.trash_root.is_none()).ok_or("請先選取使用中的筆記。")?.row.id;
                    self.open_note_graph(id)
                }
                5=>self.vault_tag_picker(),6=>self.vault_folder_picker(),
                7=>self.reconcile_vault_relocation_ui(),
                8=>self.return_from_block_link(),
                9=>self.copy_block_link_ui(),
                10=>self.audit_incoming_vault_blocks(),
                11=>self.vault_unlinked_mentions_ui(),
                12=>self.task_overview(),
                13=>self.journal_calendar(),
                14=>self.insert_synced_block(),
                15=>self.manage_synced_blocks(),
                16=>self.view_synced_consumers(),
                _=>self.workspace_search_ui(),
            }
        }
    }

    pub(super) unsafe fn quick_switcher(&mut self)->Result<()> {
        unsafe {
            let Some(query)=crate::dialog::prompt(self.hwnd,"快速切換筆記","標題、來源路徑、完整別名，或 tag:#標籤（包含子標籤）；tag:=#標籤 僅完全相符。留白列出全部。","",4096)? else{return Ok(())};
            let hits=self.notebook.quick_notes(&query)?;
            if hits.is_empty(){label(self.widgets.status,"沒有符合條件的筆記。");return Ok(());}
            let labels=hits.iter().map(|h|h.label.clone()).collect::<Vec<_>>();
            if let Some(i)=choose_navigation(self.hwnd,self.widgets.files,&labels) {
                let id=hits[i].id;
                if !self.notebook.meta(id)?.is_some_and(|r|r.trash_root.is_none()){return Err("筆記已移到垃圾桶或刪除，請重新搜尋。".into());}
                self.open_import(id,"已開啟選取的筆記。");
            }
            Ok(())
        }
    }


    unsafe fn copy_block_link_ui(&mut self) -> Result<()> {
        unsafe {
            let id = self.current.as_ref()
                .filter(|note| note.row.trash_root.is_none() && !self.extras.board)
                .ok_or("請先開啟一篇使用中的筆記。")?.row.id;
            let vault_scope = if self.notebook.graph_vault(id)?.is_some() {
                match choose_navigation(self.hwnd, self.widgets.files, &["本篇筆記", "同一個 Vault 匯入批次"]) {
                    Some(index) => index == 1,
                    None => return Ok(()),
                }
            } else {
                false
            };
            let Some(query) = crate::dialog::prompt(
                self.hwnd, "搜尋區塊並複製連結", "輸入已標記的區塊文字或 ID，留白列出全部。", "", 4096,
            )? else {
                return Ok(());
            };
            let (hits, truncated) = self.notebook.block_link_candidates(id, vault_scope, &query)?;
            if hits.is_empty() {
                label(self.widgets.status, "沒有唯一且符合條件的區塊 ID。");
                return Ok(());
            }
            if truncated {
                label(self.widgets.status, "只顯示前 200 個區塊。可縮短範圍或輸入更多關鍵字。");
            }
            if let Some(index) = choose_navigation_page(self.hwnd, self.widgets.files, hits.len(), |i| &hits[i].label) {
                let link = self.notebook.stable_block_link(&hits[index])?;
                copy_unicode_text(self.hwnd, &link)?;
                label(self.widgets.status, "已複製區塊連結，可貼到此工作區的筆記。");
            }
            Ok(())
        }
    }

    pub(super) unsafe fn show_note_outline(&mut self) {
        unsafe {
            if let Err(error) = self.note_outline() {
                self.show_error(error);
            }
        }
    }

    unsafe fn note_outline(&mut self)->Result<()> {
        unsafe {
            let note=self.current.as_ref().filter(|n|n.row.trash_root.is_none() && !self.extras.board)
                .ok_or("請先開啟一篇使用中的筆記。")?;
            let draft = if self.editing {
                if !self.editor_ready {
                    return Err("編輯器尚未就緒，請重新開啟筆記。".into());
                }
                Some(read_text(self.widgets.body)?)
            } else {
                None
            };
            let headings = crate::store::navigation::outline(draft.as_deref().unwrap_or(&note.body));
            if headings.is_empty() {
                label(self.widgets.status, "本篇沒有 Markdown 標題。");
                return Ok(());
            }
            let Some(i) = choose_navigation_page(self.hwnd, self.widgets.files, headings.len(), |i| {
                let h = &headings[i];
                format!("H{}　{}　（第 {} 行）", h.level, h.title, h.line)
            }) else { return Ok(()); };
            let h = &headings[i];
            if self.focus_source_selection(h.start, h.end)? {
                label(self.widgets.status, &format!("標題大綱：第 {} 行，{}", h.line, h.title));
            }
            Ok(())
        }
    }

    pub(super) unsafe fn focus_source_selection(&mut self, start: usize, end: usize) -> Result<bool> {
        unsafe {
            if !self.editing { self.toggle_edit(); }
            if !self.editing { return Ok(false); }
            if !self.editor_ready {
                return Err("編輯器尚未就緒，請重新開啟筆記。".into());
            }
            SetFocus(self.widgets.body);
            SendMessageW(self.widgets.body, EM_SETSEL, start, end as isize);
            SendMessageW(self.widgets.body, EM_SCROLLCARET, 0, 0);
            Ok(true)
        }
    }

    unsafe fn note_position(&self) -> NotePosition {
        unsafe {
            let editing = self.editing && self.editor_ready;
            let control = if editing { self.widgets.body } else { self.widgets.preview };
            let (mut start, mut end) = (0u32, 0u32);
            SendMessageW(control, EM_GETSEL, (&raw mut start) as usize, (&raw mut end) as isize);
            let line = SendMessageW(control, EM_GETFIRSTVISIBLELINE, 0, 0);
            let first_char = if line >= 0 {
                usize::try_from(SendMessageW(control, EM_LINEINDEX, line as usize, 0)).ok()
            } else {
                None
            };
            NotePosition { editing, start: start as usize, end: end as usize, first_char }
        }
    }

    unsafe fn restore_note_position(&self, position: &NotePosition) {
        unsafe {
            let control = if position.editing { self.widgets.body } else { self.widgets.preview };
            SetFocus(control);
            SendMessageW(control, EM_SETSEL, position.start, position.end as isize);
            if let Some(first_char) = position.first_char {
                let line = if position.editing {
                    SendMessageW(control, EM_LINEFROMCHAR, first_char, 0)
                } else {
                    SendMessageW(control, RICH_EDIT_LINE_FROM_CHAR, 0, first_char as isize)
                };
                let current = SendMessageW(control, EM_GETFIRSTVISIBLELINE, 0, 0);
                if line >= 0 && current >= 0 {
                    SendMessageW(control, EM_LINESCROLL, 0, line - current);
                }
            }
        }
    }

    unsafe fn open_block_return(&mut self, origin: BlockReturn) -> Result<()> {
        unsafe {
            let fresh = self.notebook.note(origin.note.id)?;
            let Some(fresh) = fresh.filter(|note| origin.note.state(Some(note)) != BlockNoteState::Unavailable) else {
                label(self.widgets.status, "無法返回這篇來源筆記，已保留目前畫面。");
                return Ok(());
            };
            self.open_import(origin.note.id, "已返回區塊連結來源。");
            let Some(current) = self.current.as_ref().filter(|note| note.row.id == origin.note.id) else {
                return Err("未能重新開啟區塊連結來源。".into());
            };
            if current.version < fresh.version {
                self.current = Some(fresh);
            }
            match origin.note.state(self.current.as_ref()) {
                BlockNoteState::Unchanged => {
                    // open_import starts in reading mode, even for same-note jumps.
                    self.present();
                    if origin.position.editing {
                        self.toggle_edit();
                        if !self.editing || !self.editor_ready {
                            return Err("原筆記已開啟，但編輯器無法還原原位置。".into());
                        }
                    }
                    self.restore_note_position(&origin.position);
                    label(self.widgets.status, "已返回區塊連結來源。");
                }
                BlockNoteState::Revised => {
                    self.present();
                    label(self.widgets.status, "原筆記已變更，已開啟目前版本，未還原舊位置。");
                }
                BlockNoteState::Unavailable => {
                    return Err("來源筆記在返回時變更，未還原原位置。".into());
                }
            }
            Ok(())
        }
    }

    unsafe fn return_from_block_link(&mut self) -> Result<()> {
        unsafe {
            let Some(origin) = self.extras.block_returns.pop() else {
                label(self.widgets.status, "沒有可返回的區塊連結來源。");
                return Ok(());
            };
            self.open_block_return(origin)
        }
    }

    pub(super) unsafe fn preview_block_link(&mut self, id: i64, anchor: &str) -> Result<()> {
        unsafe {
            let caller = self.current.as_ref().filter(|n| n.row.trash_root.is_none())
                .ok_or("找不到開啟此連結的來源筆記。")?;
            let origin = BlockReturn { note: BlockNoteSnapshot::capture(caller), position: self.note_position() };
            let target = self.notebook.note(id)?.filter(|n| n.row.trash_root.is_none())
                .ok_or("預覽目標已刪除或移到垃圾桶，已保留原處。")?;
            let target_snapshot = BlockNoteSnapshot::capture(&target);
            let source = match resolve_block(&target.body, anchor) {
                BlockLookup::Found { source, .. } => source,
                outcome => {
                    let message = match outcome {
                        BlockLookup::Ambiguous => "這個區塊 ID 重複，無法唯一定位。",
                        BlockLookup::Missing => "找不到這個區塊 ID。",
                        _ => "這個區塊 ID 僅能使用英文字母、數字或連字號。",
                    };
                    label(self.widgets.status, message);
                    return Ok(());
                }
            };
            let cursor = editor_source_selection(&target.body, source.clone())
                .ok_or("無法核對預覽區塊的原文位置。")?;
            let excerpt = &target.body[source];
            let end = excerpt.char_indices().nth(8192).map_or(excerpt.len(), |(i, _)| i);
            let body = format!("來源筆記：{} (#{})\r\n\r\n{}", target.row.title, id, &excerpt[..end]);
            let title = format!("區塊預覽 {anchor}");
            if !crate::dialog::preview_with_jump(self.hwnd, &title, &body)? {
                self.restore_note_position(&origin.position);
                return Ok(());
            }
            if origin.note.state(self.notebook.note(origin.note.id)?.as_ref()) != BlockNoteState::Unchanged {
                return Err("開啟連結的筆記已變更，請從目前頁面重新預覽。".into());
            }
            if target_snapshot.state(self.notebook.note(id)?.as_ref()) != BlockNoteState::Unchanged {
                return Err("預覽的目標已變更、刪除或移到垃圾桶，已保留原處。請重新預覽。".into());
            }

            self.open_import(id, "已開啟來源筆記。");
            if target_snapshot.state(self.current.as_ref()) != BlockNoteState::Unchanged {
                self.open_block_return(origin)?;
                return Err("目標在跳轉時變更，請重新預覽。".into());
            }
            if self.extras.block_returns.len() >= BLOCK_RETURN_LIMIT {
                self.extras.block_returns.remove(0);
            }
            self.extras.block_returns.push(origin);
            if self.focus_source_selection(cursor.start, cursor.end)? {
                label(self.widgets.status, &format!("區塊連結：第 {} 行，{}　F9 返回來源", cursor.line, &anchor[1..]));
            }
            Ok(())
        }
    }
}
