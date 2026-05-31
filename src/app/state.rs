use super::{
    HISTORY_CAPACITY, TreeRow, ViewMode, columns::ColumnLayout, filter::MetricFilterOperator,
    filter::ProcessFilter, owners::OwnerNameCache,
};
use crate::{
    collector::{CpuSample, ProcfsCollector},
    error::CollectorError,
    history::HistoryBuffer,
    snapshot::{ProcessRow, Snapshot, SortDirection, SortKey, SortState, SystemSummary},
};
use ratatui::layout::Rect;
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};
use strum::{EnumCount, EnumIter, IntoEnumIterator};

/// Mutable data captured from the system and derived from refresh cycles.
pub(super) struct AppDataState {
    pub(super) snapshot: Snapshot,
    pub(super) rss_history: HistoryBuffer<u64>,
    pub(super) swap_history: HistoryBuffer<u64>,
    pub(super) process_monitor: Option<ProcessMonitorState>,
    pub(super) previous_cpu: HashMap<i32, CpuSample>,
    pub(super) last_error: Option<CollectorError>,
}

impl AppDataState {
    pub(super) fn new() -> Self {
        Self {
            snapshot: empty_snapshot(),
            rss_history: HistoryBuffer::new(HISTORY_CAPACITY),
            swap_history: HistoryBuffer::new(HISTORY_CAPACITY),
            process_monitor: None,
            previous_cpu: HashMap::new(),
            last_error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessMonitorSample {
    pub(crate) rss_bytes: u64,
    pub(crate) uss_bytes: Option<u64>,
    pub(crate) pss_bytes: Option<u64>,
    pub(crate) swap_bytes: u64,
    pub(crate) cpu_percent: f32,
    pub(crate) threads: u64,
}

impl From<&ProcessRow> for ProcessMonitorSample {
    fn from(row: &ProcessRow) -> Self {
        Self {
            rss_bytes: row.rss_bytes,
            uss_bytes: row.uss_bytes,
            pss_bytes: row.pss_bytes,
            swap_bytes: row.swap_bytes,
            cpu_percent: row.cpu_percent,
            threads: row.threads,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessMonitorState {
    pub(crate) pid: i32,
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) latest: ProcessMonitorSample,
    pub(crate) history: HistoryBuffer<ProcessMonitorSample>,
    pub(crate) missing: bool,
}

impl ProcessMonitorState {
    pub(crate) fn new(row: &ProcessRow, capacity: usize) -> Self {
        let latest = ProcessMonitorSample::from(row);
        let mut history = HistoryBuffer::new(capacity);
        history.push(latest.clone());

        Self {
            pid: row.pid,
            name: row.name.clone(),
            command: row.command.clone(),
            latest,
            history,
            missing: false,
        }
    }

    pub(crate) fn record(&mut self, row: &ProcessRow) {
        let sample = ProcessMonitorSample::from(row);
        self.name.clone_from(&row.name);
        self.command.clone_from(&row.command);
        self.latest = sample.clone();
        self.history.push(sample);
        self.missing = false;
    }

    pub(crate) fn mark_missing(&mut self) {
        self.missing = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount, EnumIter)]
pub(crate) enum FilterRow {
    Pid,
    Ppid,
    Name,
    Command,
    Rss,
    Swap,
    Cpu,
    Uss,
    Pss,
}

impl FilterRow {
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        Self::iter().nth(index)
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Pid => "PID",
            Self::Ppid => "PPID",
            Self::Name => "NAME",
            Self::Command => "COMMAND",
            Self::Rss => "RSS",
            Self::Swap => "SWAP",
            Self::Cpu => "CPU",
            Self::Uss => "USS",
            Self::Pss => "PSS",
        }
    }

    pub(crate) fn is_metric(self) -> bool {
        matches!(
            self,
            Self::Rss | Self::Swap | Self::Cpu | Self::Uss | Self::Pss
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FilterModalState {
    pub(crate) open: bool,
    pub(crate) selected: usize,
    pub(crate) editing: bool,
    pub(crate) error: Option<String>,

    pub(crate) pid: String,
    pub(crate) ppid: String,
    pub(crate) name: String,
    pub(crate) command: String,

    pub(crate) rss_op: MetricFilterOperator,
    pub(crate) rss_value: String,
    pub(crate) swap_op: MetricFilterOperator,
    pub(crate) swap_value: String,
    pub(crate) cpu_op: MetricFilterOperator,
    pub(crate) cpu_value: String,
    pub(crate) uss_op: MetricFilterOperator,
    pub(crate) uss_value: String,
    pub(crate) pss_op: MetricFilterOperator,
    pub(crate) pss_value: String,
}

impl Default for FilterModalState {
    fn default() -> Self {
        Self {
            open: false,
            selected: 0,
            editing: false,
            error: None,
            pid: String::new(),
            ppid: String::new(),
            name: String::new(),
            command: String::new(),
            rss_op: MetricFilterOperator::GreaterThanOrEqual,
            rss_value: String::new(),
            swap_op: MetricFilterOperator::GreaterThanOrEqual,
            swap_value: String::new(),
            cpu_op: MetricFilterOperator::GreaterThanOrEqual,
            cpu_value: String::new(),
            uss_op: MetricFilterOperator::GreaterThanOrEqual,
            uss_value: String::new(),
            pss_op: MetricFilterOperator::GreaterThanOrEqual,
            pss_value: String::new(),
        }
    }
}

/// Mutable state used to navigate and render the process table.
pub(super) struct AppViewState {
    pub(super) sort_state: SortState,
    pub(super) selected: usize,
    pub(super) scroll_offset: usize,
    pub(super) view_mode: ViewMode,
    pub(super) viewport_rows: usize,
    pub(super) process_table_area: Option<Rect>,
    pub(super) columns: ColumnLayout,
    pub(super) column_picker_open: bool,
    pub(super) column_picker_index: usize,
    pub(super) sort_picker_open: bool,
    pub(super) sort_picker_index: usize,
    pub(super) process_monitor_open: bool,
    pub(super) paused: bool,
    pub(super) filter: ProcessFilter,
    pub(super) filter_modal: FilterModalState,
    pub(super) filtered_indexes: Vec<usize>,
}

impl AppViewState {
    pub(super) fn new() -> Self {
        Self {
            sort_state: SortState::new(SortKey::Rss, SortDirection::Descending),
            selected: 0,
            scroll_offset: 0,
            view_mode: ViewMode::Flat,
            viewport_rows: 20,
            process_table_area: None,
            columns: ColumnLayout::default(),
            column_picker_open: false,
            column_picker_index: 0,
            sort_picker_open: false,
            sort_picker_index: 0,
            process_monitor_open: false,
            paused: false,
            filter: ProcessFilter::default(),
            filter_modal: FilterModalState::default(),
            filtered_indexes: Vec::new(),
        }
    }
}

/// Derived tree view state built from the current snapshot.
pub(super) struct ProcessTreeState {
    pub(super) expanded_pids: HashSet<i32>,
    pub(super) rows: Vec<TreeRow>,
    pub(super) parents: Vec<Option<usize>>,
}

impl ProcessTreeState {
    pub(super) fn new() -> Self {
        Self {
            expanded_pids: HashSet::new(),
            rows: Vec::new(),
            parents: Vec::new(),
        }
    }
}

/// Shared resources and caches used by state transitions.
pub(super) struct AppResources {
    pub(super) owner_resolver: OwnerNameCache,
    pub(super) collector: ProcfsCollector,
}

impl AppResources {
    pub(super) fn new(collector: ProcfsCollector, owner_resolver: OwnerNameCache) -> Self {
        Self {
            owner_resolver,
            collector,
        }
    }
}

pub(super) fn empty_snapshot() -> Snapshot {
    Snapshot {
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
    }
}
