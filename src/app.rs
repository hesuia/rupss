mod input;
mod navigation;
mod runtime;
mod sort;
mod state;
mod tree;

use crate::{
    collector::{ProcfsCollector, SystemCollector},
    error::CollectorError,
    history::HistoryBuffer,
    snapshot::{ProcessRow, Snapshot, SortState, SystemSummary},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
pub use runtime::run;
use std::{
    collections::HashMap,
    fs,
    io::Stdout,
    time::{Duration, Instant},
};
use strum::{AsRefStr, IntoStaticStr};
pub(crate) use tree::TreeRow;

#[cfg(test)]
use self::sort::compare_process_rows;
// use self::sort::sort_processes;
use self::state::{AppDataState, AppResources, AppViewState, ProcessTreeState};

const HISTORY_CAPACITY: usize = 180;
const TICK_RATE: Duration = Duration::from_secs(1);
const EVENT_POLL: Duration = Duration::from_millis(250);
pub(crate) const PROCESS_TABLE_COLUMN_SPACING: u16 = 1;
const PROCESS_TABLE_COLUMN_WIDTHS_FLAT: [u16; 11] = [7, 7, 12, 8, 24, 24, 12, 12, 12, 12, 8];
const PROCESS_TABLE_COLUMN_WIDTHS_TREE: [u16; 11] = [7, 7, 12, 8, 32, 16, 12, 12, 12, 12, 8];
const NAME_COLUMN_INDEX: usize = 4;
type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

pub(crate) fn process_table_column_widths(view_mode: ViewMode) -> [u16; 11] {
    match view_mode {
        ViewMode::Flat => PROCESS_TABLE_COLUMN_WIDTHS_FLAT,
        ViewMode::Tree => PROCESS_TABLE_COLUMN_WIDTHS_TREE,
    }
}

/// Result of handling one key input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ViewMode {
    Flat,
    Tree,
}

impl ViewMode {
    pub fn toggle(self) -> Self {
        match self {
            ViewMode::Flat => ViewMode::Tree,
            ViewMode::Tree => ViewMode::Flat,
        }
    }
}

/// Mutable application state shared by the collector and the renderer.
///
/// The state is split into dedicated sub-structures so collection data,
/// navigation state, tree-derived rows, and external resources evolve
/// independently.
pub struct AppState {
    data: AppDataState,
    view: AppViewState,
    tree: ProcessTreeState,
    resources: AppResources,
}

impl AppState {
    /// Creates an empty application state with preallocated history buffers.
    ///
    /// The actual process list and system summary are populated by the first
    /// `refresh()` call.
    pub fn new(collector: ProcfsCollector) -> Self {
        Self {
            data: AppDataState::new(),
            view: AppViewState::new(),
            tree: ProcessTreeState::new(),
            resources: AppResources::new(collector, load_username_cache()),
        }
    }

    /// Refreshes the full snapshot and then loads detailed memory data for visible rows.
    ///
    /// The refresh flow:
    /// 1. Collect a cheap full-process snapshot and aggregate totals.
    /// 2. Sort the list and keep the previously selected PID if still present.
    /// 3. Update the history buffers for the top graphs.
    /// 4. Load `smaps_rollup` details only for the rows currently visible.
    pub fn refresh(&mut self) {
        let selected_pid = self.selected_pid();
        let now = Instant::now();
        match self
            .resources
            .collector
            .collect_base_snapshot(&self.data.previous_cpu, now)
        {
            Ok((mut snapshot, next_cpu, history_point)) => {
                self.data.previous_cpu = next_cpu;
                self.sort_snapshot_processes(&mut snapshot.processes);
                self.data.snapshot = snapshot;
                self.rebuild_tree_rows();
                self.restore_selection(selected_pid, false);
                self.push_history(history_point);
                self.populate_visible_details();
                self.data.last_error = None;
            }
            Err(error) => {
                self.data.last_error = Some(error);
            }
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.data.snapshot
    }

    pub fn sort_state(&self) -> SortState {
        self.view.sort_state
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view.view_mode
    }

    pub fn scroll_offset(&self) -> usize {
        self.view.scroll_offset
    }

    pub fn system_summary(&self) -> &SystemSummary {
        &self.data.snapshot.system
    }

    pub fn process_row(&self, index: usize) -> Option<&ProcessRow> {
        self.data.snapshot.processes.get(index)
    }

    /// Returns the most recent collection error, if one is being shown in the UI.
    pub fn last_error(&self) -> Option<&CollectorError> {
        self.data.last_error.as_ref()
    }

    /// Returns the most recent collection error as display text for the summary panel.
    pub fn last_error_message(&self) -> Option<String> {
        self.last_error().map(ToString::to_string)
    }

    /// Updates the number of table rows that fit on screen.
    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.view.viewport_rows = rows.max(1);
        self.clamp_scroll_offset();
    }

