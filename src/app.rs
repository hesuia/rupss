use crate::{
    collector::{CpuSample, ProcfsCollector, SystemCollector},
    history::HistoryBuffer,
    snapshot::{
        HistoryPoint, ProcessRow, Snapshot, SortDirection, SortKey, SortState, SystemSummary,
    },
    tui,
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    io::{self, Stdout},
    time::{Duration, Instant},
};
use strum::{AsRefStr, IntoStaticStr};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
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

/// Mutable application state shared by the collector and the renderer.
///
/// Responsibilities:
/// - Owns the latest `Snapshot` produced by the collector.
/// - Keeps UI state (selection, scroll offset, active sort key).
/// - Maintains fixed-size history buffers for the top graphs.
/// - Holds a PID -> CPU sample cache so per-process CPU usage can be derived
///   from two consecutive samples.
/// - Tracks the last collection error for display instead of crashing the UI.
pub struct AppState {
    /// Latest collected snapshot shown in the UI.
    pub snapshot: Snapshot,
    /// Sort state for key and direction.
    pub sort_state: SortState,
    /// Absolute index of the selected row in `snapshot.processes`.
    pub selected: usize,
    /// Absolute start index of the visible table window.
    pub scroll_offset: usize,
    pub view_mode: ViewMode,
    viewport_rows: usize,
    username_cache: HashMap<u32, String>,
    rss_history: HistoryBuffer<u64>,
    swap_history: HistoryBuffer<u64>,
    previous_cpu: HashMap<i32, CpuSample>,
    expanded_pids: HashSet<i32>,
    tree_rows: Vec<TreeRow>,
    tree_parents: Vec<Option<usize>>,
    collector: ProcfsCollector,
    process_table_area: Option<Rect>,
    /// Most recent collection error kept for on-screen display.
    pub last_error: Option<String>,
}

/// Runs the TUI application until the user quits.
///
/// This sets up the terminal, runs the event loop, and ensures the terminal is
/// restored even when the loop exits due to user input.
pub fn run() -> io::Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn run_app(terminal: &mut CrosstermTerminal) -> io::Result<()> {
    let collector = ProcfsCollector::new();
    let mut app = AppState::new(collector);
    app.refresh()?;

    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| tui::render(frame, &mut app))?;

        if event::poll(EVENT_POLL)? {
            match event::read()? {
                Event::Key(key) => {
                    if matches!(key.kind, KeyEventKind::Press) {
                        match app.handle_key(key.code) {
                            KeyAction::Quit => return Ok(()),
                            KeyAction::Continue => {}
                        }
                    }
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            app.refresh()?;
            last_tick = Instant::now();
        }
    }
}

impl AppState {
    /// Creates an empty application state with preallocated history buffers.
    ///
    /// The actual process list and system summary are populated by the first
    /// `refresh()` call.
    pub fn new(collector: ProcfsCollector) -> Self {
        Self {
            snapshot: Snapshot {
                captured_at: Instant::now(),
                system: SystemSummary {
                    mem_total: 0,
                    mem_available: None,
                    mem_free: 0,
                    mem_used: 0,
                    swap_total: 0,
                    swap_free: 0,
                    swap_used: 0,
                    total_process_rss: 0,
                    total_process_swap: 0,
                    process_count: 0,
                },
                processes: Vec::new(),
            },
            sort_state: SortState::new(SortKey::Rss, SortDirection::Descending),
            selected: 0,
            scroll_offset: 0,
            view_mode: ViewMode::Flat,
            viewport_rows: 20,
            username_cache: load_username_cache(),
            rss_history: HistoryBuffer::new(HISTORY_CAPACITY),
            swap_history: HistoryBuffer::new(HISTORY_CAPACITY),
            previous_cpu: HashMap::new(),
            expanded_pids: HashSet::new(),
            tree_rows: Vec::new(),
            tree_parents: Vec::new(),
            collector,
            process_table_area: None,
            last_error: None,
        }
    }

