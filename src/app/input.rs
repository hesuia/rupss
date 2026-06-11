use super::{
    AppState, KeyAction, ProcessColumn, ViewMode,
    navigation::{
        boundary_selection, clamp_scroll_offset, ensure_visible_scroll, is_inside_table,
        move_visible_index, table_hit_test,
    },
    state::FilterRow,
};
use crate::snapshot::{SortDirection, SortKey, SortState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use strum::{EnumCount, IntoEnumIterator};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppCommand {
    Quit,
    MoveSelection(isize),
    JumpToBoundary { to_start: bool },
    TreeLeft,
    TreeRight,
    ToggleViewMode,
    ToggleColumnPicker,
    ToggleSortPicker,
    ToggleFilterModal,
    MoveFilterSelection(isize),
    ToggleFilterEditing,
    FilterPushChar(char),
    FilterPopChar,
    FilterClear,
    CycleFilterOperator(isize),
    MoveColumnPicker(isize),
    MoveSortPicker(isize),
    ReorderSelectedColumn(isize),
    ToggleSelectedColumn,
    ApplySelectedSort,
    TogglePause,
    CloseOverlay,
    StartProcessMonitor,
    CloseProcessMonitorOverlay,
    Noop,
}

impl AppState {
    /// Applies one keyboard action.
    ///
    /// Returns a [`KeyAction`] that tells the caller whether to continue
    /// running or terminate the application.
    /// The mapping is intentionally small and focused on fast navigation.
    pub fn handle_key(&mut self, key: KeyEvent) -> KeyAction {
        self.apply_command(key_command(
            key,
            self.view.viewport_rows,
            self.view.column_picker_open,
            self.view.sort_picker_open,
            self.view.filter_modal.open,
            self.view.filter_modal.editing,
            self.view.process_monitor_open,
        ))
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
            AppCommand::ToggleColumnPicker => {
                self.toggle_column_picker();
                KeyAction::Continue
            }
            AppCommand::ToggleSortPicker => {
                self.toggle_sort_picker();
                KeyAction::Continue
            }
            AppCommand::ToggleFilterModal => {
                self.toggle_filter_modal();
                KeyAction::Continue
            }
            AppCommand::MoveFilterSelection(delta) => {
                self.move_filter_selection(delta);
                KeyAction::Continue
            }
            AppCommand::ToggleFilterEditing => {
                self.toggle_filter_editing();
                KeyAction::Continue
            }
            AppCommand::FilterPushChar(ch) => {
                self.filter_push_char(ch);
                KeyAction::Continue
            }
            AppCommand::FilterPopChar => {
                self.filter_pop_char();
                KeyAction::Continue
            }
            AppCommand::FilterClear => {
                self.filter_clear();
                KeyAction::Continue
            }
            AppCommand::CycleFilterOperator(delta) => {
                self.filter_cycle_operator(delta);
                KeyAction::Continue
            }
            AppCommand::MoveColumnPicker(delta) => {
                self.move_column_picker(delta);
                KeyAction::Continue
            }
            AppCommand::MoveSortPicker(delta) => {
                self.move_sort_picker(delta);
                KeyAction::Continue
            }
            AppCommand::ReorderSelectedColumn(delta) => {
                self.reorder_selected_column(delta);
                KeyAction::Continue
            }
            AppCommand::ToggleSelectedColumn => {
                self.toggle_selected_column();
                KeyAction::Continue
            }
            AppCommand::ApplySelectedSort => self.apply_selected_sort_and_continue(),
            AppCommand::TogglePause => {
                self.toggle_pause();
                KeyAction::Continue
            }
            AppCommand::CloseOverlay => {
                self.close_overlay();
                KeyAction::Continue
            }
            AppCommand::StartProcessMonitor => {
                self.start_process_monitor_for_selected();
                KeyAction::Continue
            }
            AppCommand::CloseProcessMonitorOverlay => {
                self.close_process_monitor_overlay();
                KeyAction::Continue
            }
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
        let flat_indexes = self.current_flat_process_indexes();
        if flat_indexes.is_empty() {
            self.view.selected = 0;
            self.view.scroll_offset = 0;
            return;
        }

        self.view.selected = selected_pid
            .and_then(|pid| {
                flat_indexes
                    .iter()
                    .copied()
                    .find(|index| self.data.snapshot.processes[*index].pid == pid)
            })
            .unwrap_or(flat_indexes[0]);

        if self.view.view_mode == ViewMode::Tree {
            self.ensure_tree_selection_visible();
            if self.selected_visible_index().is_none() {
                self.view.selected = self
                    .tree
                    .rows
                    .first()
                    .map(|row| row.process_index)
                    .unwrap_or(self.view.selected);
            }
        }

        if ensure_visible {
            self.ensure_visible();
        } else {
            self.clamp_scroll_offset();
        }
    }

    pub(super) fn ensure_visible(&mut self) {
        let selected_visible = match self.view.view_mode {
            ViewMode::Flat => self.selected_visible_index().unwrap_or(0),
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

    fn apply_selected_sort_and_continue(&mut self) -> KeyAction {
        if let Some(sort_key) = self.selected_sort_key() {
            self.resort(sort_key);
        }
        KeyAction::Continue
    }

    fn move_selection_and_continue(&mut self, delta: isize) -> KeyAction {
        self.move_selection(delta);
        KeyAction::Continue
    }

    fn jump_to_boundary_and_continue(&mut self, to_start: bool) -> KeyAction {
        self.view.selected = boundary_selection(
            self.view.view_mode,
            &self.current_flat_process_indexes(),
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
        self.rebuild_filtered_indexes();
        self.rebuild_tree_rows();
        self.restore_selection(selected_pid, true);
        self.populate_visible_details();
        self.save_config();
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
            ViewMode::Flat => match self.flat_process_index_at(next) {
                Some(process_index) => process_index,
                None => return,
            },
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
        self.save_config();
    }

    fn toggle_column_picker(&mut self) {
        self.view.column_picker_open = !self.view.column_picker_open;
        if self.view.column_picker_open {
            self.view.sort_picker_open = false;
            self.view.filter_modal.open = false;
            self.view.filter_modal.editing = false;
        }
        self.view.column_picker_index = self
            .view
            .column_picker_index
            .min(self.view.columns.ordered_columns().len().saturating_sub(1));
    }

    fn toggle_sort_picker(&mut self) {
        self.view.sort_picker_open = !self.view.sort_picker_open;
        if self.view.sort_picker_open {
            self.view.column_picker_open = false;
            self.view.filter_modal.open = false;
            self.view.filter_modal.editing = false;
            self.view.sort_picker_index = sort_key_index(self.view.sort_state.key);
        }
    }

    fn close_overlay(&mut self) {
        self.view.column_picker_open = false;
        self.view.sort_picker_open = false;
        self.view.filter_modal.open = false;
        self.view.filter_modal.editing = false;
        self.view.process_monitor_open = false;
    }

    fn toggle_filter_modal(&mut self) {
        let now_open = !self.view.filter_modal.open;
        self.view.filter_modal.open = now_open;
        self.view.filter_modal.editing = false;
        if now_open {
            self.view.column_picker_open = false;
            self.view.sort_picker_open = false;
        }
    }

    fn move_filter_selection(&mut self, delta: isize) {
        if !self.view.filter_modal.open || self.view.filter_modal.editing {
            return;
        }

        self.view.filter_modal.selected = moved_index(
            self.view.filter_modal.selected,
            delta,
            FilterRow::COUNT.saturating_sub(1),
        );
    }

    fn toggle_filter_editing(&mut self) {
        if !self.view.filter_modal.open {
            return;
        }

        self.view.filter_modal.editing = !self.view.filter_modal.editing;
    }

    fn filter_push_char(&mut self, ch: char) {
        if !self.view.filter_modal.open || !self.view.filter_modal.editing {
            return;
        }

        self.filter_active_input_mut().push(ch);
        self.apply_filter_modal();
    }

    fn filter_pop_char(&mut self) {
        if !self.view.filter_modal.open || !self.view.filter_modal.editing {
            return;
        }

        self.filter_active_input_mut().pop();
        self.apply_filter_modal();
    }

    fn filter_clear(&mut self) {
        if !self.view.filter_modal.open || !self.view.filter_modal.editing {
            return;
        }

        self.filter_active_input_mut().clear();
        self.apply_filter_modal();
    }

    fn filter_cycle_operator(&mut self, delta: isize) {
        if !self.view.filter_modal.open || self.view.filter_modal.editing {
            return;
        }

        let row = self.filter_selected_row();
        if !row.is_metric() {
            return;
        }

        let op_ref = match row {
            FilterRow::Rss => &mut self.view.filter_modal.rss_op,
            FilterRow::Swap => &mut self.view.filter_modal.swap_op,
            FilterRow::Cpu => &mut self.view.filter_modal.cpu_op,
            FilterRow::Uss => &mut self.view.filter_modal.uss_op,
            FilterRow::Pss => &mut self.view.filter_modal.pss_op,
            FilterRow::Pid | FilterRow::Ppid | FilterRow::Name | FilterRow::Command => return,
        };
        *op_ref = op_ref.cycle(delta);
        self.apply_filter_modal();
    }

    fn apply_filter_modal(&mut self) {
        let selected_pid = self.selected_pid();
        let (filter, error) = super::filter::try_build_filter_from_modal(&self.view.filter_modal);
        self.view.filter = filter;
        self.view.filter_modal.error = error;
        self.populate_filter_details();
        self.rebuild_filtered_indexes();
        self.rebuild_tree_rows();
        self.restore_selection(selected_pid, true);
        self.populate_visible_details();
        self.save_config();
    }

    fn filter_selected_row(&self) -> FilterRow {
        FilterRow::from_index(self.view.filter_modal.selected).unwrap_or(FilterRow::Pid)
    }

    fn filter_active_input_mut(&mut self) -> &mut String {
        match self.filter_selected_row() {
            FilterRow::Pid => &mut self.view.filter_modal.pid,
            FilterRow::Ppid => &mut self.view.filter_modal.ppid,
            FilterRow::Name => &mut self.view.filter_modal.name,
            FilterRow::Command => &mut self.view.filter_modal.command,
            FilterRow::Rss => &mut self.view.filter_modal.rss_value,
            FilterRow::Swap => &mut self.view.filter_modal.swap_value,
            FilterRow::Cpu => &mut self.view.filter_modal.cpu_value,
            FilterRow::Uss => &mut self.view.filter_modal.uss_value,
            FilterRow::Pss => &mut self.view.filter_modal.pss_value,
        }
    }

    fn move_column_picker(&mut self, delta: isize) {
        if !self.view.column_picker_open {
            return;
        }

        let len = self.view.columns.ordered_columns().len();
        let current = self.view.column_picker_index;
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize)
        }
        .min(len.saturating_sub(1));
        self.view.column_picker_index = next;
    }

    fn move_sort_picker(&mut self, delta: isize) {
        if !self.view.sort_picker_open {
            return;
        }

        self.view.sort_picker_index = moved_index(
            self.view.sort_picker_index,
            delta,
            SortKey::iter().count().saturating_sub(1),
        );
    }

    fn selected_sort_key(&self) -> Option<SortKey> {
        SortKey::iter().nth(self.view.sort_picker_index)
    }

    fn toggle_selected_column(&mut self) {
        if !self.view.column_picker_open {
            return;
        }

        let Some(column) = self.view.columns.column_at(self.view.column_picker_index) else {
            return;
        };
        if self.view.columns.toggle(column) {
            self.ensure_sort_key_visible();
            self.populate_visible_details();
            self.save_config();
        }
    }

    fn reorder_selected_column(&mut self, delta: isize) {
        if !self.view.column_picker_open {
            return;
        }

        if let Some(next_index) = self
            .view
            .columns
            .move_in_order(self.view.column_picker_index, delta)
        {
            self.view.column_picker_index = next_index;
            self.save_config();
        }
    }

    fn ensure_sort_key_visible(&mut self) {
        let Some(sort_column) = ProcessColumn::from_sort_key(self.view.sort_state.key) else {
            return;
        };
        if self.view.columns.is_visible(sort_column) {
            return;
        }

        let selected_pid = self.selected_pid();
        self.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);
        self.sort_current_processes();
        self.rebuild_filtered_indexes();
        self.rebuild_tree_rows();
        self.restore_selection(selected_pid, true);
    }
}

fn key_command(
    key: KeyEvent,
    viewport_rows: usize,
    column_picker_open: bool,
    sort_picker_open: bool,
    filter_modal_open: bool,
    filter_modal_editing: bool,
    process_monitor_open: bool,
) -> AppCommand {
    let code = key.code;
    if process_monitor_open {
        return match code {
            KeyCode::Char('q') => AppCommand::Quit,
            KeyCode::Esc => AppCommand::CloseProcessMonitorOverlay,
            _ => AppCommand::Noop,
        };
    }

    if filter_modal_open {
        if filter_modal_editing {
            return match code {
                KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    AppCommand::Quit
                }
                KeyCode::Esc => AppCommand::CloseOverlay,
                KeyCode::Enter => AppCommand::ToggleFilterEditing,
                KeyCode::Backspace => AppCommand::FilterPopChar,
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    AppCommand::FilterClear
                }
                KeyCode::Char(ch)
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    AppCommand::FilterPushChar(ch)
                }
                _ => AppCommand::Noop,
            };
        }

        return match code {
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => AppCommand::Quit,
            KeyCode::Esc => AppCommand::CloseOverlay,
            KeyCode::Enter => AppCommand::ToggleFilterEditing,
            KeyCode::Up | KeyCode::Char('k') => AppCommand::MoveFilterSelection(-1),
            KeyCode::Down | KeyCode::Char('j') => AppCommand::MoveFilterSelection(1),
            KeyCode::Left | KeyCode::Char('h') => AppCommand::CycleFilterOperator(-1),
            KeyCode::Right | KeyCode::Char('l') => AppCommand::CycleFilterOperator(1),
            KeyCode::Backspace => AppCommand::FilterPopChar,
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppCommand::FilterClear
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                AppCommand::FilterPushChar(ch)
            }
            _ => AppCommand::Noop,
        };
    }

    if column_picker_open {
        return match code {
            KeyCode::Char('q') => AppCommand::Quit,
            KeyCode::Esc => AppCommand::CloseOverlay,
            KeyCode::Char('v') => AppCommand::ToggleColumnPicker,
            KeyCode::Char('s') => AppCommand::ToggleSortPicker,
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                AppCommand::ReorderSelectedColumn(-1)
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                AppCommand::ReorderSelectedColumn(1)
            }
            KeyCode::Char('K') => AppCommand::ReorderSelectedColumn(-1),
            KeyCode::Char('J') => AppCommand::ReorderSelectedColumn(1),
            KeyCode::Up | KeyCode::Char('k') => AppCommand::MoveColumnPicker(-1),
            KeyCode::Down | KeyCode::Char('j') => AppCommand::MoveColumnPicker(1),
            KeyCode::Char(' ') | KeyCode::Enter => AppCommand::ToggleSelectedColumn,
            _ => AppCommand::Noop,
        };
    }

    if sort_picker_open {
        return match code {
            KeyCode::Char('q') => AppCommand::Quit,
            KeyCode::Esc => AppCommand::CloseOverlay,
            KeyCode::Char('v') => AppCommand::ToggleColumnPicker,
            KeyCode::Char('s') => AppCommand::ToggleSortPicker,
            KeyCode::Up | KeyCode::Char('k') => AppCommand::MoveSortPicker(-1),
            KeyCode::Down | KeyCode::Char('j') => AppCommand::MoveSortPicker(1),
            KeyCode::Char(' ') | KeyCode::Enter => AppCommand::ApplySelectedSort,
            _ => AppCommand::Noop,
        };
    }

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
        KeyCode::Char('v') => AppCommand::ToggleColumnPicker,
        KeyCode::Char('s') => AppCommand::ToggleSortPicker,
        KeyCode::Char('f') => AppCommand::ToggleFilterModal,
        KeyCode::Char('p') => AppCommand::TogglePause,
        KeyCode::Enter => AppCommand::StartProcessMonitor,
        _ => AppCommand::Noop,
    }
}

