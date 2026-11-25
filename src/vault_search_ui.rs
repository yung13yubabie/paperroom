use super::navigation_ui::{choose_navigation, choose_navigation_page};
use super::*;
use crate::store::vault_search::{TagMatch, TagPanel, VaultSourceItem, VaultTagTree, vault_aliases_preview};

#[derive(Clone, Copy)]
enum TagMenu {
    Open(TagMatch),
    Up,
    ToggleBranch,
    ToggleTerm,
    Child(usize),
}

#[derive(Clone, Copy)]
enum PanelMenu {
    Sort,
    Layout,
    All(bool),
    Search,
    ClearTerms,
    Details(usize, usize),
}

fn tag_row(tree: &VaultTagTree, panel: &TagPanel, id: usize, depth: usize) -> String {
    let tag = &tree.branches[id];
    let count = if panel.flat { tag.exact_count } else { tag.note_count };
    let selected = if panel.uses_term(id) { "　已加入搜尋" } else { "" };
    let indent = "　".repeat(depth.min(5));
    format!("{indent}#{}　（{count} 篇）{selected}", tree.path(id))
}

impl App {
    unsafe fn pick_imported_vault(&self) -> Result<Option<i64>> {
        let vaults = self.notebook.imported_vaults()?;
        if vaults.is_empty() {
            unsafe { label(self.widgets.status, "請先匯入一個 Vault。"); }
            return Ok(None);
        }
        let names = vaults.iter().map(|(id, title)| format!("{title} (#{id})")).collect::<Vec<_>>();
        let selected = unsafe { choose_navigation(self.hwnd, self.widgets.files, &names) };
        Ok(selected.map(|index| vaults[index].0))
    }