    /// Stores the process table area from the most recent frame.
    pub fn set_process_table_area(&mut self, area: Rect) {
        self.view.process_table_area = Some(area);
    }

    /// Resolves a UID into a cached display name or falls back to the numeric UID.
    pub fn owner_name(&self, uid: u32) -> String {
        self.resources
            .username_cache
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string())
    }

    /// Returns the PID of the currently selected row, if any.
    pub fn selected_pid(&self) -> Option<i32> {
        self.data
            .snapshot
            .processes
            .get(self.view.selected)
            .map(|row| row.pid)
    }

    /// Builds chart points for the RSS history graph.
    pub fn rss_chart_points(&self) -> Vec<(f64, f64)> {
        history_points(&self.data.rss_history)
    }

    /// Builds chart points for the swap history graph.
    pub fn swap_chart_points(&self) -> Vec<(f64, f64)> {
        history_points(&self.data.swap_history)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(ProcfsCollector::new())
    }
}

/// Converts the fixed-size history buffer into chart coordinates.
///
/// X coordinates are sample indices, Y coordinates are raw bytes.
fn history_points(history: &HistoryBuffer<u64>) -> Vec<(f64, f64)> {
    history
        .into_iter()
        .enumerate()
        .map(|(idx, value)| (idx as f64, *value as f64))
        .collect()
}

/// Loads a small UID-to-name cache from `/etc/passwd` for owner display.
fn load_username_cache() -> HashMap<u32, String> {
    let Ok(contents) = fs::read_to_string("/etc/passwd") else {
        return HashMap::new();
    };

    let mut users = HashMap::new();
    for line in contents.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(':');
        let Some(name) = fields.next() else {
            continue;
        };
        let _password = fields.next();
        let Some(uid) = fields.next() else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else {
            continue;
        };
        users.insert(uid, name.to_string());
    }
    users
}

#[cfg(test)]
mod tests {
    use super::{AppState, KeyAction, TreeRow, ViewMode, compare_process_rows, history_points};
    use crate::collector::ProcfsCollector;
    use crate::error::CollectorError;
    use crate::history::HistoryBuffer;
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use procfs::ProcError;
    use ratatui::layout::Rect;
    use std::{collections::HashMap, io};

    fn sample_row(pid: i32) -> ProcessRow {
        ProcessRow {
            pid,
            ppid: 1,
            owner_uid: 0,
            threads: 1,
            name: "name".to_string(),
            command: "cmd".to_string(),
            rss_bytes: pid as u64,
            uss_bytes: None,
            pss_bytes: Some(pid as u64),
            base_swap_bytes: pid as u64,
            detailed_swap_bytes: None,
            cpu_percent: pid as f32,
        }
    }

    fn tree_row(pid: i32, ppid: i32) -> ProcessRow {
        let mut row = sample_row(pid);
        row.ppid = ppid;
        row
    }

    #[test]
    fn chart_points_preserve_order() {
        let mut history = HistoryBuffer::new(4);
        history.push(10);
        history.push(20);
        let points = history_points(&history);
        assert_eq!(points, vec![(0.0, 10.0), (1.0, 20.0)]);
    }

