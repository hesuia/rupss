use super::{
    AppState, NAME_COLUMN_INDEX, PROCESS_TABLE_COLUMN_SPACING, ViewMode,
    process_table_column_widths, sort::compare_process_rows,
};
use crate::snapshot::ProcessRow;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeRow {
    pub(crate) process_index: usize,
    pub(crate) depth: usize,
    pub(crate) has_children: bool,
    pub(crate) expanded: bool,
    pub(crate) parent_index: Option<usize>,
    pub(crate) is_last_sibling: bool,
    pub(crate) ancestor_has_next_sibling: Vec<bool>,
}

impl AppState {
    pub(crate) fn visible_row_entries(&self) -> Vec<TreeRow> {
        match self.view.view_mode {
            ViewMode::Flat => {
                let end = self
                    .view
                    .scroll_offset
                    .saturating_add(self.view.viewport_rows)
                    .min(self.data.snapshot.processes.len());
                (self.view.scroll_offset.min(end)..end)
                    .map(TreeRow::flat)
                    .collect()
            }
            ViewMode::Tree => {
                let end = self
                    .view
                    .scroll_offset
                    .saturating_add(self.view.viewport_rows)
                    .min(self.tree.rows.len());
                self.tree.rows[self.view.scroll_offset.min(end)..end].to_vec()
            }
        }
    }

    pub fn total_visible_rows(&self) -> usize {
        match self.view.view_mode {
            ViewMode::Flat => self.data.snapshot.processes.len(),
            ViewMode::Tree => self.tree.rows.len(),
        }
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        match self.view.view_mode {
            ViewMode::Flat => (!self.data.snapshot.processes.is_empty()).then_some(
                self.view
                    .selected
                    .min(self.data.snapshot.processes.len().saturating_sub(1)),
            ),
            ViewMode::Tree => self
                .tree
                .rows
                .iter()
                .position(|row| row.process_index == self.view.selected),
        }
    }

    pub(super) fn handle_tree_left(&mut self) {
        if self.view.view_mode != ViewMode::Tree {
            return;
        }

        self.ensure_tree_selection_visible();
        let Some(current) = self.selected_visible_index() else {
            return;
        };

        let row = &self.tree.rows[current];
        let pid = self.data.snapshot.processes[row.process_index].pid;
        if row.has_children && row.expanded {
            self.tree.expanded_pids.remove(&pid);
            self.rebuild_tree_rows();
        } else if let Some(parent_index) = row.parent_index {
            self.view.selected = parent_index;
        }

        self.ensure_visible();
        self.populate_visible_details();
    }

    pub(super) fn handle_tree_right(&mut self) {
        if self.view.view_mode != ViewMode::Tree {
            return;
        }

        self.ensure_tree_selection_visible();
        let Some(current) = self.selected_visible_index() else {
            return;
        };

        let row = &self.tree.rows[current];
        let pid = self.data.snapshot.processes[row.process_index].pid;
        if row.has_children && !row.expanded {
            self.tree.expanded_pids.insert(pid);
            self.rebuild_tree_rows();
        } else if row.has_children && row.expanded {
            if let Some(next_row) = self.tree.rows.get(current + 1) {
                if next_row.parent_index == Some(row.process_index) {
                    self.view.selected = next_row.process_index;
                }
            }
        }

        self.ensure_visible();
        self.populate_visible_details();
    }

    pub(super) fn toggle_expansion(&mut self, process_index: usize) {
        let pid = self.data.snapshot.processes[process_index].pid;
        if !self.tree.expanded_pids.insert(pid) {
            self.tree.expanded_pids.remove(&pid);
        }

        self.rebuild_tree_rows();
        self.ensure_tree_selection_visible();
        self.ensure_visible();
        self.populate_visible_details();
    }

    pub(super) fn ensure_tree_selection_visible(&mut self) {
        if self.view.view_mode != ViewMode::Tree
            || self.view.selected >= self.data.snapshot.processes.len()
        {
            return;
        }

        let mut current = Some(self.view.selected);
        let mut changed = false;
        while let Some(index) =
            current.and_then(|index| self.tree.parents.get(index).copied().flatten())
        {
            let pid = self.data.snapshot.processes[index].pid;
            changed |= self.tree.expanded_pids.insert(pid);
            current = Some(index);
        }

        if changed {
            self.rebuild_tree_rows();
        }
    }

    pub(super) fn is_tree_toggle_click(&self, column: u16, row: &TreeRow) -> bool {
        if self.view.view_mode != ViewMode::Tree || !row.has_children {
            return false;
        }

        let Some((name_start, name_width)) = self.name_column_bounds() else {
            return false;
        };
        let Some((toggle_offset, toggle_width)) = row.name_toggle_range() else {
            return false;
        };

        let name_end = name_start.saturating_add(name_width.saturating_sub(1));
        if column < name_start || column > name_end {
            return false;
        }

        let toggle_start = name_start.saturating_add(toggle_offset);
        let toggle_end = toggle_start.saturating_add(toggle_width.saturating_sub(1));
        toggle_start <= column && column <= toggle_end
    }

