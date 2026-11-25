use super::{MAX_TAG_TERMS, Result, TagMatch, VaultTagTree};
use std::{cmp::Reverse, collections::HashSet};

#[derive(Default)]
pub(crate) struct TagPanel {
    pub(crate) frequency: bool,
    pub(crate) flat: bool,
    expanded: bool,
    overrides: HashSet<usize>,
    terms: Vec<(usize, TagMatch)>,
}

impl TagPanel {
    pub(crate) fn is_expanded(&self, branch: usize) -> bool {
        self.expanded ^ self.overrides.contains(&branch)
    }

    pub(crate) fn set_all(&mut self, expanded: bool) {
        self.expanded = expanded;
        self.overrides.clear();
    }

    pub(crate) fn toggle_branch(&mut self, branch: usize) {
        if !self.overrides.insert(branch) { self.overrides.remove(&branch); }
    }

    pub(crate) fn children(&self, tree: &VaultTagTree, branch: usize) -> Vec<usize> {
        let mut children = tree.branches[branch].children.values().copied().collect::<Vec<_>>();
        if self.frequency && !self.flat {
            children.sort_by_key(|&id| Reverse(tree.branches[id].note_count));
        }
        children
    }

    pub(crate) fn rows(&self, tree: &VaultTagTree) -> Vec<(usize, usize)> {
        let mut stack = Vec::new();
        let mut visible = Vec::new();
        stack.extend(self.children(tree, 0).into_iter().rev().map(|id| (id, 0)));
        while let Some((id, depth)) = stack.pop() {
            if !self.flat || tree.branches[id].exact {
                visible.push((id, if self.flat { 0 } else { depth }));
            }
            if self.flat || self.is_expanded(id) {
                stack.extend(self.children(tree, id).into_iter().rev().map(|child| (child, depth + 1)));
            }
        }
        if self.frequency && self.flat {
            visible.sort_by_key(|&(id, _)| Reverse(tree.branches[id].exact_count));
        }
        visible
    }

    pub(crate) fn term_count(&self) -> usize { self.terms.len() }

    pub(crate) fn uses_term(&self, branch: usize) -> bool {
        self.terms.iter().any(|&(id, _)| id == branch)
    }

    pub(crate) fn toggle_term(&mut self, branch: usize) -> Result<()> {
        if let Some(position) = self.terms.iter().position(|&(id, _)| id == branch) {
            self.terms.remove(position);
        } else {
            if self.terms.len() == MAX_TAG_TERMS {
                return Err(format!("最多可同時使用 {MAX_TAG_TERMS} 個標籤條件。").into());
            }
            let mode = if self.flat { TagMatch::Exact } else { TagMatch::Descendants };
            self.terms.push((branch, mode));
        }
        Ok(())
    }

    pub(crate) fn terms(&self, tree: &VaultTagTree) -> Vec<(String, TagMatch)> {
        self.terms.iter().map(|&(id, mode)| (tree.path(id), mode)).collect()
    }

    pub(crate) fn clear_terms(&mut self) { self.terms.clear(); }
}
