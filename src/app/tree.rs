use super::{
    AppState, NAME_COLUMN_INDEX, PROCESS_TABLE_COLUMN_SPACING, ViewMode,
    process_table_column_widths, sort::compare_process_rows, state::ProcessTreeState,
};
use crate::snapshot::{ProcessRow, SortState};
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
        visible_row_entries(
            self.view.view_mode,
            self.view.scroll_offset,
            self.view.viewport_rows,
            self.data.snapshot.processes.len(),
            &self.tree.rows,
        )
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
            ViewMode::Tree => selected_tree_visible_index(&self.tree.rows, self.view.selected),
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
        toggle_pid_expansion(&mut self.tree.expanded_pids, pid);

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

        let ancestors = expanded_ancestor_pids(
            self.view.selected,
            &self.tree.parents,
            &self.data.snapshot.processes,
        );
        let mut changed = false;
        for pid in ancestors {
            changed |= self.tree.expanded_pids.insert(pid);
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
        self.tree = build_process_tree_state(
            &self.data.snapshot.processes,
            self.view.sort_state,
            &self.resources.username_cache,
            &self.tree.expanded_pids,
        );
    }

    pub(super) fn sort_current_processes(&mut self) {
        super::sort::sort_processes(
            self.view.sort_state,
            &self.resources.username_cache,
            &mut self.data.snapshot.processes,
        );
    }

    pub(super) fn sort_snapshot_processes(&self, processes: &mut [ProcessRow]) {
        super::sort::sort_processes(
            self.view.sort_state,
            &self.resources.username_cache,
            processes,
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

pub(super) fn visible_row_entries(
    view_mode: ViewMode,
    scroll_offset: usize,
    viewport_rows: usize,
    process_count: usize,
    tree_rows: &[TreeRow],
) -> Vec<TreeRow> {
    match view_mode {
        ViewMode::Flat => {
            let end = scroll_offset
                .saturating_add(viewport_rows)
                .min(process_count);
            (scroll_offset.min(end)..end).map(TreeRow::flat).collect()
        }
        ViewMode::Tree => {
            let end = scroll_offset
                .saturating_add(viewport_rows)
                .min(tree_rows.len());
            tree_rows[scroll_offset.min(end)..end].to_vec()
        }
    }
}

pub(super) fn selected_tree_visible_index(tree_rows: &[TreeRow], selected: usize) -> Option<usize> {
    tree_rows
        .iter()
        .position(|row| row.process_index == selected)
}

pub(super) fn toggle_pid_expansion(expanded_pids: &mut HashSet<i32>, pid: i32) {
    if !expanded_pids.insert(pid) {
        expanded_pids.remove(&pid);
    }
}

pub(super) fn expanded_ancestor_pids(
    selected: usize,
    parents: &[Option<usize>],
    processes: &[ProcessRow],
) -> Vec<i32> {
    let mut ancestors = Vec::new();
    let mut current = Some(selected);
    while let Some(index) = current.and_then(|index| parents.get(index).copied().flatten()) {
        ancestors.push(processes[index].pid);
        current = Some(index);
    }
    ancestors
}

pub(super) fn build_process_tree_state(
    processes: &[ProcessRow],
    sort_state: SortState,
    username_cache: &HashMap<u32, String>,
    expanded_pids: &HashSet<i32>,
) -> ProcessTreeState {
    let len = processes.len();
    if len == 0 {
        return ProcessTreeState::new();
    }

    let pid_to_index = build_pid_to_index(processes);
    let expanded_pids = retain_known_expanded_pids(expanded_pids, &pid_to_index);
    let parents = build_parent_index(processes, &pid_to_index);
    let (mut roots, mut children) = build_tree_index(&parents, len);

    sort_process_indexes(sort_state, username_cache, processes, &mut roots);
    for child_indexes in &mut children {
        sort_process_indexes(sort_state, username_cache, processes, child_indexes);
    }

    let rows = build_visible_tree_rows(processes, &children, &expanded_pids, &parents, &roots);

    ProcessTreeState {
        expanded_pids,
        rows,
        parents,
    }
}

fn build_pid_to_index(processes: &[ProcessRow]) -> HashMap<i32, usize> {
    let mut pid_to_index = HashMap::with_capacity(processes.len());
    for (index, row) in processes.iter().enumerate() {
        pid_to_index.insert(row.pid, index);
    }
    pid_to_index
}

fn retain_known_expanded_pids(
    expanded_pids: &HashSet<i32>,
    pid_to_index: &HashMap<i32, usize>,
) -> HashSet<i32> {
    expanded_pids
        .iter()
        .copied()
        .filter(|pid| pid_to_index.contains_key(pid))
        .collect()
}

fn sort_process_indexes(
    sort_state: SortState,
    username_cache: &HashMap<u32, String>,
    processes: &[ProcessRow],
    indexes: &mut Vec<usize>,
) {
    indexes.sort_by(|left, right| {
        compare_process_rows(
            sort_state,
            username_cache,
            &processes[*left],
            &processes[*right],
        )
    });
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
    parents: &[Option<usize>],
    process_count: usize,
) -> (Vec<usize>, Vec<Vec<usize>>) {
    let mut children = vec![Vec::new(); process_count];
    let mut roots = Vec::new();

    for (index, parent_index) in parents.iter().copied().enumerate() {
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

#[cfg(test)]
mod tests {
    use super::{
        TreeRow, ViewMode, build_parent_index, build_process_tree_state, expanded_ancestor_pids,
        selected_tree_visible_index, visible_row_entries,
    };
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
    use std::collections::{HashMap, HashSet};

    fn tree_row(pid: i32, ppid: i32) -> ProcessRow {
        ProcessRow {
            pid,
            ppid,
            owner_uid: 0,
            threads: 1,
            name: format!("p{pid}"),
            command: format!("cmd{pid}"),
            rss_bytes: pid as u64,
            uss_bytes: None,
            pss_bytes: None,
            base_swap_bytes: 0,
            detailed_swap_bytes: None,
            cpu_percent: pid as f32,
        }
    }

    #[test]
    fn visible_row_entries_slices_flat_rows() {
        let rows = visible_row_entries(ViewMode::Flat, 1, 2, 5, &[]);
        let indexes: Vec<usize> = rows.into_iter().map(|row| row.process_index).collect();
        assert_eq!(indexes, vec![1, 2]);
    }

    #[test]
    fn selected_tree_visible_index_finds_selected_process() {
        let rows = vec![
            TreeRow {
                process_index: 4,
                depth: 0,
                has_children: true,
                expanded: true,
                parent_index: None,
                is_last_sibling: false,
                ancestor_has_next_sibling: Vec::new(),
            },
            TreeRow {
                process_index: 7,
                depth: 1,
                has_children: false,
                expanded: false,
                parent_index: Some(4),
                is_last_sibling: true,
                ancestor_has_next_sibling: vec![false],
            },
        ];

        assert_eq!(selected_tree_visible_index(&rows, 7), Some(1));
        assert_eq!(selected_tree_visible_index(&rows, 3), None);
    }

    #[test]
    fn expanded_ancestor_pids_returns_parent_chain() {
        let processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 2)];
        let parents = vec![None, Some(0), Some(1)];

        assert_eq!(expanded_ancestor_pids(2, &parents, &processes), vec![2, 1]);
    }

    #[test]
    fn build_parent_index_ignores_invalid_self_and_missing_parents() {
        let processes = vec![
            tree_row(1, 0),
            tree_row(2, 1),
            tree_row(3, 3),
            tree_row(4, 99),
        ];
        let pid_to_index = HashMap::from([(1, 0), (2, 1), (3, 2), (4, 3)]);

        assert_eq!(
            build_parent_index(&processes, &pid_to_index),
            vec![None, Some(0), None, None]
        );
    }

    #[test]
    fn build_process_tree_state_keeps_only_known_expanded_pids() {
        let processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        let expanded = HashSet::from([1, 99]);

        let state = build_process_tree_state(
            &processes,
            SortState::new(SortKey::Pid, SortDirection::Ascending),
            &HashMap::new(),
            &expanded,
        );

        assert_eq!(state.expanded_pids, HashSet::from([1]));
        let visible: Vec<i32> = state
            .rows
            .into_iter()
            .map(|row| processes[row.process_index].pid)
            .collect();
        assert_eq!(visible, vec![1, 2, 3]);
    }
}