    pub(super) fn name_column_bounds(&self) -> Option<(u16, u16)> {
        let area = self.view.process_table_area?;
        let column_widths = process_table_column_widths(self.view.view_mode);
        let mut start = area.x.saturating_add(1);
        for width in column_widths.iter().take(NAME_COLUMN_INDEX) {
            start = start
                .saturating_add(*width)
                .saturating_add(PROCESS_TABLE_COLUMN_SPACING);
        }

        Some((start, column_widths[NAME_COLUMN_INDEX]))
    }

    pub(super) fn rebuild_tree_rows(&mut self) {
        let len = self.data.snapshot.processes.len();
        self.tree.rows.clear();
        self.tree.parents = vec![None; len];
        if len == 0 {
            self.tree.expanded_pids.clear();
            return;
        }

        let mut pid_to_index = HashMap::with_capacity(len);
        for (index, row) in self.data.snapshot.processes.iter().enumerate() {
            pid_to_index.insert(row.pid, index);
        }
        self.tree
            .expanded_pids
            .retain(|pid| pid_to_index.contains_key(pid));

        let (mut roots, children) = build_tree_index(&self.data.snapshot.processes, &pid_to_index);
        self.tree.parents = build_parent_index(&self.data.snapshot.processes, &pid_to_index);

        let sort_state = self.view.sort_state;
        let username_cache = &self.resources.username_cache;
        let processes = &self.data.snapshot.processes;
        let sort_indexes = |indexes: &mut Vec<usize>| {
            indexes.sort_by(|left, right| {
                compare_process_rows(
                    sort_state,
                    username_cache,
                    &processes[*left],
                    &processes[*right],
                )
            });
        };
        sort_indexes(&mut roots);

        let mut children = children;
        for child_indexes in &mut children {
            sort_indexes(child_indexes);
        }

        self.tree.rows = build_visible_tree_rows(
            &self.data.snapshot.processes,
            &children,
            &self.tree.expanded_pids,
            &self.tree.parents,
            &roots,
        );
    }
}

impl TreeRow {
    fn flat(process_index: usize) -> Self {
        Self {
            process_index,
            depth: 0,
            has_children: false,
            expanded: false,
            parent_index: None,
            is_last_sibling: true,
            ancestor_has_next_sibling: Vec::new(),
        }
    }

    pub(crate) fn name_toggle_range(&self) -> Option<(u16, u16)> {
        if !self.has_children {
            return None;
        }

        let mut offset = (self.ancestor_has_next_sibling.len() as u16).saturating_mul(3);
        if self.depth > 0 {
            offset = offset.saturating_add(2);
        }
        Some((offset, 3))
    }
}

fn build_parent_index(
    processes: &[ProcessRow],
    pid_to_index: &HashMap<i32, usize>,
) -> Vec<Option<usize>> {
    processes
        .iter()
        .map(|row| match row.ppid {
            ppid if ppid <= 0 || ppid == row.pid => None,
            ppid => pid_to_index.get(&ppid).copied(),
        })
        .collect()
}

fn build_tree_index(
    processes: &[ProcessRow],
    pid_to_index: &HashMap<i32, usize>,
) -> (Vec<usize>, Vec<Vec<usize>>) {
    let parents = build_parent_index(processes, pid_to_index);
    let mut children = vec![Vec::new(); processes.len()];
    let mut roots = Vec::new();

    for (index, parent_index) in parents.into_iter().enumerate() {
        if let Some(parent_index) = parent_index {
            children[parent_index].push(index);
        } else {
            roots.push(index);
        }
    }

    (roots, children)
}

fn build_visible_tree_rows(
    processes: &[ProcessRow],
    children: &[Vec<usize>],
    expanded_pids: &HashSet<i32>,
    parents: &[Option<usize>],
    roots: &[usize],
) -> Vec<TreeRow> {
    fn push_visible_rows(
        rows: &mut Vec<TreeRow>,
        processes: &[ProcessRow],
        children: &[Vec<usize>],
        expanded_pids: &HashSet<i32>,
        parents: &[Option<usize>],
        index: usize,
        depth: usize,
        is_last_sibling: bool,
        ancestor_has_next_sibling: &[bool],
    ) {
        let has_children = !children[index].is_empty();
        let expanded = has_children && expanded_pids.contains(&processes[index].pid);
        rows.push(TreeRow {
            process_index: index,
            depth,
            has_children,
            expanded,
            parent_index: parents[index],
            is_last_sibling,
            ancestor_has_next_sibling: ancestor_has_next_sibling.to_vec(),
        });

        if expanded {
            let mut child_guides = ancestor_has_next_sibling.to_vec();
            child_guides.push(!is_last_sibling);
            let child_count = children[index].len();
            for (child_idx, &child) in children[index].iter().enumerate() {
                push_visible_rows(
                    rows,
                    processes,
                    children,
                    expanded_pids,
                    parents,
                    child,
                    depth + 1,
                    child_idx + 1 == child_count,
                    &child_guides,
                );
            }
        }
    }

    let mut rows = Vec::new();
    let root_count = roots.len();
    for (root_idx, root) in roots.iter().copied().enumerate() {
        push_visible_rows(
            &mut rows,
            processes,
            children,
            expanded_pids,
            parents,
            root,
            0,
            root_idx + 1 == root_count,
            &[],
        );
    }

    rows
}
