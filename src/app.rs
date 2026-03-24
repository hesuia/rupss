use crate::collector::{CpuSample, ProcfsCollector, SystemCollector};
use crate::history::HistoryBuffer;
use crate::snapshot::{
    HistoryPoint, ProcessRow, Snapshot, SortDirection, SortKey, SortState, SystemSummary,
};
use crate::tui;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Stdout};
use std::time::{Duration, Instant};

const HISTORY_CAPACITY: usize = 180;
const TICK_RATE: Duration = Duration::from_secs(1);
const EVENT_POLL: Duration = Duration::from_millis(250);
type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

/// Result of handling one key input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    Quit,
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
    viewport_rows: usize,
    username_cache: HashMap<u32, String>,
    rss_history: HistoryBuffer<u64>,
    swap_history: HistoryBuffer<u64>,
    previous_cpu: HashMap<i32, CpuSample>,
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
                    if key.kind == KeyEventKind::Press {
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
            viewport_rows: 20,
            username_cache: load_username_cache(),
            rss_history: HistoryBuffer::new(HISTORY_CAPACITY),
            swap_history: HistoryBuffer::new(HISTORY_CAPACITY),
            previous_cpu: HashMap::new(),
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

    /// Returns the rows currently visible in the process table viewport.
    pub fn visible_processes(&self) -> &[ProcessRow] {
        let end = self
            .scroll_offset
            .saturating_add(self.viewport_rows)
            .min(self.snapshot.processes.len());
        &self.snapshot.processes[self.scroll_offset.min(end)..end]
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
                self.selected = 0;
                self.ensure_visible();
                self.populate_visible_details();
                KeyAction::Continue
            }
            KeyCode::End => {
                self.selected = self.snapshot.processes.len().saturating_sub(1);
                self.ensure_visible();
                self.populate_visible_details();
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
            self.sort_state.toggle();
        } else {
            self.sort_state = SortState::new(sort_key, sort_key.default_direction());
        }
        let selected_pid = self.selected_pid();
        sort_processes(
            self.sort_state,
            &self.username_cache,
            &mut self.snapshot.processes,
        );
        self.restore_selection(selected_pid, true);
        self.populate_visible_details();
    }

    fn move_selection(&mut self, delta: isize) {
        if self.snapshot.processes.is_empty() {
            return;
        }

        let max_index = self.snapshot.processes.len().saturating_sub(1) as isize;
        let next = (self.selected as isize + delta).clamp(0, max_index) as usize;
        self.selected = next;
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
        let visible_len = self.visible_processes().len();
        if data_row >= visible_len {
            return;
        }

        self.selected = self.scroll_offset.saturating_add(data_row);
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn scroll_with_wheel(&mut self, column: u16, row: u16, delta: isize) {
        if !self.is_inside_process_table(column, row) {
            return;
        }
        if self.snapshot.processes.is_empty() {
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
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }

        let view_end = self.scroll_offset.saturating_add(self.viewport_rows);
        if self.selected >= view_end {
            self.scroll_offset = self
                .selected
                .saturating_sub(self.viewport_rows.saturating_sub(1));
        }
        self.clamp_scroll_offset();
    }

    fn clamp_scroll_offset(&mut self) {
        let max_offset = self
            .snapshot
            .processes
            .len()
            .saturating_sub(self.viewport_rows.max(1));
        self.scroll_offset = self.scroll_offset.min(max_offset);
    }

    fn visible_pids(&self) -> Vec<i32> {
        self.visible_processes().iter().map(|row| row.pid).collect()
    }

    fn populate_visible_details(&mut self) {
        let pids = self.visible_pids();
        if pids.is_empty() {
            return;
        }

        let details = self.collector.collect_visible_memory_details(&pids);
        // Clear stale detail values first so hidden rows do not keep old `smaps_rollup` data.
        for row in &mut self.snapshot.processes {
            row.uss_bytes = None;
            row.pss_bytes = None;
            row.detailed_swap_bytes = None;
        }
        for row in &mut self.snapshot.processes {
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

fn owner_display_name(username_cache: &HashMap<u32, String>, uid: u32) -> String {
    username_cache
        .get(&uid)
        .map(|name| name.to_lowercase())
        .unwrap_or_else(|| uid.to_string())
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
    use super::{AppState, KeyAction, compare_process_rows, history_points};
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
}
