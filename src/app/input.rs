use super::{
    AppState, KeyAction, ViewMode,
    navigation::{
        boundary_selection, clamp_scroll_offset, ensure_visible_scroll, is_inside_table,
        move_visible_index, restored_selection, table_hit_test,
    },
};
use crate::snapshot::SortKey;
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppCommand {
    Quit,
    MoveSelection(isize),
    JumpToBoundary { to_start: bool },
    TreeLeft,
    TreeRight,
    ToggleViewMode,
    Resort(SortKey),
    Noop,
}

impl AppState {
    /// Applies one keyboard action.
    ///
    /// Returns a [`KeyAction`] that tells the caller whether to continue
    /// running or terminate the application.
    /// The mapping is intentionally small and focused on fast navigation.
    pub fn handle_key(&mut self, code: KeyCode) -> KeyAction {
        self.apply_command(key_command(code, self.view.viewport_rows))
    }

    fn apply_command(&mut self, command: AppCommand) -> KeyAction {
        match command {
            AppCommand::Quit => KeyAction::Quit,
            AppCommand::MoveSelection(delta) => self.move_selection_and_continue(delta),
            AppCommand::JumpToBoundary { to_start } => self.jump_to_boundary_and_continue(to_start),
            AppCommand::TreeLeft => {
                self.handle_tree_left();
                KeyAction::Continue
            }
            AppCommand::TreeRight => {
                self.handle_tree_right();
                KeyAction::Continue
            }
            AppCommand::ToggleViewMode => {
                self.toggle_view_mode();
                KeyAction::Continue
            }
            AppCommand::Resort(sort_key) => self.resort_and_continue(sort_key),
            AppCommand::Noop => KeyAction::Continue,
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
        let visible_index = self.view.scroll_offset.saturating_add(data_row);
        let Some(process_index) = self.process_index_at_visible_row(visible_index) else {
            return;
        };

        if self
            .tree_row_at_visible_row(visible_index)
            .is_some_and(|row| self.is_tree_toggle_click(column, row))
        {
            self.view.selected = process_index;
            self.toggle_expansion(process_index);
            return;
        }

        self.view.selected = process_index;
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

    fn toggle_view_mode(&mut self) {
        self.view.view_mode = self.view.view_mode.toggle();
        if self.view.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
        }
        self.ensure_visible();
        self.populate_visible_details();
    }
}

fn key_command(code: KeyCode, viewport_rows: usize) -> AppCommand {
    let page_step = viewport_rows.max(1) as isize;

    match code {
        KeyCode::Char('q') => AppCommand::Quit,
        KeyCode::Up | KeyCode::Char('k') => AppCommand::MoveSelection(-1),
        KeyCode::Down | KeyCode::Char('j') => AppCommand::MoveSelection(1),
        KeyCode::PageUp => AppCommand::MoveSelection(-page_step),
        KeyCode::PageDown => AppCommand::MoveSelection(page_step),
        KeyCode::Home => AppCommand::JumpToBoundary { to_start: true },
        KeyCode::End => AppCommand::JumpToBoundary { to_start: false },
        KeyCode::Left => AppCommand::TreeLeft,
        KeyCode::Right => AppCommand::TreeRight,
        KeyCode::Char('t') => AppCommand::ToggleViewMode,
        KeyCode::Char('i') => AppCommand::Resort(SortKey::Pid),
        KeyCode::Char('p') => AppCommand::Resort(SortKey::Ppid),
        KeyCode::Char('o') => AppCommand::Resort(SortKey::Owner),
        KeyCode::Char('n') => AppCommand::Resort(SortKey::Name),
        KeyCode::Char('m') => AppCommand::Resort(SortKey::Command),
        KeyCode::Char('r') => AppCommand::Resort(SortKey::Rss),
        KeyCode::Char('s') => AppCommand::Resort(SortKey::Swap),
        KeyCode::Char('c') => AppCommand::Resort(SortKey::Cpu),
        _ => AppCommand::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::{AppCommand, AppState, KeyAction, key_command};
    use crate::collector::ProcfsCollector;
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

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
    fn key_command_maps_page_navigation() {
        assert_eq!(
            key_command(KeyCode::PageUp, 3),
            AppCommand::MoveSelection(-3)
        );
        assert_eq!(
            key_command(KeyCode::PageDown, 3),
            AppCommand::MoveSelection(3)
        );
    }

    #[test]
    fn sort_direction_toggles_on_same_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.view.sort_state.direction, SortDirection::Ascending);

        app.handle_key(KeyCode::Char('r'));
        assert_eq!(app.view.sort_state.direction, SortDirection::Descending);
    }

