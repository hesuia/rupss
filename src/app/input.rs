use super::{
    AppState, KeyAction, ViewMode,
    navigation::{
        boundary_selection, clamp_scroll_offset, ensure_visible_scroll, is_inside_table,
        move_visible_index, restored_selection, table_hit_test,
    },
};
use crate::collector::SystemCollector;
use crate::snapshot::{HistoryPoint, SortKey};
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

impl AppState {
    /// Applies one keyboard action.
    ///
    /// Returns a [`KeyAction`] that tells the caller whether to continue
    /// running or terminate the application.
    /// The mapping is intentionally small and focused on fast navigation.
    pub fn handle_key(&mut self, code: KeyCode) -> KeyAction {
        match code {
            KeyCode::Char('q') => KeyAction::Quit,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection_and_continue(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection_and_continue(1),
            KeyCode::PageUp => {
                self.move_selection_and_continue(-(self.view.viewport_rows.max(1) as isize))
            }
            KeyCode::PageDown => {
                self.move_selection_and_continue(self.view.viewport_rows.max(1) as isize)
            }
            KeyCode::Home => self.jump_to_boundary_and_continue(true),
            KeyCode::End => self.jump_to_boundary_and_continue(false),
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
            KeyCode::Char('i') => self.resort_and_continue(SortKey::Pid),
            KeyCode::Char('p') => self.resort_and_continue(SortKey::Ppid),
            KeyCode::Char('o') => self.resort_and_continue(SortKey::Owner),
            KeyCode::Char('n') => self.resort_and_continue(SortKey::Name),
            KeyCode::Char('m') => self.resort_and_continue(SortKey::Command),
            KeyCode::Char('r') => self.resort_and_continue(SortKey::Rss),
            KeyCode::Char('s') => self.resort_and_continue(SortKey::Swap),
            KeyCode::Char('c') => self.resort_and_continue(SortKey::Cpu),
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

    pub(super) fn restore_selection(&mut self, selected_pid: Option<i32>, ensure_visible: bool) {
        if self.data.snapshot.processes.is_empty() {
            self.view.selected = 0;
            self.view.scroll_offset = 0;
            return;
        }

        self.view.selected = restored_selection(selected_pid, &self.data.snapshot.processes);

        if self.view.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }

        if ensure_visible {
            self.ensure_visible();
        } else {
            self.clamp_scroll_offset();
        }
    }

    pub(super) fn push_history(&mut self, point: HistoryPoint) {
        self.data.rss_history.push(point.rss_bytes);
        self.data.swap_history.push(point.swap_bytes);
    }

    pub(super) fn populate_visible_details(&mut self) {
        let pids = self.visible_pids();
        if pids.is_empty() {
            return;
        }

        let details = self
            .resources
            .collector
            .collect_visible_memory_details(&pids);
        for row in &mut self.data.snapshot.processes {
            if let Some(detail) = details.get(&row.pid) {
                row.uss_bytes = detail.uss_bytes;
                row.pss_bytes = detail.pss_bytes;
                row.detailed_swap_bytes = detail.swap_bytes;
            }
        }
    }

    pub(super) fn ensure_visible(&mut self) {
        let selected_visible = match self.view.view_mode {
            ViewMode::Flat => self.view.selected,
            ViewMode::Tree => self.selected_visible_index().unwrap_or(0),
        };

        self.view.scroll_offset = ensure_visible_scroll(
            selected_visible,
            self.view.scroll_offset,
            self.view.viewport_rows,
            self.total_visible_rows(),
        );
    }

    pub(super) fn clamp_scroll_offset(&mut self) {
        self.view.scroll_offset = clamp_scroll_offset(
            self.view.scroll_offset,
            self.total_visible_rows(),
            self.view.viewport_rows,
        );
    }

    fn resort_and_continue(&mut self, sort_key: SortKey) -> KeyAction {
        self.resort(sort_key);
        KeyAction::Continue
    }

    fn move_selection_and_continue(&mut self, delta: isize) -> KeyAction {
        self.move_selection(delta);
        KeyAction::Continue
    }

    fn jump_to_boundary_and_continue(&mut self, to_start: bool) -> KeyAction {
        self.view.selected = boundary_selection(
            self.view.view_mode,
            self.data.snapshot.processes.len(),
            &self.tree.rows,
            to_start,
        );
        self.ensure_visible();
        self.populate_visible_details();
        KeyAction::Continue
    }

    fn resort(&mut self, sort_key: SortKey) {
        self.view.sort_state.toggle_key(sort_key);
        let selected_pid = self.selected_pid();
        self.sort_current_processes();
        self.rebuild_tree_rows();
        self.restore_selection(selected_pid, true);
        self.populate_visible_details();
    }

    fn move_selection(&mut self, delta: isize) {
        if self.total_visible_rows() == 0 {
            return;
        }
        if self.view.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }

        let next = match move_visible_index(
            self.selected_visible_index(),
            delta,
            self.total_visible_rows(),
        ) {
            Some(next) => next,
            None => return,
        };
        self.view.selected = match self.view.view_mode {
            ViewMode::Flat => next,
            ViewMode::Tree => self.tree.rows[next].process_index,
        };
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn select_process_at(&mut self, column: u16, row: u16) {
        let Some(area) = self.view.process_table_area else {
            return;
        };
        let Some(data_row) = table_hit_test(area, column, row) else {
            return;
        };
        let visible_rows = self.visible_row_entries();
        if data_row >= visible_rows.len() {
            return;
        }

        let clicked = &visible_rows[data_row];
        if self.is_tree_toggle_click(column, clicked) {
            self.view.selected = clicked.process_index;
            self.toggle_expansion(clicked.process_index);
            return;
        }

        self.view.selected = clicked.process_index;
        self.ensure_visible();
        self.populate_visible_details();
    }

    fn scroll_with_wheel(&mut self, column: u16, row: u16, delta: isize) {
        let Some(area) = self.view.process_table_area else {
            return;
        };
        if !is_inside_table(area, column, row) || self.total_visible_rows() == 0 {
            return;
        }

        if delta < 0 {
            self.view.scroll_offset = self.view.scroll_offset.saturating_sub(delta.unsigned_abs());
        } else {
            self.view.scroll_offset = self.view.scroll_offset.saturating_add(delta as usize);
        }

        self.clamp_scroll_offset();
        self.populate_visible_details();
    }

    fn visible_pids(&self) -> Vec<i32> {
        self.visible_row_entries()
            .into_iter()
            .map(|entry| self.data.snapshot.processes[entry.process_index].pid)
            .collect()
    }

    fn toggle_view_mode(&mut self) {
        self.view.view_mode = self.view.view_mode.toggle();
        if self.view.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }
        self.ensure_visible();
        self.populate_visible_details();
    }
}
