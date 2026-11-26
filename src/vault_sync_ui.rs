use super::*;

const BACKUP_NOTICE: &str = "套用前，請選擇新檔保存整個工作區的 SQLite 備份。備份未加密，含其他筆記與附件。若目標資料夾由外部服務同步，該服務可能依你的設定上傳備份或保留歷史版本。";
const PUSH_NOTICE: &str = "每次實際改寫原檔，都會在同一資料夾另建一份明文 .bak，保存改寫前的完整內容。先前的 .bak 會保留。若 Vault 在同步資料夾內，這些副本也可能隨服務設定同步。";
const FORK_NOTICE: &str = "選入衝突列時，磁碟版會成為紙間工作區內另一篇筆記，佔用工作區空間，也會收入之後的工作區備份。日後再對新的衝突選擇保留兩份，會再新增一篇。";

impl App {
    pub(super) unsafe fn sync_vault_incoming(&mut self)->Result<()> {
        unsafe {
            let vaults=self.notebook.imported_vaults()?;
            if vaults.is_empty(){label(self.widgets.status,"請先匯入一個 Vault。");return Ok(());}
            let labels:Vec<_>=vaults.iter().map(|(id,title)|format!("{title} (#{id})")).collect();
            let choices:Vec<_>=labels.iter().enumerate().map(|(i,s)|(i+1,s.as_str())).collect();
            let Some(index)=popup(self.hwnd,self.widgets.files,None,&choices).checked_sub(1) else{return Ok(());};
            let (id,title)=&vaults[index];
            let Some(folder)=folder_dialog(self.hwnd)? else{return Ok(());};
            let preview=self.notebook.preview_vault_pull(*id,&folder)?;
            let context=format!("紙間：{title} (#{id})\r\n磁碟：{}",folder.display());
            crate::dialog::show_text(self.hwnd,"Vault 批次讀入預覽",&format!("{context}\r\n{BACKUP_NOTICE}\r\n{FORK_NOTICE}\r\n\r\n{}",preview.report()))?;
            if !preview.actionable(){return Ok(());}
            let choice = loop {
                let Some(input) = crate::dialog::prompt(
                    self.hwnd, "選取讀入項目",
                    "輸入「安全」或用半形逗號列出列號。輸入 ?1 查看第 1 列的差異。選入衝突會另存磁碟副本。",
                    "安全", 65535,
                )? else { return Ok(()); };
                if let Some(detail) = input.trim().strip_prefix('?') {
                    let row = detail.trim().parse::<usize>().map_err(|_| "請在問號後輸入預覽列號。")?;
                    let text = preview.details(&self.notebook, row)?;
                    crate::dialog::show_text(self.hwnd, "比對兩份原文", &text)?;
                } else {
                    break input;
                }
            };
            let (count,copies)=preview.counts(&choice)?;
            if message(self.hwnd,"確認批次讀入",&format!("{context}\r\n套用 {count} 列，其中 {copies} 列另存磁碟版本。{BACKUP_NOTICE}"),MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2)!=IDYES{return Ok(());}
            let Some(backup)=file_dialog(self.hwnd,"SQLite\0*.sqlite\0\0","sqlite",Some("vault-before-pull.sqlite"))? else{return Ok(());};
            self.notebook.apply_vault_pull(&preview,&folder,&choice,&backup)?;
            if let Some(id)=self.current.as_ref().map(|n|n.row.id) {
                self.current=self.notebook.note(id)?;
                self.present();
            }
            self.rebuild();
            label(self.widgets.status,&format!("已套用 {count} 列，新增 {copies} 篇磁碟版本。備份：{}",backup.display()));
            Ok(())
        }
    }
}