    #[test]
    fn sort_direction_resets_on_new_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(KeyCode::Char('i'));
        assert_eq!(app.view.sort_state.key, SortKey::Pid);
        assert_eq!(app.view.sort_state.direction, SortDirection::Ascending);
    }

    #[test]
    fn j_and_k_move_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2), sample_row(3)];
        app.set_viewport_rows(3);
        app.view.selected = 1;

        assert_eq!(app.handle_key(KeyCode::Char('j')), KeyAction::Continue);
        assert_eq!(app.view.selected, 2);

        assert_eq!(app.handle_key(KeyCode::Char('j')), KeyAction::Continue);
        assert_eq!(app.view.selected, 2);

        assert_eq!(app.handle_key(KeyCode::Char('k')), KeyAction::Continue);
        assert_eq!(app.view.selected, 1);

        assert_eq!(app.handle_key(KeyCode::Char('k')), KeyAction::Continue);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn q_requests_quit() {
        let mut app = AppState::new(ProcfsCollector::new());
        assert_eq!(app.handle_key(KeyCode::Char('q')), KeyAction::Quit);
    }

    #[test]
    fn left_click_selects_visible_row() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.set_viewport_rows(3);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.selected, 1);
    }

    #[test]
    fn click_outside_data_rows_does_not_change_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        app.set_viewport_rows(3);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 2;

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

        assert_eq!(app.view.selected, 2);
    }

    #[test]
    fn wheel_scroll_up_changes_offset_not_selected() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 3;
        app.view.scroll_offset = 2;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
        assert_eq!(app.view.selected, 3);
    }

    #[test]
    fn wheel_scroll_down_changes_offset_not_selected() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 0;
        app.view.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn wheel_scroll_clamps_at_bounds() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.view.scroll_offset, 0);

        app.view.scroll_offset = 2;
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.view.scroll_offset, 2);
    }

    #[test]
    fn wheel_outside_table_does_not_change_state() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(0, 0, 40, 8));
        app.view.selected = 2;
        app.view.scroll_offset = 1;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 41,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
        assert_eq!(app.view.selected, 2);
    }

    #[test]
    fn wheel_on_table_border_is_handled() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
        ];
        app.set_viewport_rows(2);
        app.set_process_table_area(Rect::new(5, 2, 40, 8));
        app.view.scroll_offset = 0;

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(app.view.scroll_offset, 1);
    }

    #[test]
    fn set_viewport_rows_does_not_force_selected_visible() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
            sample_row(50),
        ];
        app.view.selected = 0;
        app.view.scroll_offset = 2;

        app.set_viewport_rows(2);

        assert_eq!(app.view.scroll_offset, 2);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn restore_selection_without_ensure_visible_keeps_scroll_offset() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![
            sample_row(10),
            sample_row(20),
            sample_row(30),
            sample_row(40),
            sample_row(50),
        ];
        app.set_viewport_rows(2);
        app.view.selected = 0;
        app.view.scroll_offset = 3;

        app.restore_selection(Some(10), false);

        assert_eq!(app.view.selected, 0);
        assert_eq!(app.view.scroll_offset, 3);
    }

    #[test]
    fn tree_gutter_click_toggles_expansion() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
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
            .visible_row_range()
            .filter_map(|visible_index| app.process_index_at_visible_row(visible_index))
            .map(|index| app.data.snapshot.processes[index].pid)
            .collect();
        assert_eq!(visible, vec![1, 2, 3]);
    }

    #[test]
    fn tree_name_branch_click_selects_without_toggling() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.data.snapshot.processes = vec![tree_row(1, 0), tree_row(2, 1), tree_row(3, 0)];
        app.rebuild_tree_rows();
        app.tree.expanded_pids.insert(1);
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
            .visible_row_range()
            .filter_map(|visible_index| app.process_index_at_visible_row(visible_index))
            .map(|index| app.data.snapshot.processes[index].pid)
            .collect();
        assert_eq!(visible, vec![1, 2, 3]);
    }
}
