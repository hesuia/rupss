use super::AppState;
use crate::{
    collector::{DetailedMemorySample, SystemCollector, VisibleDetailRequest},
    snapshot::{HistoryPoint, ProcessRow},
};
use std::collections::HashMap;
use std::time::Instant;

impl AppState {
    /// Refreshes the full snapshot and then loads any detailed memory data needed by the UI.
    ///
    /// The refresh flow:
    /// 1. Collect a cheap full-process snapshot and aggregate totals.
    /// 2. Sort the list and keep the previously selected PID if still present.
    /// 3. Update the history buffers for the top graphs.
    /// 4. Load `smaps_rollup` details for filter predicates and visible rows when needed.
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
                self.populate_filter_details();
                self.record_process_monitor_sample();
                self.rebuild_filtered_indexes();
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

    pub(super) fn push_history(&mut self, point: HistoryPoint) {
        self.data.rss_history.push(point.rss_bytes);
        self.data.swap_history.push(point.swap_bytes);
    }

    pub(super) fn populate_visible_details(&mut self) {
        let pids = self.visible_pids();
        let request = self.detail_request();
        if pids.is_empty() || !request.needs_any() {
            return;
        }

        let details = self
            .resources
            .collector
            .collect_visible_memory_details(&pids, request);
        self.apply_memory_details(details);
    }

    pub(super) fn populate_filter_details(&mut self) {
        let request = self.view.filter.detail_request();
        if !request.needs_any() {
            return;
        }

        let pids = self
            .data
            .snapshot
            .processes
            .iter()
            .map(|row| row.pid)
            .collect::<Vec<_>>();
        self.populate_memory_details(&pids, request);
    }

    fn populate_memory_details(&mut self, pids: &[i32], request: VisibleDetailRequest) {
        if pids.is_empty() || !request.needs_any() {
            return;
        }

        let details = self
            .resources
            .collector
            .collect_visible_memory_details(pids, request);
        self.apply_memory_details(details);
    }

    fn apply_memory_details(&mut self, details: HashMap<i32, DetailedMemorySample>) {
        for row in &mut self.data.snapshot.processes {
            if let Some(detail) = details.get(&row.pid) {
                row.uss_bytes = detail.uss_bytes;
                row.pss_bytes = detail.pss_bytes;
            }
        }
    }

    pub(super) fn populate_process_monitor_details(&mut self, pid: i32) {
        self.populate_memory_details(
            &[pid],
            VisibleDetailRequest {
                uss: true,
                pss: true,
            },
        );
    }

    pub(super) fn start_process_monitor_for_selected(&mut self) {
        let Some(selected) = self.selected_process_row().cloned() else {
            return;
        };

        self.view.column_picker_open = false;
        self.view.sort_picker_open = false;
        self.view.filter_modal.open = false;
        self.view.filter_modal.editing = false;

        self.populate_process_monitor_details(selected.pid);
        let row = self
            .data
            .snapshot
            .processes
            .iter()
            .find(|row| row.pid == selected.pid)
            .cloned()
            .unwrap_or(selected);

        self.data.process_monitor = Some(super::state::ProcessMonitorState::new(
            &row,
            super::HISTORY_CAPACITY,
        ));
        self.view.process_monitor_open = true;
    }

    pub(super) fn close_process_monitor_overlay(&mut self) {
        self.view.process_monitor_open = false;
    }

    fn record_process_monitor_sample(&mut self) {
        let Some(pid) = self
            .data
            .process_monitor
            .as_ref()
            .map(|monitor| monitor.pid)
        else {
            return;
        };

        let Some(row) = self
            .data
            .snapshot
            .processes
            .iter()
            .find(|row| row.pid == pid)
            .cloned()
        else {
            if let Some(monitor) = self.data.process_monitor.as_mut() {
                monitor.mark_missing();
            }
            return;
        };

        self.populate_process_monitor_details(pid);
        let row = self
            .data
            .snapshot
            .processes
            .iter()
            .find(|row| row.pid == pid)
            .cloned()
            .unwrap_or(row);

        self.record_process_monitor_row(&row);
    }