impl App {
    pub(super) unsafe fn sync_vault_outgoing(&mut self)->Result<()> {
        unsafe {
            let vaults=self.notebook.imported_vaults()?;
            if vaults.is_empty(){label(self.widgets.status,"請先匯入一個 Vault。");return Ok(());}
            let labels:Vec<_>=vaults.iter().map(|(id,title)|format!("{title} (#{id})")).collect();
            let choices:Vec<_>=labels.iter().enumerate().map(|(i,s)|(i+1,s.as_str())).collect();
            let Some(index)=popup(self.hwnd,self.widgets.files,None,&choices).checked_sub(1) else{return Ok(());};
            let (id,title)=&vaults[index];
            let Some(dir)=folder_dialog(self.hwnd)? else{return Ok(());};
            let plan=self.notebook.preview_vault_push(*id,&dir)?;
            let context=format!("紙間：{title} (#{id})\r\n磁碟：{}",dir.display());
            crate::dialog::show_text(self.hwnd,"Vault 批次寫回預覽",&format!("{context}\r\n{PUSH_NOTICE}\r\n{BACKUP_NOTICE}\r\n\r\n{}",plan.report()))?;
            if !plan.actionable(){return Ok(());}
            let choice=loop {
                let Some(input)=crate::dialog::prompt(self.hwnd,"選取寫回項目","輸入「安全」或可寫回的列號（半形逗號分隔）；?1 比對第一列。衝突列保留兩端。","安全",65535)? else{return Ok(());};
                if let Some(row)=input.trim().strip_prefix('?'){
                    let row=row.trim().parse::<usize>().map_err(|_|"請輸入有效列號。")?;
                    crate::dialog::show_text(self.hwnd,"比對兩份原文",&plan.details(&self.notebook,row)?)?;
                } else {break input;}
            };
            let count=plan.select(&choice)?.len();
            let warning=format!("{context}\r\n將處理 {count} 個原檔。寫回保留原有 BOM 與換行格式，並使紙間本文與磁碟一致。\r\n{PUSH_NOTICE}\r\n{BACKUP_NOTICE}");
            if message(self.hwnd,"確認批次寫回",&warning,MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2)!=IDYES{return Ok(());}
            let Some(backup)=file_dialog(self.hwnd,"SQLite\0*.sqlite\0\0","sqlite",Some("vault-before-push.sqlite"))? else{return Ok(());};
            let result=self.notebook.apply_vault_push(&plan,&choice,&backup)?;
            if let Some(id)=self.current.as_ref().map(|n|n.row.id) {
                self.current=self.notebook.note(id)?;
                self.present();
            }
            self.rebuild();
            crate::dialog::show_text(self.hwnd,"Vault 寫回結果",&result)
        }
    }
}

impl App {
    pub(super) unsafe fn refresh_vault_attachment_ui(&mut self) -> Result<()> {
        unsafe {
            let note = self.current.as_ref().filter(|note| note.row.trash_root.is_none())
                .ok_or("請先開啟有來源附件的 Vault 匯入筆記。")?.row.id;
            let choices = self.notebook.vault_attachment_choices(note)?;
            if choices.is_empty() {
                label(self.widgets.status, "這篇筆記沒有已匯入的 Vault 來源附件。");
                return Ok(());
            }
            let labels = choices.iter().map(|(_, relative)| relative.as_str()).collect::<Vec<_>>();
            let Some(index) = super::navigation_ui::choose_navigation(self.hwnd, self.widgets.files, &labels) else {
                return Ok(());
            };
            let Some(directory) = folder_dialog(self.hwnd)? else { return Ok(()); };
            let preview = self.notebook.preview_vault_attachment(note, choices[index].0, &directory)?;
            let report = preview.report();
            crate::dialog::show_text(self.hwnd, "Vault 來源附件預覽", &report)?;
            if !preview.actionable() {
                return Ok(());
            }
            let text = format!(
                "將重新讀入 {} 份已配對附件。Vault 的原附件與 Markdown 只供讀取。\r\n{BACKUP_NOTICE}",
                preview.count(),
            );
            if message(self.hwnd, "確認更新 Vault 附件", &text, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(());
            }
            let Some(backup) = file_dialog(self.hwnd, "SQLite\0*.sqlite\0\0", "sqlite", Some("vault-before-attachment.sqlite"))? else {
                return Ok(());
            };
            let count = self.notebook.apply_vault_attachment(&preview, &backup)?;
            label(self.widgets.status, &format!("已更新 {count} 筆 Vault 附件。備份：{}", backup.display()));
            Ok(())
        }
    }
}