fn moved_index(current: usize, delta: isize, max_index: usize) -> usize {
    if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize)
    }
    .min(max_index)
}

fn sort_key_index(sort_key: SortKey) -> usize {
    SortKey::iter()
        .position(|key| key == sort_key)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{AppCommand, AppState, KeyAction, key_command};
    use crate::app::filter::ProcessFilter;
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
    use crate::{app::ProcessColumn, collector::ProcfsCollector};
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Rect;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shifted(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn controlled(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

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
            pss_bytes: Some(pid as u64), // Just for testing, we can have PSS equal to PID
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
    fn key_command_maps_page_navigation() {
        assert_eq!(
            key_command(key(KeyCode::PageUp), 3, false, false, false, false, false),
            AppCommand::MoveSelection(-3)
        );
        assert_eq!(
            key_command(key(KeyCode::PageDown), 3, false, false, false, false, false),
            AppCommand::MoveSelection(3)
        );
    }

    #[test]
    fn key_command_maps_pause_toggle() {
        assert_eq!(
            key_command(
                key(KeyCode::Char('p')),
                3,
                false,
                false,
                false,
                false,
                false
            ),
            AppCommand::TogglePause
        );
    }

    #[test]
    fn key_command_maps_enter_to_process_monitor_from_table() {
        assert_eq!(
            key_command(key(KeyCode::Enter), 3, false, false, false, false, false),
            AppCommand::StartProcessMonitor
        );
    }

    #[test]
    fn key_command_uses_process_monitor_bindings_when_open() {
        assert_eq!(
            key_command(key(KeyCode::Esc), 3, false, false, false, false, true),
            AppCommand::CloseProcessMonitorOverlay
        );
        assert_eq!(
            key_command(key(KeyCode::Char('q')), 3, false, false, false, false, true),
            AppCommand::Quit
        );
        assert_eq!(
            key_command(key(KeyCode::Enter), 3, false, false, false, false, true),
            AppCommand::Noop
        );
    }

    #[test]
    fn key_command_uses_column_picker_bindings_when_open() {
        assert_eq!(
            key_command(key(KeyCode::Down), 3, true, false, false, false, false),
            AppCommand::MoveColumnPicker(1)
        );
        assert_eq!(
            key_command(key(KeyCode::Enter), 3, true, false, false, false, false),
            AppCommand::ToggleSelectedColumn
        );
        assert_eq!(
            key_command(shifted(KeyCode::Up), 3, true, false, false, false, false),
            AppCommand::ReorderSelectedColumn(-1)
        );
        assert_eq!(
            key_command(key(KeyCode::Char('J')), 3, true, false, false, false, false),
            AppCommand::ReorderSelectedColumn(1)
        );
    }

    #[test]
    fn key_command_uses_sort_picker_bindings_when_open() {
        assert_eq!(
            key_command(key(KeyCode::Down), 3, false, true, false, false, false),
            AppCommand::MoveSortPicker(1)
        );
        assert_eq!(
            key_command(key(KeyCode::Enter), 3, false, true, false, false, false),
            AppCommand::ApplySelectedSort
        );
        assert_eq!(
            key_command(key(KeyCode::Esc), 3, false, true, false, false, false),
            AppCommand::CloseOverlay
        );
    }

    #[test]
    fn key_command_uses_filter_modal_bindings_when_open() {
        assert_eq!(
            key_command(key(KeyCode::Char('a')), 3, false, false, true, true, false),
            AppCommand::FilterPushChar('a')
        );
        assert_eq!(
            key_command(key(KeyCode::Backspace), 3, false, false, true, true, false),
            AppCommand::FilterPopChar
        );
        assert_eq!(
            key_command(
                controlled(KeyCode::Char('u')),
                3,
                false,
                false,
                true,
                true,
                false
            ),
            AppCommand::FilterClear
        );
        assert_eq!(
            key_command(key(KeyCode::Enter), 3, false, false, true, true, false),
            AppCommand::ToggleFilterEditing
        );
        assert_eq!(
            key_command(key(KeyCode::Char('p')), 3, false, false, true, true, false),
            AppCommand::FilterPushChar('p')
        );
    }

    #[test]
    fn f_opens_filter_modal() {
        let mut app = AppState::new(ProcfsCollector::new());

        app.handle_key(key(KeyCode::Char('f')));

        assert!(app.view.filter_modal.open);
    }

    #[test]
    fn enter_opens_process_monitor_and_esc_closes_overlay_only() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.replace_processes_for_test(vec![sample_row(10)]);

        app.handle_key(key(KeyCode::Enter));
        assert!(app.view.process_monitor_open);
        assert!(app.data.process_monitor.is_some());

        app.handle_key(key(KeyCode::Esc));
        assert!(!app.view.process_monitor_open);
        assert!(app.data.process_monitor.is_some());
    }

    #[test]
    fn enter_closes_other_overlays_when_monitoring() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.replace_processes_for_test(vec![sample_row(10)]);
        app.view.column_picker_open = true;
        assert!(app.view.column_picker_open);

        app.start_process_monitor_for_selected();

        assert!(!app.view.column_picker_open);
        assert!(app.view.process_monitor_open);
    }

    #[test]
    fn filter_modal_filters_name() {
        let mut app = AppState::new(ProcfsCollector::new());
        let mut ssh = sample_row(10);
        ssh.name = "sshd".to_string();
        ssh.command = "/usr/sbin/sshd".to_string();
        let mut postgres = sample_row(20);
        postgres.name = "postgres".to_string();
        postgres.command = "postgres: checkpointer".to_string();
        app.data.snapshot.processes = vec![ssh, postgres];
        app.rebuild_filtered_indexes();

        app.handle_key(key(KeyCode::Char('f')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j'))); // move to NAME row
        app.handle_key(key(KeyCode::Enter)); // start editing
        for ch in "post".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Enter)); // stop editing

        assert_eq!(app.view.filtered_indexes, vec![1]);
        assert_eq!(app.selected_pid(), Some(20));
    }

    #[test]
    fn filter_modal_filters_metric_threshold() {
        let mut app = AppState::new(ProcfsCollector::new());
        let mut small = sample_row(10);
        small.rss_bytes = 99 * 1024 * 1024;
        let mut large = sample_row(20);
        large.rss_bytes = 100 * 1024 * 1024;
        app.data.snapshot.processes = vec![small, large];
        app.rebuild_filtered_indexes();

        app.handle_key(key(KeyCode::Char('f')));
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Char('j'))); // move to RSS row
        }
        app.handle_key(key(KeyCode::Enter)); // edit
        for ch in "100MB".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Enter)); // stop edit

        assert_eq!(app.view.filtered_indexes, vec![1]);
        assert_eq!(app.selected_pid(), Some(20));
    }

    #[test]
    fn filter_modal_backspace_and_ctrl_u_update_visible_rows() {
        let mut app = AppState::new(ProcfsCollector::new());
        let mut alpha = sample_row(10);
        alpha.name = "alpha".to_string();
        let mut beta = sample_row(20);
        beta.name = "beta".to_string();
        app.data.snapshot.processes = vec![alpha, beta];
        app.rebuild_filtered_indexes();

        app.handle_key(key(KeyCode::Char('f')));
        assert_eq!(app.view.filter_modal.selected, 0);
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j'))); // NAME
        assert_eq!(app.view.filter_modal.selected, 2);
        app.handle_key(key(KeyCode::Enter)); // edit
        assert!(app.view.filter_modal.editing);
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Char('l')));
        assert_eq!(app.view.filtered_indexes, vec![0]);

        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.view.filtered_indexes, vec![0, 1]);

        app.handle_key(controlled(KeyCode::Char('u')));
        assert_eq!(app.view.filtered_indexes, vec![0, 1]);
    }

    #[test]
    fn filtered_flat_navigation_moves_between_matching_process_indexes() {
        let mut app = AppState::new(ProcfsCollector::new());
        let mut first = sample_row(10);
        first.name = "skip".to_string();
        let mut second = sample_row(20);
        second.name = "target-a".to_string();
        let mut third = sample_row(30);
        third.name = "skip".to_string();
        let mut fourth = sample_row(40);
        fourth.name = "target-b".to_string();
        app.data.snapshot.processes = vec![first, second, third, fourth];
        app.rebuild_filtered_indexes();

        app.handle_key(key(KeyCode::Char('f')));
        app.handle_key(key(KeyCode::Char('j')));
        app.handle_key(key(KeyCode::Char('j'))); // NAME
        app.handle_key(key(KeyCode::Enter)); // edit
        for ch in "target".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Enter)); // stop edit
        app.handle_key(key(KeyCode::Esc)); // close modal

        assert_eq!(app.view.filtered_indexes, vec![1, 3]);
        assert_eq!(app.selected_pid(), Some(20));

        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.selected_pid(), Some(40));

        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.selected_pid(), Some(20));

        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.selected_pid(), Some(40));

        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.selected_pid(), Some(20));
    }

    #[test]
    fn filtered_flat_ensure_visible_uses_visible_position_not_process_index() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = (10..70).step_by(10).map(sample_row).collect();
        app.view.filter = ProcessFilter::from_text_query("name");
        app.rebuild_filtered_indexes();
        app.view.filtered_indexes = vec![1, 3, 5];
        app.view.selected = 5;
        app.view.viewport_rows = 2;

        app.ensure_visible();

        assert_eq!(app.view.scroll_offset, 1);
    }

    #[test]
    fn sort_direction_toggles_on_same_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(key(KeyCode::Char('s')));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.view.sort_state.direction, SortDirection::Ascending);

        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.view.sort_state.direction, SortDirection::Descending);
    }

    #[test]
    fn sort_direction_resets_on_new_key() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(key(KeyCode::Char('s')));
        app.view.sort_picker_index = 0;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.view.sort_state.key, SortKey::Pid);
        assert_eq!(app.view.sort_state.direction, SortDirection::Ascending);
    }

    #[test]
    fn old_sort_shortcuts_do_not_resort_from_table() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Rss, SortDirection::Descending);

        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Char('c')));

        assert_eq!(
            app.view.sort_state,
            SortState::new(SortKey::Rss, SortDirection::Descending)
        );
    }

    #[test]
    fn j_and_k_move_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2), sample_row(3)];
        app.set_viewport_rows(3);
        app.view.selected = 1;

        assert_eq!(app.handle_key(key(KeyCode::Char('j'))), KeyAction::Continue);
        assert_eq!(app.view.selected, 2);

        assert_eq!(app.handle_key(key(KeyCode::Char('j'))), KeyAction::Continue);
        assert_eq!(app.view.selected, 2);

        assert_eq!(app.handle_key(key(KeyCode::Char('k'))), KeyAction::Continue);
        assert_eq!(app.view.selected, 1);

        assert_eq!(app.handle_key(key(KeyCode::Char('k'))), KeyAction::Continue);
        assert_eq!(app.view.selected, 0);
    }

    #[test]
    fn q_requests_quit() {
        let mut app = AppState::new(ProcfsCollector::new());
        assert_eq!(app.handle_key(key(KeyCode::Char('q'))), KeyAction::Quit);
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
        app.handle_key(key(KeyCode::Char('t')));
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
        app.handle_key(key(KeyCode::Char('t')));
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

    #[test]
    fn v_toggles_column_picker() {
        let mut app = AppState::new(ProcfsCollector::new());

        assert!(!app.view.column_picker_open);
        app.handle_key(key(KeyCode::Char('v')));
        assert!(app.view.column_picker_open);
        app.handle_key(key(KeyCode::Char('v')));
        assert!(!app.view.column_picker_open);
    }

    #[test]
    fn s_toggles_sort_picker() {
        let mut app = AppState::new(ProcfsCollector::new());

        assert!(!app.view.sort_picker_open);
        app.handle_key(key(KeyCode::Char('s')));
        assert!(app.view.sort_picker_open);
        app.handle_key(key(KeyCode::Char('s')));
        assert!(!app.view.sort_picker_open);
    }

    #[test]
    fn p_toggles_pause() {
        let mut app = AppState::new(ProcfsCollector::new());

        assert!(!app.is_paused());
        app.handle_key(key(KeyCode::Char('p')));
        assert!(app.is_paused());
        app.handle_key(key(KeyCode::Char('p')));
        assert!(!app.is_paused());
    }

    #[test]
    fn opening_one_picker_closes_the_other() {
        let mut app = AppState::new(ProcfsCollector::new());

        app.handle_key(key(KeyCode::Char('v')));
        assert!(app.view.column_picker_open);

        app.handle_key(key(KeyCode::Char('s')));

        assert!(!app.view.column_picker_open);
        assert!(app.view.sort_picker_open);
    }

    #[test]
    fn sort_picker_can_move_selection() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.handle_key(key(KeyCode::Char('s')));

        app.handle_key(key(KeyCode::Char('j')));

        assert_eq!(app.view.sort_picker_index, 6);

        app.handle_key(key(KeyCode::Char('k')));

        assert_eq!(app.view.sort_picker_index, 5);
    }

    #[test]
    fn column_picker_can_toggle_visible_column() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.handle_key(key(KeyCode::Char('v')));
        app.view.column_picker_index = 5;

        app.handle_key(key(KeyCode::Enter));

        assert!(!app.view.columns.is_visible(super::ProcessColumn::Command));
    }

    #[test]
    fn hiding_sorted_column_falls_back_to_rss_sort() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.data.snapshot.processes = vec![sample_row(1), sample_row(2)];
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.handle_key(key(KeyCode::Char('v')));
        app.view.column_picker_index = 0;

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(
            app.view.sort_state,
            SortState::new(SortKey::Rss, SortDirection::Descending)
        );
    }

    #[test]
    fn column_picker_reorders_selected_column() {
        let mut app = AppState::new(ProcfsCollector::new());
        app.handle_key(key(KeyCode::Char('v')));
        app.view.column_picker_index = 0;

        app.handle_key(shifted(KeyCode::Down));

        assert_eq!(app.view.column_picker_index, 1);
        assert_eq!(app.visible_columns()[0], ProcessColumn::Ppid);
        assert_eq!(app.visible_columns()[1], ProcessColumn::Pid);
    }
}