    #[test]
    fn sort_prefers_highest_metric() {
        let ordering = compare_process_rows(
            SortState::new(SortKey::Cpu, SortDirection::Descending),
            &HashMap::new(),
            &sample_row(10),
            &sample_row(20),
        );
        assert_eq!(ordering, std::cmp::Ordering::Greater);
    }

    #[test]
    fn sort_by_pid_is_ascending() {
        let ordering = compare_process_rows(
            SortState::new(SortKey::Pid, SortDirection::Ascending),
            &HashMap::new(),
            &sample_row(10),
            &sample_row(20),
        );
        assert_eq!(ordering, std::cmp::Ordering::Less);
    }

    #[test]
    fn sort_by_owner_is_case_insensitive() {
        let mut owners = HashMap::new();
        owners.insert(1000, "Alice".to_string());
        owners.insert(1001, "bob".to_string());

        let mut left = sample_row(10);
        left.owner_uid = 1000;
        let mut right = sample_row(20);
        right.owner_uid = 1001;

        let ordering = compare_process_rows(
            SortState::new(SortKey::Owner, SortDirection::Ascending),
            &owners,
            &left,
            &right,
        );
        assert_eq!(ordering, std::cmp::Ordering::Less);
    }

    #[test]
    fn sort_direction_toggles_on_same_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.view.sort_state.direction, SortDirection::Ascending);

        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.view.sort_state.direction, SortDirection::Descending);
    }

    #[test]
    fn sort_direction_resets_on_new_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(KeyCode::Char('i'));
        assert_eq!(app.view.sort_state.key, SortKey::Pid);
        assert_eq!(app.view.sort_state.direction, SortDirection::Ascending);
    }

    #[test]
    fn j_and_k_move_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2), sample_row(3)];
        app.set_viewport_rows(3);
        app.view.selected = 1;

        assert_eq!(app.handle_key(KeyCode::Char('j')), KeyAction::Continue);
        assert_eq!(app.view.selected, 2);

        assert_eq!(app.handle_key(KeyCode::Char('j')), KeyAction::Continue);
        assert_eq!(app.view.selected, 2);

        assert_eq!(app.handle_key(KeyCode::Char('k')), KeyAction::Continue);
        assert_eq!(app.view.selected, 1);

        assert_eq!(app.handle_key(KeyCode::Char('k')), KeyAction::Continue);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn q_requests_quit() {
        let mut app = AppState::new(ProcfsCollector::new());
        assert_eq!(app.handle_key(KeyCode::Char('q')), KeyAction::Quit);
    }

    #[test]
    fn left_click_selects_visible_row() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.set_viewport_rows(3);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.selected, 1);
    }

    #[test]
    fn click_outside_data_rows_does_not_change_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.set_viewport_rows(3);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 2;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 7,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.selected, 2);
    }

    #[test]
    fn wheel_scroll_up_changes_offset_not_selected() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 3;
        app.view.scroll_offset = 2;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
        assert_eq!(app.view.selected, 3);
    }

    #[test]
    fn wheel_scroll_down_changes_offset_not_selected() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 0;
        app.view.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn wheel_scroll_clamps_at_bounds() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.view.scroll_offset, 0);

        app.view.scroll_offset = 2;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.view.scroll_offset, 2);
    }

    #[test]
    fn wheel_outside_table_does_not_change_state() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 2;
        app.view.scroll_offset = 1;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 41,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
        assert_eq!(app.view.selected, 2);
    }

    #[test]
    fn wheel_on_table_border_is_handled() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(5, 2, 40, 8));
        app.view.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
    }

    #[test]
    fn set_viewport_rows_does_not_force_selected_visible() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
            sample_row(50),
        ];
        app.view.selected = 0;
        app.view.scroll_offset = 2;

        app.set_viewport_rows(2);

        assert_eq!(app.view.scroll_offset, 2);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn restore_selection_without_ensure_visible_keeps_scroll_offset() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
            sample_row(50),
        ];
        app.set_viewport_rows(2);
        app.view.selected = 0;
        app.view.scroll_offset = 3;

        app.restore_selection(Some(10), false);

        assert_eq!(app.view.selected, 0);
        assert_eq!(app.view.scroll_offset, 3);
    }

    #[test]
    fn tree_mode_starts_with_roots_only() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![
            tree_row(1, 0),
            tree_row(2, 1),
            tree_row(3, 1),
            tree_row(4, 0),
        ];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));

        let visible: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();

        assert_eq!(app.view.view_mode, ViewMode::Tree);
        assert_eq!(visible, vec![1, 4]);
    }

    #[test]
    fn tree_right_expands_and_left_collapses() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 2)];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));

        app.handle_key(KeyCode::Right);
        let visible_after_expand: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible_after_expand, vec![1, 2]);

        app.handle_key(KeyCode::Left);
        let visible_after_collapse: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible_after_collapse, vec![1]);
    }

    #[test]
    fn tree_gutter_click_toggles_expansion() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));
        app.set_viewport_rows(5);
        app.set_process_table_area(Rect::new(0, 0, 120, 8));
        let (name_start, _) = app.name_column_bounds().unwrap();

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: name_start,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });

        let visible: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible, vec![1, 2, 3]);
    }

    #[test]
    fn tree_name_branch_click_selects_without_toggling() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.rebuild_tree_rows();
        app.tree.expanded_pids.insert(1);
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));
        app.set_viewport_rows(5);
        app.set_process_table_area(Rect::new(0, 0, 120, 8));
        let (name_start, _) = app.name_column_bounds().unwrap();

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: name_start,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.selected_pid(), Some(2));
        let visible: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible, vec![1, 2, 3]);
    }

    #[test]
    fn tree_sort_reorders_siblings_without_breaking_hierarchy() {
        let mut app = AppState::new(ProcfsCollector::new());
        let mut parent = tree_row(1, 0);
        parent.rss_bytes = 100;
        let mut child_a = tree_row(2, 1);
        child_a.rss_bytes = 10;
        let mut child_b = tree_row(3, 1);
        child_b.rss_bytes = 50;
        app.data.snapshot.processes = vec![parent, child_a, child_b];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));
        app.handle_key(KeyCode::Right);

        let visible: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();

        assert_eq!(visible, vec![1, 3, 2]);
    }

    #[test]
    fn tree_mode_keeps_multiple_unresolved_roots_visible() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![
            tree_row(1, 0),
            tree_row(2, 9999),
            tree_row(3, -1),
            tree_row(4, 4),
            tree_row(5, 1),
        ];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));

        let visible: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.data.snapshot.processes[entry.process_index].pid)
            .collect();

        assert_eq!(visible, vec![1, 2, 3, 4]);
    }

    #[test]
    fn tree_child_of_non_last_root_tracks_root_vertical_guide() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.tree.expanded_pids.insert(1);
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));

        let child = app
            .visible_row_entries()
            .into_iter()
            .find(|entry| app.data.snapshot.processes[entry.process_index].pid == 2)
            .unwrap();

        assert_eq!(child.ancestor_has_next_sibling, vec![true]);
    }

    #[test]
    fn root_tree_toggle_starts_at_name_column() {
        let row = TreeRow {
            process_index: 0,
            depth: 0,
            has_children: true,
            expanded: false,
            parent_index: None,
            is_last_sibling: false,
            ancestor_has_next_sibling: Vec::new(),
        };

        assert_eq!(row.name_toggle_range(), Some((0, 3)));
    }

    #[test]
    fn last_error_keeps_typed_collector_error() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.last_error = Some(CollectorError::ReadMeminfo(ProcError::from(
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        )));

        assert!(matches!(
            app.last_error(),
            Some(CollectorError::ReadMeminfo(_))
        ));
    }

    #[test]
    fn last_error_message_formats_collector_error_for_display() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.last_error = Some(CollectorError::ListProcesses(ProcError::from(
            io::Error::new(io::ErrorKind::NotFound, "missing"),
        )));

        assert_eq!(
            app.last_error_message().as_deref(),
            Some("failed to enumerate /proc processes")
        );
    }
}
