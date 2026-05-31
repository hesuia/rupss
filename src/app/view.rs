use crate::history::HistoryBuffer;
use std::ops::Range;

pub(super) fn visible_row_range(
    scroll_offset: usize,
    viewport_rows: usize,
    total_rows: usize,
) -> Range<usize> {
    let start = scroll_offset.min(total_rows);
    let end = start.saturating_add(viewport_rows).min(total_rows);
    start..end
}

/// Converts the fixed-size history buffer into chart coordinates.
///
/// X coordinates are sample indices, Y coordinates are raw bytes.
pub(super) fn history_points(history: &HistoryBuffer<u64>) -> Vec<(f64, f64)> {
    history
        .into_iter()
        .enumerate()
        .map(|(idx, value)| (idx as f64, *value as f64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{history_points, visible_row_range};
    use crate::history::HistoryBuffer;

    #[test]
    fn visible_row_range_clamps_to_last_page() {
        assert_eq!(visible_row_range(1, 2, 5), 1..3);
        assert_eq!(visible_row_range(10, 2, 5), 5..5);
    }

    #[test]
    fn chart_points_preserve_order() {
        let mut history = HistoryBuffer::new(4);
        history.push(10);
        history.push(20);

        assert_eq!(history_points(&history), vec![(0.0, 10.0), (1.0, 20.0)]);
    }
}