    /// Refreshes the full snapshot and then loads detailed memory data for visible rows.
    ///
    /// The refresh flow:
    /// 1. Collect a cheap full-process snapshot and aggregate totals.
    /// 2. Sort the list and keep the previously selected PID if still present.
    /// 3. Update the history buffers for the top graphs.
    /// 4. Load `smaps_rollup` details only for the rows currently visible.
    pub fn refresh(&mut self) -> io::Result<()> {
        let selected_pid = self.selected_pid();
        let now = Instant::now();
        match self
            .collector
            .collect_base_snapshot(&self.previous_cpu, now)
        {
            Ok((mut snapshot, next_cpu, history_point)) => {
                self.previous_cpu = next_cpu;
                sort_processes(
                    self.sort_state,
                    &self.username_cache,
                    &mut snapshot.processes,
                );
                self.snapshot = snapshot;
                self.rebuild_tree_rows();
                self.restore_selection(selected_pid, false);
                self.push_history(history_point);
                self.populate_visible_details();
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                Ok(())
            }
        }
    }

    /// Updates the number of table rows that fit on screen.
    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
        self.clamp_scroll_offset();
    }

    /// Stores the process table area from the most recent frame.
    pub fn set_process_table_area(&mut self, area: Rect) {
        self.process_table_area = Some(area);
    }

    /// Resolves a UID into a cached display name or falls back to the numeric UID.
    pub fn owner_name(&self, uid: u32) -> String {
        self.username_cache
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string())
    }

    /// Returns the PID of the currently selected row, if any.
    pub fn selected_pid(&self) -> Option<i32> {
        self.snapshot
            .processes
            .get(self.selected)
            .map(|row| row.pid)
    }

    pub(crate) fn visible_row_entries(&self) -> Vec<TreeRow> {
        match self.view_mode {
            ViewMode::Flat => {
                let end = self
                    .scroll_offset
                    .saturating_add(self.viewport_rows)
                    .min(self.snapshot.processes.len());
                (self.scroll_offset.min(end)..end)
                    .map(|process_index| TreeRow {
                        process_index,
                        depth: 0,
                        has_children: false,
                        expanded: false,
                        parent_index: None,
                        is_last_sibling: true,
                        ancestor_has_next_sibling: Vec::new(),
                    })
                    .collect()
            }
            ViewMode::Tree => {
                let end = self
                    .scroll_offset
                    .saturating_add(self.viewport_rows)
                    .min(self.tree_rows.len());
                self.tree_rows[self.scroll_offset.min(end)..end].to_vec()
            }
        }
    }

    pub fn total_visible_rows(&self) -> usize {
        match self.view_mode {
            ViewMode::Flat => self.snapshot.processes.len(),
            ViewMode::Tree => self.tree_rows.len(),
        }
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        match self.view_mode {
            ViewMode::Flat => {
                if self.snapshot.processes.is_empty() {
                    None
                } else {
                    Some(
                        self.selected
                            .min(self.snapshot.processes.len().saturating_sub(1)),
                    )
                }
            }
            ViewMode::Tree => self
                .tree_rows
                .iter()
                .position(|row| row.process_index == self.selected),
        }
    }

    /// Builds chart points for the RSS history graph.
    pub fn rss_chart_points(&self) -> Vec<(f64, f64)> {
        history_points(&self.rss_history)
    }

    /// Builds chart points for the swap history graph.
    pub fn swap_chart_points(&self) -> Vec<(f64, f64)> {
        history_points(&self.swap_history)
    }

    /// Applies one keyboard action.
    ///
    /// Returns a [`KeyAction`] that tells the caller whether to continue
    /// running or terminate the application.
    /// The mapping is intentionally small and focused on fast navigation.
    pub fn handle_key(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Char('q') => KeyAction::Quit,
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                KeyAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                KeyAction::Continue
            }
            KeyCode::PageUp => {
                let page = self.viewport_rows.max(1) as isize;
                self.move_selection(-page);
                KeyAction::Continue
            }
            KeyCode::PageDown => {
                let page = self.viewport_rows.max(1) as isize;
                self.move_selection(page);
                KeyAction::Continue
            }
            KeyCode::Home => {
                self.selected = match self.view_mode {
                    ViewMode::Flat => 0,
                    ViewMode::Tree => self
                        .tree_rows
                        .first()
                        .map(|row| row.process_index)
                        .unwrap_or(0),
                };
                self.ensure_visible();
                self.populate_visible_details();
                KeyAction::Continue
            }
            KeyCode::End => {
                self.selected = match self.view_mode {
                    ViewMode::Flat => self.snapshot.processes.len().saturating_sub(1),
                    ViewMode::Tree => self
                        .tree_rows
                        .last()
                        .map(|row| row.process_index)
                        .unwrap_or(0),
                };
                self.ensure_visible();
                self.populate_visible_details();
                KeyAction::Continue
            }
            KeyCode::Left => {
                self.handle_tree_left();
                KeyAction::Continue
            }
            KeyCode::Right => {
                self.handle_tree_right();
                KeyAction::Continue
            }
            KeyCode::Char('t') => {
                self.toggle_view_mode();
                KeyAction::Continue
            }
            KeyCode::Char('i') => {
                self.resort(SortKey::Pid);
                KeyAction::Continue
            }
            KeyCode::Char('p') => {
                self.resort(SortKey::Ppid);
                KeyAction::Continue
            }
            KeyCode::Char('o') => {
                self.resort(SortKey::Owner);
                KeyAction::Continue
            }
            KeyCode::Char('n') => {
                self.resort(SortKey::Name);
                KeyAction::Continue
            }
            KeyCode::Char('m') => {
                self.resort(SortKey::Command);
                KeyAction::Continue
            }
            KeyCode::Char('r') => {
                self.resort(SortKey::Rss);
                KeyAction::Continue
            }
            KeyCode::Char('s') => {
                self.resort(SortKey::Swap);
                KeyAction::Continue
            }
            KeyCode::Char('c') => {
                self.resort(SortKey::Cpu);
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    /// Applies one mouse action.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.select_process_at(mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => {
                self.scroll_with_wheel(mouse.column, mouse.row, -1);
            }
            MouseEventKind::ScrollDown => {
                self.scroll_with_wheel(mouse.column, mouse.row, 1);
            }
            _ => {}
        }
    }

    fn resort(&mut self, sort_key: SortKey) {
        if self.sort_state.key == sort_key {
            self.sort_state.toggle_direction();
        } else {
            self.sort_state = SortState::default_for_key(sort_key);
        }
        let selected_pid = self.selected_pid();
        sort_processes(
            self.sort_state,
            &self.username_cache,
            &mut self.snapshot.processes,
        );
        self.rebuild_tree_rows();
        self.restore_selection(selected_pid, true);
        self.populate_visible_details();
    }

    fn move_selection(&mut self, delta: isize) {
        if self.total_visible_rows() == 0 {
            return;
        }
        if self.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }

        let Some(current) = self.selected_visible_index() else {
            return;
        };
        let max_index = self.total_visible_rows().saturating_sub(1) as isize;
        let next = (current as isize + delta).clamp(0, max_index) as usize;
        self.selected = match self.view_mode {
            ViewMode::Flat => next,
            ViewMode::Tree => self.tree_rows[next].process_index,
        };
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn select_process_at(&mut self, column: u16, row: u16) {
        let Some(area) = self.process_table_area else {
            return;
        };
        if area.width < 3 || area.height < 4 {
            return;
        }

        // Require clicks in the table body (inside borders, excluding header).
        let left = area.x.saturating_add(1);
        let right = area.x.saturating_add(area.width.saturating_sub(2));
        if column < left || column > right {
            return;
        }
        let first_data_row = area.y.saturating_add(2);
        let last_data_row = area.y.saturating_add(area.height.saturating_sub(2));
        if row < first_data_row || row > last_data_row {
            return;
        }

        let data_row = row.saturating_sub(first_data_row) as usize;
        let visible_rows = self.visible_row_entries();
        if data_row >= visible_rows.len() {
            return;
        }

        let clicked = &visible_rows[data_row];
        if self.is_tree_toggle_click(column, clicked) {
            self.selected = clicked.process_index;
            self.toggle_expansion(clicked.process_index);
            return;
        }

        self.selected = clicked.process_index;
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn scroll_with_wheel(&mut self, column: u16, row: u16, delta: isize) {
        if !self.is_inside_process_table(column, row) {
            return;
        }
        if self.total_visible_rows() == 0 {
            return;
        }

        if delta < 0 {
            self.scroll_offset = self.scroll_offset.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll_offset = self.scroll_offset.saturating_add(delta as usize);
        }
        self.clamp_scroll_offset();
        self.populate_visible_details();
    }

    fn is_inside_process_table(&self, column: u16, row: u16) -> bool {
        let Some(area) = self.process_table_area else {
            return false;
        };
        let right = area.x.saturating_add(area.width.saturating_sub(1));
        let bottom = area.y.saturating_add(area.height.saturating_sub(1));
        column >= area.x && column <= right && row >= area.y && row <= bottom
    }

    fn ensure_visible(&mut self) {
        // Keep the selected row inside the current viewport after movement or resize.
        let selected_visible = match self.view_mode {
            ViewMode::Flat => self.selected,
            ViewMode::Tree => self.selected_visible_index().unwrap_or(0),
        };

        if selected_visible < self.scroll_offset {
            self.scroll_offset = selected_visible;
        }

        let view_end = self.scroll_offset.saturating_add(self.viewport_rows);
        if selected_visible >= view_end {
            self.scroll_offset =
                selected_visible.saturating_sub(self.viewport_rows.saturating_sub(1));
        }
        self.clamp_scroll_offset();
    }

    fn clamp_scroll_offset(&mut self) {
        let max_offset = self
            .total_visible_rows()
            .saturating_sub(self.viewport_rows.max(1));
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    fn visible_pids(&self) -> Vec<i32> {
        self.visible_row_entries()
            .into_iter()
            .map(|entry| self.snapshot.processes[entry.process_index].pid)
            .collect()
    }

    fn populate_visible_details(&mut self) {
        let pids = self.visible_pids();
        if pids.is_empty() {
            return;
        }
        let details = self.collector.collect_visible_memory_details(&pids);

        for row in self.snapshot.processes.iter_mut() {
            if let Some(detail) = details.get(&row.pid) {
                row.uss_bytes = detail.uss_bytes;
                row.pss_bytes = detail.pss_bytes;
                row.detailed_swap_bytes = detail.swap_bytes;
            }
        }
    }

    fn restore_selection(&mut self, selected_pid: Option<i32>, ensure_visible: bool) {
        if self.snapshot.processes.is_empty() {
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }

        // Keep the same PID selected across refreshes when it is still present.
        self.selected = selected_pid
            .and_then(|pid| {
                self.snapshot
                    .processes
                    .iter()
                    .position(|row| row.pid == pid)
            })
            .unwrap_or(0)
            .min(self.snapshot.processes.len().saturating_sub(1));
        if self.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }
        if ensure_visible {
            self.ensure_visible();
        } else {
            self.clamp_scroll_offset();
        }
    }

    fn push_history(&mut self, point: HistoryPoint) {
        self.rss_history.push(point.rss_bytes);
        self.swap_history.push(point.swap_bytes);
    }

    fn toggle_view_mode(&mut self) {
        self.view_mode = self.view_mode.toggle();
        if self.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn handle_tree_left(&mut self) {
        if self.view_mode != ViewMode::Tree {
            return;
        }
        self.ensure_tree_selection_visible();
        let Some(current) = self.selected_visible_index() else {
            return;
        };
        let row = &self.tree_rows[current];
        let pid = self.snapshot.processes[row.process_index].pid;
        if row.has_children && row.expanded {
            self.expanded_pids.remove(&pid);
            self.rebuild_tree_rows();
        } else if let Some(parent_index) = row.parent_index {
            self.selected = parent_index;
        }
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn handle_tree_right(&mut self) {
        if self.view_mode != ViewMode::Tree {
            return;
        }
        self.ensure_tree_selection_visible();
        let Some(current) = self.selected_visible_index() else {
            return;
        };
        let row = &self.tree_rows[current];
        let pid = self.snapshot.processes[row.process_index].pid;
        if row.has_children && !row.expanded {
            self.expanded_pids.insert(pid);
            self.rebuild_tree_rows();
        } else if row.has_children && row.expanded {
            if let Some(next_row) = self.tree_rows.get(current + 1) {
                if next_row.parent_index == Some(row.process_index) {
                    self.selected = next_row.process_index;
                }
            }
        }
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn toggle_expansion(&mut self, process_index: usize) {
        let pid = self.snapshot.processes[process_index].pid;
        if !self.expanded_pids.insert(pid) {
            self.expanded_pids.remove(&pid);
        }
        self.rebuild_tree_rows();
        self.ensure_tree_selection_visible();
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn ensure_tree_selection_visible(&mut self) {
        if self.view_mode != ViewMode::Tree || self.selected >= self.snapshot.processes.len() {
            return;
        }

        let mut current = Some(self.selected);
        let mut changed = false;
        while let Some(index) =
            current.and_then(|index| self.tree_parents.get(index).copied().flatten())
        {
            let pid = self.snapshot.processes[index].pid;
            changed |= self.expanded_pids.insert(pid);
            current = Some(index);
        }
        if changed {
            self.rebuild_tree_rows();
        }
    }

    fn is_tree_toggle_click(&self, column: u16, row: &TreeRow) -> bool {
        if self.view_mode != ViewMode::Tree || !row.has_children {
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
        column >= toggle_start && column <= toggle_end
    }

    fn name_column_bounds(&self) -> Option<(u16, u16)> {
        let area = self.process_table_area?;
        let column_widths = process_table_column_widths(self.view_mode);
        let mut start = area.x.saturating_add(1);
        for width in column_widths.iter().take(NAME_COLUMN_INDEX) {
            start = start
                .saturating_add(*width)
                .saturating_add(PROCESS_TABLE_COLUMN_SPACING);
        }
        Some((start, column_widths[NAME_COLUMN_INDEX]))
    }

    fn rebuild_tree_rows(&mut self) {
        let len = self.snapshot.processes.len();
        self.tree_rows.clear();
        self.tree_parents = vec![None; len];
        if len == 0 {
            self.expanded_pids.clear();
            return;
        }

        let mut pid_to_index = HashMap::with_capacity(len);
        for (index, row) in self.snapshot.processes.iter().enumerate() {
            pid_to_index.insert(row.pid, index);
        }

        self.expanded_pids
            .retain(|pid| pid_to_index.contains_key(pid));

        let mut children = vec![Vec::new(); len];
        let mut roots = Vec::new();
        for (index, row) in self.snapshot.processes.iter().enumerate() {
            let parent_index = if row.ppid <= 0 || row.ppid == row.pid {
                None
            } else {
                pid_to_index.get(&row.ppid).copied()
            };
            if let Some(parent_index) = parent_index {
                self.tree_parents[index] = Some(parent_index);
                children[parent_index].push(index);
            } else {
                roots.push(index);
            }
        }

        let sort_state = self.sort_state;
        let username_cache = &self.username_cache;
        let processes = &self.snapshot.processes;
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
        for child_indexes in &mut children {
            sort_indexes(child_indexes);
        }

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
                let child_count: usize = children[index].len();
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

        let root_count = roots.len();
        for (root_idx, root) in roots.into_iter().enumerate() {
            push_visible_rows(
                &mut self.tree_rows,
                &self.snapshot.processes,
                &children,
                &self.expanded_pids,
                &self.tree_parents,
                root,
                0,
                root_idx + 1 == root_count,
                &[],
            );
        }
    }
}

impl TreeRow {
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

/// Compares two rows using the active sort key and PID as a stable tie-breaker.
///
/// A deterministic tie-breaker keeps the table stable across refreshes.
fn compare_process_rows(
    sort_state: SortState,
    username_cache: &HashMap<u32, String>,
    left: &ProcessRow,
    right: &ProcessRow,
) -> Ordering {
    fn owner_display_name<'a>(username_cache: &'a HashMap<u32, String>, uid: u32) -> Cow<'a, str> {
        username_cache
            .get(&uid)
            .map(|name| Cow::Borrowed(name.as_str()))
            .unwrap_or_else(|| Cow::Owned(format!("uid:{}", uid)))
    }
    let primary = match sort_state.key {
        SortKey::Pid => left.pid.cmp(&right.pid),
        SortKey::Ppid => left.ppid.cmp(&right.ppid),
        SortKey::Owner => {
            let left_owner = owner_display_name(username_cache, left.owner_uid);
            let right_owner = owner_display_name(username_cache, right.owner_uid);
            left_owner.cmp(&right_owner)
        }
        SortKey::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortKey::Command => left
            .command
            .to_lowercase()
            .cmp(&right.command.to_lowercase()),
        SortKey::Rss => left.rss_bytes.cmp(&right.rss_bytes),
        SortKey::Swap => left.visible_swap_bytes().cmp(&right.visible_swap_bytes()),
        SortKey::Cpu => left
            .cpu_percent
            .partial_cmp(&right.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    let primary = match sort_state.direction {
        SortDirection::Ascending => primary,
        SortDirection::Descending => primary.reverse(),
    };

    primary.then_with(|| left.pid.cmp(&right.pid))
}

/// Sorts the process list in place using the current table policy.
///
/// Sorting is done after each refresh and whenever the user changes sort key.
fn sort_processes(
    sort_state: SortState,
    username_cache: &HashMap<u32, String>,
    processes: &mut [ProcessRow],
) {
    processes.sort_by(|left, right| compare_process_rows(sort_state, username_cache, left, right));
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

/// Switches the terminal into raw mode and enters the alternate screen.
fn setup_terminal() -> io::Result<CrosstermTerminal> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

/// Restores the terminal back to the normal shell state.
fn restore_terminal(terminal: &mut CrosstermTerminal) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppState, KeyAction, TreeRow, ViewMode, compare_process_rows, history_points};
    use crate::collector::ProcfsCollector;
    use crate::history::HistoryBuffer;
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;
    use std::collections::HashMap;

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
        app.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.sort_state.direction, SortDirection::Ascending);

        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.sort_state.direction, SortDirection::Descending);
    }

    #[test]
    fn sort_direction_resets_on_new_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(KeyCode::Char('i'));
        assert_eq!(app.sort_state.key, SortKey::Pid);
        assert_eq!(app.sort_state.direction, SortDirection::Ascending);
    }

    #[test]
    fn j_and_k_move_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![sample_row(1), sample_row(2), sample_row(3)];
        app.set_viewport_rows(3);
        app.selected = 1;

        assert_eq!(app.handle_key(KeyCode::Char('j')), KeyAction::Continue);
        assert_eq!(app.selected, 2);

        assert_eq!(app.handle_key(KeyCode::Char('j')), KeyAction::Continue);
        assert_eq!(app.selected, 2);

        assert_eq!(app.handle_key(KeyCode::Char('k')), KeyAction::Continue);
        assert_eq!(app.selected, 1);

        assert_eq!(app.handle_key(KeyCode::Char('k')), KeyAction::Continue);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn q_requests_quit() {
        let mut app = AppState::new(ProcfsCollector::new());
        assert_eq!(app.handle_key(KeyCode::Char('q')), KeyAction::Quit);
    }

    #[test]
    fn left_click_selects_visible_row() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.set_viewport_rows(3);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.selected, 1);
    }

    #[test]
    fn click_outside_data_rows_does_not_change_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.set_viewport_rows(3);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.selected = 2;

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

        assert_eq!(app.selected, 2);
    }

    #[test]
    fn wheel_scroll_up_changes_offset_not_selected() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.selected = 3;
        app.scroll_offset = 2;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.scroll_offset, 1);
        assert_eq!(app.selected, 3);
    }

    #[test]
    fn wheel_scroll_down_changes_offset_not_selected() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.selected = 0;
        app.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.scroll_offset, 1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn wheel_scroll_clamps_at_bounds() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.scroll_offset, 0);

        app.scroll_offset = 2;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.scroll_offset, 2);
    }

    #[test]
    fn wheel_outside_table_does_not_change_state() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.selected = 2;
        app.scroll_offset = 1;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 41,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.scroll_offset, 1);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn wheel_on_table_border_is_handled() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(5, 2, 40, 8));
        app.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn set_viewport_rows_does_not_force_selected_visible() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
            sample_row(50),
        ];
        app.selected = 0;
        app.scroll_offset = 2;

        app.set_viewport_rows(2);

        assert_eq!(app.scroll_offset, 2);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn restore_selection_without_ensure_visible_keeps_scroll_offset() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
            sample_row(50),
        ];
        app.set_viewport_rows(2);
        app.selected = 0;
        app.scroll_offset = 3;

        app.restore_selection(Some(10), false);

        assert_eq!(app.selected, 0);
        assert_eq!(app.scroll_offset, 3);
    }

    #[test]
    fn tree_mode_starts_with_roots_only() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.snapshot.processes = vec![
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
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
            .collect();

        assert_eq!(app.view_mode, ViewMode::Tree);
        assert_eq!(visible, vec![1, 4]);
    }

    #[test]
    fn tree_right_expands_and_left_collapses() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 2)];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));

        app.handle_key(KeyCode::Right);
        let visible_after_expand: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible_after_expand, vec![1, 2]);

        app.handle_key(KeyCode::Left);
        let visible_after_collapse: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible_after_collapse, vec![1]);
    }

    #[test]
    fn tree_gutter_click_toggles_expansion() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
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
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
            .collect();
        assert_eq!(visible, vec![1, 2, 3]);
    }

    #[test]
    fn tree_name_branch_click_selects_without_toggling() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.rebuild_tree_rows();
        app.expanded_pids.insert(1);
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
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
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
        app.snapshot.processes = vec![parent, child_a, child_b];
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));
        app.handle_key(KeyCode::Right);

        let visible: Vec<i32> = app
            .visible_row_entries()
            .into_iter()
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
            .collect();

        assert_eq!(visible, vec![1, 3, 2]);
    }

    #[test]
    fn tree_mode_keeps_multiple_unresolved_roots_visible() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.snapshot.processes = vec![
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
            .map(|entry| app.snapshot.processes[entry.process_index].pid)
            .collect();

        assert_eq!(visible, vec![1, 2, 3, 4]);
    }

    #[test]
    fn tree_child_of_non_last_root_tracks_root_vertical_guide() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.expanded_pids.insert(1);
        app.rebuild_tree_rows();
        app.handle_key(KeyCode::Char('t'));

        let child = app
            .visible_row_entries()
            .into_iter()
            .find(|entry| app.snapshot.processes[entry.process_index].pid == 2)
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
}
