use super::{HISTORY_CAPACITY, TreeRow, ViewMode, owners::OwnerNameCache};
use crate::{
    collector::{CpuSample, ProcfsCollector},
    error::CollectorError,
    history::HistoryBuffer,
    snapshot::{Snapshot, SortDirection, SortKey, SortState, SystemSummary},
};
use ratatui::layout::Rect;
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

/// Mutable data captured from the system and derived from refresh cycles.
pub(super) struct AppDataState {
    pub(super) snapshot: Snapshot,
    pub(super) rss_history: HistoryBuffer<u64>,
    pub(super) swap_history: HistoryBuffer<u64>,
    pub(super) previous_cpu: HashMap<i32, CpuSample>,
    pub(super) last_error: Option<CollectorError>,
}

impl AppDataState {
    pub(super) fn new() -> Self {
        Self {
            snapshot: empty_snapshot(),
            rss_history: HistoryBuffer::new(HISTORY_CAPACITY),
            swap_history: HistoryBuffer::new(HISTORY_CAPACITY),
            previous_cpu: HashMap::new(),
            last_error: None,
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