    pub(super) fn record_process_monitor_row(&mut self, row: &ProcessRow) {
        match self.data.process_monitor.as_mut() {
            Some(monitor) if monitor.pid == row.pid => monitor.record(row),
            _ => {
                self.data.process_monitor = Some(super::state::ProcessMonitorState::new(
                    row,
                    super::HISTORY_CAPACITY,
                ));
            }
        }
    }

    fn selected_process_row(&self) -> Option<&ProcessRow> {
        self.selected_visible_index()?;
        self.data.snapshot.processes.get(self.view.selected)
    }

    fn visible_pids(&self) -> Vec<i32> {
        self.visible_row_range()
            .filter_map(|visible_index| self.process_index_at_visible_row(visible_index))
            .filter_map(|process_index| self.data.snapshot.processes.get(process_index))
            .map(|row| row.pid)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::app::ProcessColumn;
    use crate::collector::ProcfsCollector;
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};

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
            swap_bytes: pid as u64,
            cpu_percent: pid as f32,
        }
    }

    fn tree_row(pid: i32, ppid: i32) -> ProcessRow {
        let mut row = sample_row(pid);
        row.ppid = ppid;
        row
    }

    #[test]
    fn visible_pids_uses_flat_viewport_slice() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.view.scroll_offset = 1;
        app.view.viewport_rows = 2;

        assert_eq!(app.visible_pids(), vec![20, 30]);
    }

    #[test]
    fn visible_pids_uses_tree_visible_rows() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.tree.expanded_pids.insert(1);
        app.rebuild_tree_rows();
        app.view.view_mode = super::super::ViewMode::Tree;
        app.view.viewport_rows = 3;

        assert_eq!(app.visible_pids(), vec![1, 2, 3]);
    }

    #[test]
    fn detail_request_is_empty_when_heavy_columns_hidden() {
        let mut app = AppState::new(ProcfsCollector::new());
        assert!(app.view.columns.toggle(ProcessColumn::Uss));
        assert!(app.view.columns.toggle(ProcessColumn::Pss));

        let request = app.detail_request();
        assert!(!request.needs_any());
    }

    #[test]
    fn process_monitor_replaces_prior_target() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.replace_processes_for_test(vec![sample_row(10), sample_row(20)]);
        app.view.selected = 0;
        app.start_process_monitor_for_selected();

        app.view.selected = 1;
        app.start_process_monitor_for_selected();

        let monitor = app.process_monitor().unwrap();
        assert_eq!(monitor.pid, 20);
        assert_eq!(monitor.history.len(), 1);
    }

    #[test]
    fn process_monitor_records_heavy_values_when_columns_hidden() {
        let mut app = AppState::new(ProcfsCollector::new());
        let mut row = sample_row(10);
        row.uss_bytes = Some(123);
        row.pss_bytes = Some(456);
        assert!(app.view.columns.toggle(ProcessColumn::Uss));
        assert!(app.view.columns.toggle(ProcessColumn::Pss));

        app.record_process_monitor_row(&row);

        let monitor = app.process_monitor().unwrap();
        assert_eq!(monitor.latest.uss_bytes, Some(123));
        assert_eq!(monitor.latest.pss_bytes, Some(456));
        assert_eq!(app.process_monitor_uss_points(), vec![(0.0, 123.0)]);
        assert_eq!(app.process_monitor_pss_points(), vec![(0.0, 456.0)]);
    }

    #[test]
    fn process_monitor_marks_missing_without_adding_zero_sample() {
        let mut app = AppState::new(ProcfsCollector::new());
        let row = sample_row(10);
        app.record_process_monitor_row(&row);
        app.data.snapshot.processes.clear();

        app.record_process_monitor_sample();

        let monitor = app.process_monitor().unwrap();
        assert!(monitor.missing);
        assert_eq!(monitor.history.len(), 1);
        assert_eq!(monitor.latest.rss_bytes, 10);
    }
}
