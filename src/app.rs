mod columns;
mod filter;
mod input;
mod navigation;
mod owners;
mod refresh;
mod runtime;
mod sort;
mod state;
mod tree;
mod view;

pub use runtime::run;

use crate::{
    collector::{ProcfsCollector, VisibleDetailRequest},
    error::CollectorError,
    snapshot::{ProcessRow, Snapshot, SortKey, SortState, SystemSummary},
};
pub(crate) use columns::ProcessColumn;
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
pub(crate) use state::FilterRow;
use std::{io::Stdout, ops::Range, time::Duration};
use strum::{AsRefStr, IntoEnumIterator, IntoStaticStr};
pub(crate) use tree::TreeRow;

use self::owners::{OwnerNameCache, OwnerNameResolver};
use self::state::{
    AppDataState, AppResources, AppViewState, ProcessMonitorSample, ProcessMonitorState,
    ProcessTreeState,
};

const HISTORY_CAPACITY: usize = 180;
const TICK_RATE: Duration = Duration::from_secs(1);
const EVENT_POLL: Duration = Duration::from_millis(250);
pub(crate) const PROCESS_TABLE_COLUMN_SPACING: u16 = 1;
type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

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
            resources: AppResources::new(collector, OwnerNameCache::new()),
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

    pub fn is_paused(&self) -> bool {
        self.view.paused
    }

    pub(crate) fn toggle_pause(&mut self) {
        self.view.paused = !self.view.paused;
    }

    pub(crate) fn visible_columns(&self) -> Vec<ProcessColumn> {
        self.view.columns.visible_columns()
    }

    pub(crate) fn is_column_picker_open(&self) -> bool {
        self.view.column_picker_open
    }

    pub(crate) fn is_sort_picker_open(&self) -> bool {
        self.view.sort_picker_open
    }

    pub(crate) fn is_filter_modal_open(&self) -> bool {
        self.view.filter_modal.open
    }

    pub(crate) fn is_process_monitor_open(&self) -> bool {
        self.view.process_monitor_open
    }

    pub(crate) fn process_monitor(&self) -> Option<&ProcessMonitorState> {
        self.data.process_monitor.as_ref()
    }

    pub(crate) fn filter_modal(&self) -> &self::state::FilterModalState {
        &self.view.filter_modal
    }

    pub(crate) fn is_filter_active(&self) -> bool {
        self.view.filter.is_active()
    }

    pub(crate) fn filtered_process_count(&self) -> usize {
        self.flat_process_count()
    }

    pub(crate) fn column_picker_index(&self) -> usize {
        self.view.column_picker_index
    }

    pub(crate) fn column_picker_columns(&self) -> &[ProcessColumn] {
        self.view.columns.ordered_columns()
    }

    pub(crate) fn sort_picker_index(&self) -> usize {
        self.view.sort_picker_index
    }

    pub(crate) fn sort_picker_keys(&self) -> Vec<SortKey> {
        SortKey::iter().collect()
    }

    pub fn system_summary(&self) -> &SystemSummary {
        &self.data.snapshot.system
    }

    pub fn process_row(&self, index: usize) -> Option<&ProcessRow> {
        self.data.snapshot.processes.get(index)
    }

    pub(crate) fn visible_row_range(&self) -> Range<usize> {
        view::visible_row_range(
            self.view.scroll_offset,
            self.view.viewport_rows,
            self.total_visible_rows(),
        )
    }

    pub(crate) fn process_index_at_visible_row(&self, visible_index: usize) -> Option<usize> {
        match self.view.view_mode {
            ViewMode::Flat => self.flat_process_index_at(visible_index),
            ViewMode::Tree => self
                .tree
                .rows
                .get(visible_index)
                .map(|row| row.process_index),
        }
    }

    pub(crate) fn tree_row_at_visible_row(&self, visible_index: usize) -> Option<&TreeRow> {
        match self.view.view_mode {
            ViewMode::Flat => None,
            ViewMode::Tree => self.tree.rows.get(visible_index),
        }
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
        self.resources.owner_resolver.owner_name(uid)
    }

    /// Returns the PID of the currently selected row, if any.
    pub fn selected_pid(&self) -> Option<i32> {
        self.selected_visible_index()?;
        self.data
            .snapshot
            .processes
            .get(self.view.selected)
            .map(|row| row.pid)
    }

    /// Builds chart points for the RSS history graph.
    pub fn rss_chart_points(&self) -> Vec<(f64, f64)> {
        view::history_points(&self.data.rss_history)
    }

    /// Builds chart points for the swap history graph.
    pub fn swap_chart_points(&self) -> Vec<(f64, f64)> {
        view::history_points(&self.data.swap_history)
    }

    pub(crate) fn process_monitor_rss_points(&self) -> Vec<(f64, f64)> {
        self.process_monitor_points(|sample| Some(sample.rss_bytes as f64))
    }

    pub(crate) fn process_monitor_uss_points(&self) -> Vec<(f64, f64)> {
        self.process_monitor_points(|sample| sample.uss_bytes.map(|value| value as f64))
    }

    pub(crate) fn process_monitor_pss_points(&self) -> Vec<(f64, f64)> {
        self.process_monitor_points(|sample| sample.pss_bytes.map(|value| value as f64))
    }

    pub(crate) fn process_monitor_swap_points(&self) -> Vec<(f64, f64)> {
        self.process_monitor_points(|sample| Some(sample.swap_bytes as f64))
    }

    pub(crate) fn process_monitor_cpu_points(&self) -> Vec<(f64, f64)> {
        self.process_monitor_points(|sample| Some(sample.cpu_percent as f64))
    }

    pub(crate) fn process_monitor_threads_points(&self) -> Vec<(f64, f64)> {
        self.process_monitor_points(|sample| Some(sample.threads as f64))
    }

    fn process_monitor_points(
        &self,
        value: impl Fn(&ProcessMonitorSample) -> Option<f64>,
    ) -> Vec<(f64, f64)> {
        self.data
            .process_monitor
            .as_ref()
            .map(|monitor| {
                monitor
                    .history
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sample)| value(sample).map(|value| (index as f64, value)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn detail_request(&self) -> VisibleDetailRequest {
        self.view.columns.detail_request()
    }

    pub(super) fn flat_process_count(&self) -> usize {
        if self.uses_cached_filtered_indexes() {
            self.view.filtered_indexes.len()
        } else {
            self.data.snapshot.processes.len()
        }
    }

    pub(super) fn flat_process_index_at(&self, visible_index: usize) -> Option<usize> {
        if self.uses_cached_filtered_indexes() {
            return self.view.filtered_indexes.get(visible_index).copied();
        }

        self.data
            .snapshot
            .processes
            .get(visible_index)
            .map(|_| visible_index)
    }

    pub(super) fn flat_visible_position(&self, process_index: usize) -> Option<usize> {
        if self.uses_cached_filtered_indexes() {
            return self
                .view
                .filtered_indexes
                .iter()
                .position(|index| *index == process_index);
        }

        (process_index < self.data.snapshot.processes.len()).then_some(process_index)
    }

    pub(super) fn current_flat_process_indexes(&self) -> Vec<usize> {
        if self.uses_cached_filtered_indexes() {
            return self.view.filtered_indexes.clone();
        }

        (0..self.data.snapshot.processes.len()).collect()
    }

    fn uses_cached_filtered_indexes(&self) -> bool {
        if self.view.filter.is_active() || !self.view.filtered_indexes.is_empty() {
            return true;
        }

        false
    }

    #[cfg(test)]
    pub(crate) fn replace_processes_for_test(&mut self, processes: Vec<ProcessRow>) {
        self.data.snapshot.processes = processes;
        self.rebuild_filtered_indexes();
        self.rebuild_tree_rows();
        self.restore_selection(None, false);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(ProcfsCollector::new())
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::collector::ProcfsCollector;
    use crate::error::CollectorError;
    use procfs::ProcError;
    use std::io;

    #[test]
    fn owner_name_falls_back_to_numeric_uid() {
        let app = AppState::new(ProcfsCollector::new());

        assert_eq!(app.owner_name(u32::MAX), u32::MAX.to_string());
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