impl App {
    pub(super) unsafe fn reconcile_vault_relocation_ui(&mut self) -> Result<()> {
        unsafe {
            let target = self.current.as_ref().ok_or("請先開啟已重新命名或搬移的 Vault 匯入筆記。")?.row.id;
            let (_, old) = self.notebook.vault_relocation_origin(target)?;
            let Some(directory) = folder_dialog(self.hwnd)? else { return Ok(()); };
            let hint = format!("原路徑：{old}\r\n輸入新的 Vault 相對 .md 路徑，以 / 分隔資料夾。");
            let Some(next) = crate::dialog::prompt(
                self.hwnd, "新的 Vault 來源路徑", &hint, "", 4096,
            )? else { return Ok(()); };
            let plan = self.notebook.preview_vault_relocation(target, &directory, &next)?;
            let text = plan.report();
            crate::dialog::show_text(self.hwnd, "Vault 外部改名或搬移預覽", &text)?;
            let confirmation = format!("配對新路徑並更新預覽所列的紙間連結與 Canvas？\r\n{BACKUP_NOTICE}");
            if message(self.hwnd, "確認配對新的 Vault 來源", &confirmation, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(());
            }
            let Some(backup) = file_dialog(self.hwnd, "SQLite\0*.sqlite\0\0", "sqlite", Some("vault-before-relocation.sqlite"))? else {
                return Ok(());
            };
            self.notebook.apply_vault_relocation(&plan, &backup)?;
            self.open_import(target, &format!("已配對 Vault 來源：{next}　備份：{}", backup.display()));
            Ok(())
        }
    }
}

impl App {
    pub(super) unsafe fn audit_incoming_vault_blocks(&mut self) -> Result<()> {
        unsafe {
            let target = self.current.as_ref().filter(|note| note.row.trash_root.is_none())
                .ok_or("請先開啟要查核的 Vault 匯入筆記。")?.row.id;
            let report = self.notebook.vault_incoming_block_report(target)?;
            crate::dialog::show_text(self.hwnd, "檢查 Vault 傳入區塊連結", &report)
        }
    }
}

impl App {
    pub(super) unsafe fn vault_unlinked_mentions_ui(&mut self) -> Result<()> {
        unsafe {
            let target = self.current.as_ref()
                .filter(|note| note.row.trash_root.is_none() && !self.extras.board)
                .ok_or("請先開啟要找提及的 Vault 匯入筆記。")?.row.id;
            let Some(tag) = crate::dialog::prompt(self.hwnd, "Vault 未連結提及",
                "輸入來源筆記的完整標籤，例如 #拍攝/夜景。留白檢查同一匯入批次。",
                "", 4096)? else { return Ok(()); };
            let (hits, limited) = self.notebook.vault_unlinked_mentions(target, &tag)?;
            if hits.is_empty() {
                label(self.widgets.status, "同一匯入批次沒有找到唯一且未連結的提及。");
                return Ok(());
            }
            if limited {
                label(self.widgets.status, "未連結提及最多顯示 200 處。");
            }
            let Some(index) = super::navigation_ui::choose_navigation_page(self.hwnd, self.widgets.files, hits.len(), |index| hits[index].label()) else {
                return Ok(());
            };
            let hit: &crate::store::note_links::VaultMention = &hits[index];
            crate::dialog::show_text(self.hwnd, "Vault 未連結提及預覽", &hit.context())?;
            let confirmation = format!("在紙間建立這一處本機連結？\r\n{}", hit.label());
            if message(self.hwnd, "建立本機連結", &confirmation, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                return Ok(());
            }
            let source = self.notebook.link_vault_mention(hit)?;
            self.open_import(source, "已建立這一處工作區連結。可用筆記版本記錄復原。");
            Ok(())
        }
    }
}

impl App {
    pub(super) unsafe fn add_verified_vault_source(&mut self)->Result<()> {
        unsafe {
            let anchor=self.current.as_ref().ok_or("請先開啟這個 Vault 的既有來源筆記。")?.row.id;
            self.notebook.vault_relocation_origin(anchor)?;
            let Some(dir)=folder_dialog(self.hwnd)? else{return Ok(())};
            let Some(path)=crate::dialog::prompt(self.hwnd,"加入 Vault 新來源","輸入新 Markdown 的精確相對路徑，以 / 分隔資料夾。","",4096)? else{return Ok(())};
            let preview=self.notebook.preview_vault_admission(anchor,&dir,&path)?;
            if message(self.hwnd,"確認加入 Vault 新來源",&preview.report(),MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2)!=IDYES{return Ok(())}
            let note=self.notebook.apply_vault_admission(&preview)?;
            self.open_import(note,"已建立新來源筆記，並更新跟隨這個 Vault 的 Bases。");
            Ok(())
        }
    }
}
