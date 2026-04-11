use super::AppState;
use crate::{collector::SystemCollector, snapshot::HistoryPoint};
use std::time::Instant;

impl AppState {
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
        for row in &mut self.data.snapshot.processes {
            if let Some(detail) = details.get(&row.pid) {
                row.uss_bytes = detail.uss_bytes;
                row.pss_bytes = detail.pss_bytes;
            }
        }
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
        assert!(app.view.column_visibility.toggle(ProcessColumn::Uss));
        assert!(app.view.column_visibility.toggle(ProcessColumn::Pss));

        let request = app.detail_request();
        assert!(!request.needs_any());
    }
}