    unsafe fn open_vault_search_result(&mut self, root: i64, hits: &[(i64, String)]) -> Result<bool> {
        if hits.is_empty() {
            unsafe { label(self.widgets.status, "這個 Vault 沒有符合條件的來源筆記。"); }
            return Ok(false);
        }
        let labels = hits.iter().map(|(_, path)| path.as_str()).collect::<Vec<_>>();
        if let Some(index) = unsafe { choose_navigation(self.hwnd, self.widgets.files, &labels) } {
            let (id, path) = &hits[index];
            if !self.notebook.vault_search_hit(root, *id, path)? {
                return Err("來源筆記已變更或移到垃圾桶，請重新搜尋。".into());
            }
            unsafe { self.open_import(*id, "已開啟符合條件的 Vault 來源筆記。"); }
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) unsafe fn vault_folder_picker(&mut self) -> Result<()> {
        unsafe {
            let Some(root) = self.pick_imported_vault()? else { return Ok(()); };
            let mut folder = String::new();
            loop {
                let items = self.notebook.vault_source_children(root, &folder)?;
                if folder.is_empty() && items.is_empty() {
                    label(self.widgets.status, "這個 Vault 沒有可瀏覽的來源筆記。");
                    return Ok(());
                }
                let location = if folder.is_empty() { "根目錄" } else { &folder };
                label(self.widgets.status, &format!("Vault 來源資料夾：{location}"));
                let up = usize::from(!folder.is_empty());
                let selected = choose_navigation_page(self.hwnd, self.widgets.files, up + items.len(), |index| {
                    let Some(index) = index.checked_sub(up) else { return "返回上一層".to_owned(); };
                    match &items[index] {
                        VaultSourceItem::Folder(path) => format!("資料夾　{}/", source_leaf(path)),
                        VaultSourceItem::Note { path, .. } => format!("筆記　{}", source_leaf(path)),
                    }
                });
                let Some(selected) = selected else { return Ok(()); };
                let Some(index) = selected.checked_sub(up) else {
                    folder.truncate(folder.rfind('/').unwrap_or(0));
                    continue;
                };
                match &items[index] {
                    VaultSourceItem::Folder(path) => folder.clone_from(path),
                    VaultSourceItem::Note { id, path } => {
                        if !self.notebook.vault_search_hit(root, *id, path)? {
                            return Err("來源筆記已變更或移到垃圾桶，請重新瀏覽。".into());
                        }
                        let status = format!("Vault 來源：{path}");
                        self.open_import(*id, &status);
                        if let Some(note) = self.current.as_ref().filter(|note| note.row.id == *id) {
                            let aliases = vault_aliases_preview(&note.body)?;
                            if !aliases.is_empty() {
                                label(self.widgets.status, &format!("{status}　別名：{}", aliases.join("、")));
                            }
                        }
                        return Ok(());
                    }
                }
            }
        }
    }

    pub(super) unsafe fn vault_source_picker(&mut self) -> Result<()> {
        unsafe {
            let Some(root) = self.pick_imported_vault()? else { return Ok(()); };
            let Some(folder) = crate::dialog::prompt(
                self.hwnd, "搜尋 Vault 來源", "來源資料夾（含子資料夾；留白搜尋整個 Vault）", "", 4096
            )? else { return Ok(()); };
            let Some(alias) = crate::dialog::prompt(
                self.hwnd, "搜尋 Vault 來源", "別名（完全相符；留白不限）", "", 4096
            )? else { return Ok(()); };
            let Some(tag) = crate::dialog::prompt(
                self.hwnd, "搜尋 Vault 來源", "標籤（可含 #，留白不限）", "", 4096
            )? else { return Ok(()); };
            let mode = if tag.trim().is_empty() {
                TagMatch::Exact
            } else {
                let options = ["完全相符", "包含子標籤"];
                match choose_navigation(self.hwnd, self.widgets.files, &options) {
                    Some(0) => TagMatch::Exact,
                    Some(1) => TagMatch::Descendants,
                    _ => return Ok(()),
                }
            };
            let hits = self.notebook.search_vault_sources(root, &folder, &alias, &tag, mode)?;
            self.open_vault_search_result(root, &hits).map(|_| ())
        }
    }

    pub(super) unsafe fn vault_tag_picker(&mut self) -> Result<()> {
        unsafe {
            let Some(root) = self.pick_imported_vault()? else { return Ok(()); };
            let tree = self.notebook.vault_tag_tree(root)?;
            if tree.branches[0].children.is_empty() {
                label(self.widgets.status, "這個 Vault 尚無可瀏覽的標籤。");
                return Ok(());
            }
            let mut panel = TagPanel::default();
            loop {
                let mut rows = vec![PanelMenu::Sort, PanelMenu::Layout];
                if !panel.flat {
                    rows.extend([PanelMenu::All(true), PanelMenu::All(false)]);
                }
                if panel.term_count() != 0 {
                    rows.extend([PanelMenu::Search, PanelMenu::ClearTerms]);
                }
                rows.extend(panel.rows(&tree).into_iter().map(|(id, depth)| PanelMenu::Details(id, depth)));
                let selected = choose_navigation_page(self.hwnd, self.widgets.files, rows.len(), |index| {
                    match rows[index] {
                        PanelMenu::Sort => (if panel.frequency { "依名稱排序" } else { "依筆記數排序" }).into(),
                        PanelMenu::Layout => (if panel.flat { "切換樹狀標籤" } else { "切換平面清單" }).into(),
                        PanelMenu::All(true) => "全部展開".into(),
                        PanelMenu::All(false) => "全部收合".into(),
                        PanelMenu::Search => format!("搜尋目前標籤　（{} 項皆須符合）", panel.term_count()),
                        PanelMenu::ClearTerms => "清除目前搜尋標籤".into(),
                        PanelMenu::Details(id, depth) => tag_row(&tree, &panel, id, depth),
                    }
                });
                let Some(selected) = selected else { return Ok(()); };
                match rows[selected] {
                    PanelMenu::Sort => panel.frequency = !panel.frequency,
                    PanelMenu::Layout => panel.flat = !panel.flat,
                    PanelMenu::All(expanded) => panel.set_all(expanded),
                    PanelMenu::ClearTerms => panel.clear_terms(),
                    PanelMenu::Search => {
                        let hits = self.notebook.search_vault_tag_terms(root, &panel.terms(&tree))?;
                        if self.open_vault_search_result(root, &hits)? { return Ok(()); }
                    }
                    PanelMenu::Details(id, _) => {
                        if self.vault_tag_actions(root, &tree, &mut panel, id)? { return Ok(()); }
                    }
                }
            }
        }
    }

    unsafe fn vault_tag_actions(
        &mut self, root: i64, tree: &VaultTagTree, panel: &mut TagPanel, initial: usize,
    ) -> Result<bool> {
        unsafe {
            let mut trail = vec![initial];
            loop {
                let id = *trail.last().unwrap();
                let branch = &tree.branches[id];
                let mut choices = vec![TagMenu::Open(TagMatch::Descendants)];
                if branch.exact {
                    let position = usize::from(!panel.flat);
                    choices.insert(position, TagMenu::Open(TagMatch::Exact));
                }
                choices.push(TagMenu::Up);
                if !panel.flat && !branch.children.is_empty() { choices.push(TagMenu::ToggleBranch); }
                choices.push(TagMenu::ToggleTerm);
                if !panel.flat {
                    choices.extend(panel.children(tree, id).into_iter().map(TagMenu::Child));
                }
                let selected = choose_navigation_page(self.hwnd, self.widgets.files, choices.len(), |index| {
                    match choices[index] {
                        TagMenu::Open(TagMatch::Descendants) => "開啟此層與子標籤的筆記".into(),
                        TagMenu::Open(TagMatch::Exact) => "僅開啟此層標籤的筆記".into(),
                        TagMenu::Up => "返回上層".into(),
                        TagMenu::ToggleBranch => (if panel.is_expanded(id) { "收合這個標籤" } else { "展開這個標籤" }).into(),
                        TagMenu::ToggleTerm => (if panel.uses_term(id) { "從搜尋移除此標籤" } else { "將此標籤加入搜尋" }).into(),
                        TagMenu::Child(child) => tag_row(tree, panel, child, 0),
                    }
                });
                let Some(selected) = selected else { return Ok(false); };
                match choices[selected] {
                    TagMenu::Child(child) => trail.push(child),
                    TagMenu::Up if trail.len() > 1 => { trail.pop(); }
                    TagMenu::Up => return Ok(false),
                    TagMenu::ToggleBranch => {
                        panel.toggle_branch(id);
                        return Ok(false);
                    }
                    TagMenu::ToggleTerm => {
                        panel.toggle_term(id)?;
                        return Ok(false);
                    }
                    TagMenu::Open(mode) => {
                        let hits = self.notebook.search_vault_sources(root, "", "", &tree.path(id), mode)?;
                        return self.open_vault_search_result(root, &hits);
                    }
                }
            }
        }
    }
}

fn source_leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
