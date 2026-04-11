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
    snapshot::{ProcessRow, Snapshot, SortState, SystemSummary},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::{io::Stdout, ops::Range, time::Duration};
use strum::{AsRefStr, IntoStaticStr};
pub(crate) use tree::TreeRow;

use self::owners::{OwnerNameCache, OwnerNameResolver};
use self::state::{AppDataState, AppResources, AppViewState, ProcessTreeState};

const HISTORY_CAPACITY: usize = 180;
const TICK_RATE: Duration = Duration::from_secs(1);
const EVENT_POLL: Duration = Duration::from_millis(250);
pub(crate) const PROCESS_TABLE_COLUMN_SPACING: u16 = 1;
const PROCESS_TABLE_COLUMN_WIDTHS_FLAT: [u16; ProcessColumn::ALL.len()] =
    [7, 7, 12, 8, 24, 24, 12, 12, 12, 12, 8];
const PROCESS_TABLE_COLUMN_WIDTHS_TREE: [u16; ProcessColumn::ALL.len()] =
    [7, 7, 12, 8, 32, 16, 12, 12, 12, 12, 8];
type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessColumn {
    Pid,
    Ppid,
    Owner,
    Thread,
    Name,
    Command,
    Rss,
    Uss,
    Pss,
    Swap,
    Cpu,
}

impl ProcessColumn {
    pub(crate) const ALL: [Self; 11] = [
        Self::Pid,
        Self::Ppid,
        Self::Owner,
        Self::Thread,
        Self::Name,
        Self::Command,
        Self::Rss,
        Self::Uss,
        Self::Pss,
        Self::Swap,
        Self::Cpu,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Pid => "PID",
            Self::Ppid => "PPID",
            Self::Owner => "OWNER",
            Self::Thread => "THREAD",
            Self::Name => "NAME",
            Self::Command => "COMMAND",
            Self::Rss => "RSS",
            Self::Uss => "USS",
            Self::Pss => "PSS",
            Self::Swap => "SWAP",
            Self::Cpu => "CPU",
        }
    }

    pub(crate) fn width(self, view_mode: ViewMode) -> u16 {
        let widths = match view_mode {
            ViewMode::Flat => PROCESS_TABLE_COLUMN_WIDTHS_FLAT,
            ViewMode::Tree => PROCESS_TABLE_COLUMN_WIDTHS_TREE,
        };
        widths[self.index()]
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Pid => 0,
            Self::Ppid => 1,
            Self::Owner => 2,
            Self::Thread => 3,
            Self::Name => 4,
            Self::Command => 5,
            Self::Rss => 6,
            Self::Uss => 7,
            Self::Pss => 8,
            Self::Swap => 9,
            Self::Cpu => 10,
        }
    }

    pub(crate) fn from_sort_key(sort_key: crate::snapshot::SortKey) -> Option<Self> {
        match sort_key {
            crate::snapshot::SortKey::Pid => Some(Self::Pid),
            crate::snapshot::SortKey::Ppid => Some(Self::Ppid),
            crate::snapshot::SortKey::Owner => Some(Self::Owner),
            crate::snapshot::SortKey::Name => Some(Self::Name),
            crate::snapshot::SortKey::Command => Some(Self::Command),
            crate::snapshot::SortKey::Rss => Some(Self::Rss),
            crate::snapshot::SortKey::Swap => Some(Self::Swap),
            crate::snapshot::SortKey::Cpu => Some(Self::Cpu),
        }
    }

    pub(crate) fn is_toggleable(self) -> bool {
        self != Self::Name
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ColumnVisibility {
    order: [ProcessColumn; ProcessColumn::ALL.len()],
    visible: [bool; ProcessColumn::ALL.len()],
}

impl ColumnVisibility {
    pub(crate) fn new() -> Self {
        Self {
            order: ProcessColumn::ALL,
            visible: [true; ProcessColumn::ALL.len()],
        }
    }

    pub(crate) fn is_visible(&self, column: ProcessColumn) -> bool {
        self.visible[column.index()]
    }

    pub(crate) fn ordered_columns(&self) -> &[ProcessColumn; ProcessColumn::ALL.len()] {
        &self.order
    }

    pub(crate) fn visible_columns(&self) -> impl Iterator<Item = ProcessColumn> + '_ {
        self.order
            .into_iter()
            .filter(|column| self.is_visible(*column))
    }

    pub(crate) fn column_at(&self, index: usize) -> Option<ProcessColumn> {
        self.order.get(index).copied()
    }

    pub(crate) fn toggle(&mut self, column: ProcessColumn) -> bool {
        if !column.is_toggleable() || self.visible_count() == 1 && self.is_visible(column) {
            return false;
        }

        let index = column.index();
        self.visible[index] = !self.visible[index];
        true
    }

    pub(crate) fn visible_count(&self) -> usize {
        self.visible.iter().filter(|visible| **visible).count()
    }

    pub(crate) fn move_column(&mut self, index: usize, delta: isize) -> Option<usize> {
        let target = if delta < 0 {
            index.checked_sub(delta.unsigned_abs())?
        } else {
            index.checked_add(delta as usize)?
        };
        if target >= self.order.len() {
            return None;
        }

        self.order.swap(index, target);
        Some(target)
    }
}

pub(crate) fn process_table_columns(visibility: &ColumnVisibility) -> Vec<ProcessColumn> {
    visibility.visible_columns().collect()
}

pub(crate) fn process_table_detail_request(visibility: &ColumnVisibility) -> VisibleDetailRequest {
    VisibleDetailRequest {
        uss: visibility.is_visible(ProcessColumn::Uss),
        pss: visibility.is_visible(ProcessColumn::Pss),
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

    pub(crate) fn visible_columns(&self) -> Vec<ProcessColumn> {
        process_table_columns(&self.view.column_visibility)
    }

    pub(crate) fn is_column_picker_open(&self) -> bool {
        self.view.column_picker_open
    }

    pub(crate) fn column_picker_index(&self) -> usize {
        self.view.column_picker_index
    }

    pub(crate) fn column_picker_columns(&self) -> &[ProcessColumn; ProcessColumn::ALL.len()] {
        self.view.column_visibility.ordered_columns()
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
            ViewMode::Flat => self
                .data
                .snapshot
                .processes
                .get(visible_index)
                .map(|_| visible_index),
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

    pub(crate) fn detail_request(&self) -> VisibleDetailRequest {
        process_table_detail_request(&self.view.column_visibility)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(ProcfsCollector::new())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, ColumnVisibility, ProcessColumn, process_table_detail_request};
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

    #[test]
    fn column_visibility_keeps_name_visible() {
        let mut visibility = ColumnVisibility::new();

        assert!(!visibility.toggle(ProcessColumn::Name));
        assert!(visibility.is_visible(ProcessColumn::Name));
    }

    #[test]
    fn detail_request_only_includes_visible_heavy_columns() {
        let mut visibility = ColumnVisibility::new();
        assert!(visibility.toggle(ProcessColumn::Uss));

        let request = process_table_detail_request(&visibility);
        assert!(!request.uss);
        assert!(request.pss);
    }

    #[test]
    fn column_visibility_can_reorder_columns() {
        let mut visibility = ColumnVisibility::new();

        assert_eq!(visibility.move_column(0, 1), Some(1));
        assert_eq!(visibility.ordered_columns()[0], ProcessColumn::Ppid);
        assert_eq!(visibility.ordered_columns()[1], ProcessColumn::Pid);
    }
}
