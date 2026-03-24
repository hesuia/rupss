use crate::collector::{CpuSample, ProcfsCollector, SystemCollector};
use crate::history::HistoryBuffer;
use crate::snapshot::{HistoryPoint, ProcessRow, Snapshot, SortKey, SystemSummary};
use crate::tui;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Stdout};
use std::time::{Duration, Instant};

const HISTORY_CAPACITY: usize = 180;
const TICK_RATE: Duration = Duration::from_secs(1);
const EVENT_POLL: Duration = Duration::from_millis(250);

pub struct AppState {
    pub snapshot: Snapshot,
    pub sort_key: SortKey,
    pub selected: usize,
    pub scroll_offset: usize,
    viewport_rows: usize,
    username_cache: HashMap<u32, String>,
    rss_history: HistoryBuffer<u64>,
    swap_history: HistoryBuffer<u64>,
    previous_cpu: HashMap<i32, CpuSample>,
    collector: ProcfsCollector,
    pub last_error: Option<String>,
}

pub fn run() -> io::Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    restore_terminal(&mut terminal)?;
    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    let collector = ProcfsCollector::new();
    let mut app = AppState::new(collector);
    app.refresh()?;

    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|frame| tui::render(frame, &mut app))?;

        if event::poll(EVENT_POLL)? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if app.handle_key(key.code) {
                return Ok(());
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            app.refresh()?;
            last_tick = Instant::now();
        }
    }
}

impl AppState {
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
            sort_key: SortKey::Rss,
            selected: 0,
            scroll_offset: 0,
            viewport_rows: 20,
            username_cache: load_username_cache(),
            rss_history: HistoryBuffer::new(HISTORY_CAPACITY),
            swap_history: HistoryBuffer::new(HISTORY_CAPACITY),
            previous_cpu: HashMap::new(),
            collector,
            last_error: None,
        }
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        let selected_pid = self.selected_pid();
        let now = Instant::now();
        match self
            .collector
            .collect_base_snapshot(&self.previous_cpu, now)
        {
            Ok((mut snapshot, next_cpu, history_point)) => {
                self.previous_cpu = next_cpu;
                sort_processes(self.sort_key, &mut snapshot.processes);
                self.snapshot = snapshot;
                self.restore_selection(selected_pid);
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

    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
        self.ensure_visible();
    }

    pub fn visible_processes(&self) -> &[ProcessRow] {
        let end = self
            .scroll_offset
            .saturating_add(self.viewport_rows)
            .min(self.snapshot.processes.len());
        &self.snapshot.processes[self.scroll_offset.min(end)..end]
    }

    pub fn owner_name(&self, uid: u32) -> String {
        self.username_cache
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string())
    }

    pub fn selected_pid(&self) -> Option<i32> {
        self.snapshot
            .processes
            .get(self.selected)
            .map(|row| row.pid)
    }

    pub fn rss_chart_points(&self) -> Vec<(f64, f64)> {
        history_points(&self.rss_history)
    }

    pub fn swap_chart_points(&self) -> Vec<(f64, f64)> {
        history_points(&self.swap_history)
    }

    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('q') => true,
            KeyCode::Up => {
                self.move_selection(-1);
                false
            }
            KeyCode::Down => {
                self.move_selection(1);
                false
            }
            KeyCode::PageUp => {
                let page = self.viewport_rows.max(1) as isize;
                self.move_selection(-page);
                false
            }
            KeyCode::PageDown => {
                let page = self.viewport_rows.max(1) as isize;
                self.move_selection(page);
                false
            }
            KeyCode::Home => {
                self.selected = 0;
                self.ensure_visible();
                self.populate_visible_details();
                false
            }
            KeyCode::End => {
                self.selected = self.snapshot.processes.len().saturating_sub(1);
                self.ensure_visible();
                self.populate_visible_details();
                false
            }
            KeyCode::Char('r') => {
                self.sort_key = SortKey::Rss;
                self.resort();
                false
            }
            KeyCode::Char('s') => {
                self.sort_key = SortKey::Swap;
                self.resort();
                false
            }
            KeyCode::Char('p') => {
                self.sort_key = SortKey::Pss;
                self.resort();
                false
            }
            KeyCode::Char('c') => {
                self.sort_key = SortKey::Cpu;
                self.resort();
                false
            }
            _ => false,
        }
    }

    fn resort(&mut self) {
        let selected_pid = self.selected_pid();
        sort_processes(self.sort_key, &mut self.snapshot.processes);
        self.restore_selection(selected_pid);
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

    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }

        let view_end = self.scroll_offset.saturating_add(self.viewport_rows);
        if self.selected >= view_end {
            self.scroll_offset = self
                .selected
                .saturating_sub(self.viewport_rows.saturating_sub(1));
        }
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

    fn restore_selection(&mut self, selected_pid: Option<i32>) {
        if self.snapshot.processes.is_empty() {
            self.selected = 0;
            self.scroll_offset = 0;
            return;
        }

        self.selected = selected_pid
            .and_then(|pid| {
                self.snapshot
                    .processes
                    .iter()
                    .position(|row| row.pid == pid)
            })
            .unwrap_or(0)
            .min(self.snapshot.processes.len().saturating_sub(1));
        self.ensure_visible();
    }

    fn push_history(&mut self, point: HistoryPoint) {
        self.rss_history.push(point.rss_bytes);
        self.swap_history.push(point.swap_bytes);
    }
}

fn compare_process_rows(sort_key: SortKey, left: &ProcessRow, right: &ProcessRow) -> Ordering {
    let primary = match sort_key {
        SortKey::Rss => right.rss_bytes.cmp(&left.rss_bytes),
        SortKey::Swap => right.visible_swap_bytes().cmp(&left.visible_swap_bytes()),
        SortKey::Pss => right
            .pss_bytes
            .unwrap_or(0)
            .cmp(&left.pss_bytes.unwrap_or(0)),
        SortKey::Cpu => right
            .cpu_percent
            .partial_cmp(&left.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    primary.then_with(|| left.pid.cmp(&right.pid))
}

fn sort_processes(sort_key: SortKey, processes: &mut [ProcessRow]) {
    processes.sort_by(|left, right| compare_process_rows(sort_key, left, right));
}

fn history_points(history: &HistoryBuffer<u64>) -> Vec<(f64, f64)> {
    history
        .iter()
        .enumerate()
        .map(|(idx, value)| (idx as f64, *value as f64))
        .collect()
}

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

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{compare_process_rows, history_points};
    use crate::history::HistoryBuffer;
    use crate::snapshot::{ProcessRow, SortKey};

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
        let ordering = compare_process_rows(SortKey::Cpu, &sample_row(10), &sample_row(20));
        assert_eq!(ordering, std::cmp::Ordering::Greater);
    }
}
