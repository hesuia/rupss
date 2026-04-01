use super::{TreeRow, ViewMode};
use crate::snapshot::ProcessRow;
use ratatui::layout::Rect;

pub(super) fn clamp_scroll_offset(
    scroll_offset: usize,
    total_rows: usize,
    viewport_rows: usize,
) -> usize {
    let max_offset = total_rows.saturating_sub(viewport_rows.max(1));
    scroll_offset.min(max_offset)
}

pub(super) fn ensure_visible_scroll(
    selected_visible: usize,
    scroll_offset: usize,
    viewport_rows: usize,
    total_rows: usize,
) -> usize {
    let viewport_rows = viewport_rows.max(1);
    let next_offset = if selected_visible < scroll_offset {
        selected_visible
    } else {
        let view_end = scroll_offset.saturating_add(viewport_rows);
        if selected_visible >= view_end {
            selected_visible.saturating_sub(viewport_rows.saturating_sub(1))
        } else {
            scroll_offset
        }
    };

    clamp_scroll_offset(next_offset, total_rows, viewport_rows)
}

pub(super) fn restored_selection(selected_pid: Option<i32>, processes: &[ProcessRow]) -> usize {
    if processes.is_empty() {
        return 0;
    }

    selected_pid
        .and_then(|pid| processes.iter().position(|row| row.pid == pid))
        .unwrap_or(0)
        .min(processes.len().saturating_sub(1))
}

pub(super) fn move_visible_index(
    current: Option<usize>,
    delta: isize,
    total_rows: usize,
) -> Option<usize> {
    let current = current?;
    if total_rows == 0 {
        return None;
    }

    let max_index = total_rows.saturating_sub(1) as isize;
    Some((current as isize + delta).clamp(0, max_index) as usize)
}

pub(super) fn boundary_selection(
    view_mode: ViewMode,
    process_count: usize,
    tree_rows: &[TreeRow],
    to_start: bool,
) -> usize {
    match (view_mode, to_start) {
        (ViewMode::Flat, true) => 0,
        (ViewMode::Flat, false) => process_count.saturating_sub(1),
        (ViewMode::Tree, true) => tree_rows.first().map(|row| row.process_index).unwrap_or(0),
        (ViewMode::Tree, false) => tree_rows.last().map(|row| row.process_index).unwrap_or(0),
    }
}

pub(super) fn table_hit_test(area: Rect, column: u16, row: u16) -> Option<usize> {
    if area.width < 3 || area.height < 4 {
        return None;
    }

    let left = area.x.saturating_add(1);
    let right = area.x.saturating_add(area.width.saturating_sub(2));
    if column < left || column > right {
        return None;
    }

    let first_data_row = area.y.saturating_add(2);
    let last_data_row = area.y.saturating_add(area.height.saturating_sub(2));
    if row < first_data_row || row > last_data_row {
        return None;
    }

    Some(row.saturating_sub(first_data_row) as usize)
}

pub(super) fn is_inside_table(area: Rect, column: u16, row: u16) -> bool {
    let right = area.x.saturating_add(area.width.saturating_sub(1));
    let bottom = area.y.saturating_add(area.height.saturating_sub(1));
    area.x <= column && column <= right && area.y <= row && row <= bottom
}

#[cfg(test)]
mod tests {
    use super::{
        TreeRow, ViewMode, boundary_selection, clamp_scroll_offset, ensure_visible_scroll,
        move_visible_index, restored_selection, table_hit_test,
    };
    use crate::snapshot::ProcessRow;
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
            pss_bytes: None,
            base_swap_bytes: 0,
            detailed_swap_bytes: None,
            cpu_percent: 0.0,
        }
    }

    #[test]
    fn clamp_scroll_offset_limits_to_last_page() {
        assert_eq!(clamp_scroll_offset(8, 10, 3), 7);
        assert_eq!(clamp_scroll_offset(2, 2, 5), 0);
    }

    #[test]
    fn ensure_visible_scroll_moves_to_show_selected_row() {
        assert_eq!(ensure_visible_scroll(1, 4, 3, 10), 1);
        assert_eq!(ensure_visible_scroll(8, 4, 3, 10), 6);
    }

    #[test]
    fn restored_selection_prefers_matching_pid() {
        let processes = vec![sample_row(10), sample_row(20), sample_row(30)];
        assert_eq!(restored_selection(Some(20), &processes), 1);
        assert_eq!(restored_selection(Some(99), &processes), 0);
        assert_eq!(restored_selection(None, &[]), 0);
    }

    #[test]
    fn move_visible_index_clamps_at_edges() {
        assert_eq!(move_visible_index(Some(1), 2, 4), Some(3));
        assert_eq!(move_visible_index(Some(1), -3, 4), Some(0));
        assert_eq!(move_visible_index(None, 1, 4), None);
    }

    #[test]
    fn boundary_selection_uses_tree_rows_in_tree_mode() {
        let tree_rows = vec![
            TreeRow {
                process_index: 4,
                depth: 0,
                has_children: false,
                expanded: false,
                parent_index: None,
                is_last_sibling: false,
                ancestor_has_next_sibling: Vec::new(),
            },
            TreeRow {
                process_index: 7,
                depth: 0,
                has_children: false,
                expanded: false,
                parent_index: None,
                is_last_sibling: true,
                ancestor_has_next_sibling: Vec::new(),
            },
        ];

        assert_eq!(boundary_selection(ViewMode::Tree, 10, &tree_rows, true), 4);
        assert_eq!(boundary_selection(ViewMode::Tree, 10, &tree_rows, false), 7);
        assert_eq!(boundary_selection(ViewMode::Flat, 3, &tree_rows, false), 2);
    }

    #[test]
    fn table_hit_test_returns_visible_row_index() {
        let area = Rect::new(5, 2, 40, 8);
        assert_eq!(table_hit_test(area, 10, 4), Some(0));
        assert_eq!(table_hit_test(area, 10, 5), Some(1));
        assert_eq!(table_hit_test(area, 4, 4), None);
    }
}
