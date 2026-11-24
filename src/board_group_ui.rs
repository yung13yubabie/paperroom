use super::*;

impl App {
    pub(in crate::window) unsafe fn board_group_menu(&mut self, action: usize) -> Result<()> {
        unsafe {
            let board = self.board_id();
            let groups = self.notebook.groups_on(board)?;
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            if groups.is_empty() {
                label(self.widgets.status, "這張白板還沒有分區。");
                return Ok(());
            }
            let Some(index) = choose(hwnd, anchor, &groups, |group| {
                format!("{}（{} 張卡片）", group.title, group.members.len())
            }) else { return Ok(()); };
            let group = &groups[index];
            match action {
                5 => {
                    let Some(title) = crate::dialog::prompt(hwnd, "重新命名分區", "分區名稱", &group.title, 80)?
                        else { return Ok(()); };
                    self.notebook.rename_board_group(board, group.id, &group.title, &title)?;
                    self.refresh_board()?;
                    label(self.widgets.status, "分區已重新命名。");
                }
                6 => {
                    let snapshot = self.notebook.board_group_positions(board, group.id)?;
                    if snapshot.is_empty() {
                        label(self.widgets.status, "這個分區沒有可移動的卡片。");
                        return Ok(());
                    }
                    let Some(input) = crate::dialog::prompt(
                        hwnd, "整組移動分區",
                        "輸入水平,垂直位移（整數）。正值往右或往下，負值往左或往上。",
                        "", 30,
                    )? else { return Ok(()); };
                    let (dx, dy) = input.split_once(',').ok_or("請輸入兩個逗號分隔的整數。")?;
                    let dx = dx.trim().parse::<i32>().map_err(|_| "水平位移須為整數。")?;
                    let dy = dy.trim().parse::<i32>().map_err(|_| "垂直位移須為整數。")?;
                    self.notebook.move_board_group(board, group.id, &snapshot, dx, dy)?;
                    self.refresh_board()?;
                    label(self.widgets.status, &format!("已整組移動 {} 張卡片。", snapshot.len()));
                }
                7 => self.open_group_source(board, group.id)?,
                8 => self.group_pdf_source_menu(board, group.id)?,
                _ => {}
            }
            Ok(())
        }
    }

    unsafe fn group_pdf_source_menu(&mut self, board: i64, group: i64) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let current = self.notebook.board_group_source_id(board, group)?;
            let mut actions = vec![(1, "設定／更換釘選標註…")];
            if current.is_some() {
                actions.extend([(2, "開啟釘選標註"), (3, "移除釘選標註")]);
            }
            match popup(hwnd, anchor, None, &actions) {
                1 => {
                    let Some(annotation) = self.choose_group_pdf_annotation(board, group)?
                        else { return Ok(()); };
                    self.notebook.set_board_group_source(board, group, current, Some(annotation))?;
                    label(self.widgets.status, "已釘選 PDF 標註。");
                }
                2 => {
                    let Some(annotation) = current else { return Ok(()); };
                    let Some(source) = self.notebook.live_board_group_source(board, group, annotation)? else {
                        label(self.widgets.status, "釘選標註已變更，或其筆記已離開分區、移入垃圾桶。");
                        return Ok(());
                    };
                    self.open_anchored_note(source.note_id, None)?;
                    self.open_pdf_annotation_at(source.attachment_id, source.page, source.id)?;
                }
                3 => {
                    self.notebook.set_board_group_source(board, group, current, None)?;
                    label(self.widgets.status, "已移除釘選標註。");
                }
                _ => {}
            }
            Ok(())
        }
    }

    unsafe fn choose_group_pdf_annotation(&self, board: i64, group: i64) -> Result<Option<i64>> {
        unsafe {
            let mut sources = Vec::new();
            for card in self.notebook.board_group_positions(board, group)? {
                let Some(note) = self.notebook.note(card.note_id)?.filter(|note| note.row.trash_root.is_none())
                    else { continue; };
                let pdfs = self.notebook.list_attachments(note.row.id)?.into_iter()
                    .filter(|attachment| attachment.kind == "pdf").collect::<Vec<_>>();
                if !pdfs.is_empty() {
                    sources.push((note.row, pdfs));
                }
            }
            if sources.is_empty() {
                label(self.widgets.status, "這個分區沒有附帶 PDF 的筆記。");
                return Ok(None);
            }
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let Some(index) = choose(hwnd, anchor, &sources, |(note, _)| {
                let title = note.title.split_whitespace().collect::<Vec<_>>().join(" ");
                format!("{title}（#{}）", note.id)
            }) else { return Ok(None); };
            let files = &sources[index].1;
            let Some(index) = choose(hwnd, anchor, files, |file| format!("{}（#{}）", file.name, file.id))
                else { return Ok(None); };
            let annotations = self.notebook.list_pdf_annotations(files[index].id)?;
            if annotations.is_empty() {
                label(self.widgets.status, "這份 PDF 還沒有標註。");
                return Ok(None);
            }
            let index = choose(hwnd, anchor, &annotations, |annotation| {
                let snippet = if annotation.quote.trim().is_empty() { &annotation.comment } else { &annotation.quote };
                let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
                let preview: String = snippet.chars().take(54).collect();
                format!("第 {} 頁（#{}）· {preview}", annotation.page, annotation.id)
            });
            Ok(index.map(|index| annotations[index].id))
        }
    }

    unsafe fn open_group_source(&mut self, board: i64, group: i64) -> Result<()> {
        unsafe {
            let hwnd = self.hwnd;
            let anchor = self.widgets.links;
            let mut sources = Vec::new();
            for card in self.notebook.board_group_positions(board, group)? {
                if let Some(note) = self.notebook.note(card.note_id)?.filter(|note| note.row.trash_root.is_none()) {
                    let pdfs = self.notebook.list_attachments(note.row.id)?.into_iter()
                        .filter(|attachment| attachment.kind == "pdf").collect::<Vec<_>>();
                    sources.push((note.row, pdfs));
                }
            }
            if sources.is_empty() {
                label(self.widgets.status, "這個分區沒有可開啟的來源筆記。");
                return Ok(());
            }
            let Some(index) = choose(hwnd, anchor, &sources, |(note, pdfs)| {
                let title = note.title.split_whitespace().collect::<Vec<_>>().join(" ");
                format!("{title}（#{}） · {} 份 PDF", note.id, pdfs.len())
            }) else { return Ok(()); };
            let (note, pdfs) = &sources[index];
            let attachment = if pdfs.is_empty() {
                None
            } else {
                match popup(hwnd, anchor, None, &[(1, "開啟來源筆記"), (2, "開啟附帶的 PDF…")]) {
                    1 => None,
                    2 => {
                        let Some(index) = choose(hwnd, anchor, pdfs, |file| format!("{}（#{}）", file.name, file.id))
                            else { return Ok(()); };
                        Some(pdfs[index].id)
                    }
                    _ => return Ok(()),
                }
            };
            if !self.notebook.board_group_positions(board, group)?.iter().any(|card| card.note_id == note.id) {
                return Err("這篇筆記已離開分區、移出白板或移到垃圾桶。".into());
            }
            self.open_anchored_note(note.id, None)?;
            if let Some(file) = attachment {
                self.open_pdf_view(file, 1)?;
            }
            Ok(())
        }
    }
}
