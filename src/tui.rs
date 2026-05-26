use crate::{
    app::{AppState, FilterRow, ProcessColumn, ViewMode},
    format::{format_bytes, format_option_bytes, format_percent},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Cell, Chart, Clear, Dataset, Paragraph, Row, Table},
};

const COLUMN_PICKER_BACKGROUND: Color = Color::Black;
const COLUMN_PICKER_BORDER: Color = Color::Yellow;
const COLUMN_PICKER_SELECTED_BACKGROUND: Color = Color::Blue;

/// Draws the complete application frame.
///
/// Layout:
/// - Top: RSS history (left) and swap history (right).
/// - Middle: system summary panel.
/// - Bottom: process table.
pub fn render(frame: &mut Frame<'_>, app: &mut AppState) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11),
            Constraint::Length(5),
            Constraint::Min(8),
        ])
        .split(frame.area());

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(areas[0]);

    render_history_chart(frame, top[0], app, true);
    render_history_chart(frame, top[1], app, false);
    render_summary(frame, areas[1], app);
    render_process_table(frame, areas[2], app);
    if app.is_column_picker_open() {
        render_column_picker(frame, app);
    }
    if app.is_sort_picker_open() {
        render_sort_picker(frame, app);
    }
    if app.is_filter_modal_open() {
        render_filter_modal(frame, app);
    }
}

fn render_history_chart(frame: &mut Frame<'_>, area: Rect, app: &AppState, rss: bool) {
    let snapshot = app.snapshot();
    let (title, points, latest_value, y_axis_upper, color) = if rss {
        (
            "RSS History",
            app.rss_chart_points(),
            snapshot.system.total_process_rss,
            chart_y_upper_bound(snapshot.system.mem_total),
            Color::LightGreen,
        )
    } else {
        (
            "Swap History",
            app.swap_chart_points(),
            snapshot.system.total_process_swap,
            chart_y_upper_bound(snapshot.system.swap_total),
            Color::LightBlue,
        )
    };

    let x_max = points.last().map(|(x, _)| *x).unwrap_or(180.0).max(1.0);
    let y_max = y_axis_upper as f64;

    let datasets = vec![
        Dataset::default()
            .name(title)
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(color))
            .graph_type(ratatui::widgets::GraphType::Line)
            .data(&points),
    ];

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(format!("{title}  {}", format_bytes(latest_value)))
                .borders(Borders::ALL),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .labels([Line::from("-3m"), Line::from("now")]),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, y_max])
                .labels([Line::from("0"), Line::from(format_bytes(y_axis_upper))]),
        );

    frame.render_widget(chart, area);
}

fn chart_y_upper_bound(total_bytes: u64) -> u64 {
    total_bytes.max(1)
}

fn render_summary(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let snapshot = app.snapshot();
    let system = app.system_summary();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "Mem ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "used {} / total {} / avail {} / free {}",
                format_bytes(system.mem_used),
                format_bytes(system.mem_total),
                format_option_bytes(system.mem_available),
                format_bytes(system.mem_free),
            )),
        ]),
        Line::from(vec![
            Span::styled(
                "Swap ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "used {} / total {} / free {}",
                format_bytes(system.swap_used),
                format_bytes(system.swap_total),
                format_bytes(system.swap_free),
            )),
        ]),
        Line::from(vec![
            Span::styled(
                "Proc ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "count {} / shown {} / agg-rss {} / agg-swap {} / sort {} / mode {} / status {} / selected {} / age {}ms",
                system.process_count,
                app.filtered_process_count(),
                format_bytes(system.total_process_rss),
                format_bytes(system.total_process_swap),
                app.sort_state().label(),
                app.view_mode().as_ref(),
                if app.is_paused() { "paused" } else { "running" },
                app.selected_pid()
                    .map_or("-".to_string(), |pid| pid.to_string()),
                snapshot.captured_at.elapsed().as_millis()
            )),
        ]),
    ];

    if let Some(error) = app.last_error_message() {
        lines.push(Line::from(vec![
            Span::styled(
                "Last Error ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(error),
        ]));
    }

    let paragraph =
        Paragraph::new(lines).block(Block::default().title("System").borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn render_process_table(frame: &mut Frame<'_>, area: Rect, app: &mut AppState) {
    app.set_process_table_area(area);
    // The viewport height decides which rows are considered visible and therefore
    // which PIDs are eligible for `smaps_rollup` collection.
    app.set_viewport_rows(area.height.saturating_sub(3) as usize);
    let columns = app.visible_columns();
    let header = Row::new(columns.iter().map(|column| Cell::from(column.title()))).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let selected_visible_index = app.selected_visible_index();
    let rows = app.visible_row_range().map(|visible_index| {
        let process_index = app
            .process_index_at_visible_row(visible_index)
            .expect("visible row exists");
        let row = app.process_row(process_index).expect("visible row exists");
        let style = if selected_visible_index == Some(visible_index) {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default()
        };
        let name_cell = match (app.view_mode(), app.tree_row_at_visible_row(visible_index)) {
            (ViewMode::Flat, _) => row.name.clone(),
            (ViewMode::Tree, Some(tree_row)) => format_tree_name(tree_row, &row.name),
            (ViewMode::Tree, None) => row.name.clone(),
        };

        Row::new(
            columns
                .iter()
                .map(|column| process_table_cell(*column, row, &name_cell, app))
                .collect::<Vec<_>>(),
        )
        .style(style)
    });

    let filter_hint = if app.is_filter_modal_open() {
        let modal = app.filter_modal();
        let mode = if modal.editing { "edit" } else { "nav" };
        let err = if modal.error.is_some() { " !" } else { "" };
        format!(" filter[{mode}]{err}")
    } else if app.is_filter_active() {
        " filter[on]".to_string()
    } else {
        " filter[-]".to_string()
    };
    let pause_hint = if app.is_paused() {
        "p:resume"
    } else {
        "p:pause"
    };
    let title = format!(
        "Processes  q:quit  {pause_hint}  f:filter{filter_hint}  t:tree  v:columns  s:sort  arrows/jk:move  Left/Right:collapse/expand  click:select/toggle  PgUp/PgDn:page"
    );

    let table = Table::new(
        rows,
        columns
            .iter()
            .map(|column| Constraint::Length(column.width(app.view_mode())))
            .collect::<Vec<_>>(),
    )
    .header(header)
    .block(Block::default().title(title).borders(Borders::ALL))
    .column_spacing(crate::app::PROCESS_TABLE_COLUMN_SPACING);

    frame.render_widget(table, area);
}

fn render_filter_modal(frame: &mut Frame<'_>, app: &AppState) {
    let area = centered_rect(frame.area(), 56, 16);
    frame.render_widget(Clear, area);

    let modal = app.filter_modal();
    let selected_row = FilterRow::ALL
        .get(modal.selected)
        .copied()
        .unwrap_or(FilterRow::Pid);

    let rows = FilterRow::ALL.iter().enumerate().map(|(index, row)| {
        let selected = index == modal.selected;
        let style = if selected {
            Style::default()
                .bg(COLUMN_PICKER_SELECTED_BACKGROUND)
                .fg(Color::White)
        } else {
            Style::default().bg(COLUMN_PICKER_BACKGROUND)
        };

        let (op, value) = match row {
            FilterRow::Pid => ("", modal.pid.as_str()),
            FilterRow::Ppid => ("", modal.ppid.as_str()),
            FilterRow::Name => ("", modal.name.as_str()),
            FilterRow::Command => ("", modal.command.as_str()),
            FilterRow::Rss => (modal.rss_op.label(), modal.rss_value.as_str()),
            FilterRow::Swap => (modal.swap_op.label(), modal.swap_value.as_str()),
            FilterRow::Cpu => (modal.cpu_op.label(), modal.cpu_value.as_str()),
            FilterRow::Uss => (modal.uss_op.label(), modal.uss_value.as_str()),
            FilterRow::Pss => (modal.pss_op.label(), modal.pss_value.as_str()),
        };

        let title = row.title();
        let value_display = if value.is_empty() { "-" } else { value };
        let op_display = if op.is_empty() { "" } else { op };
        let line = if row.is_metric() {
            format!("{:<8} {:<2} {}", title, op_display, value_display)
        } else {
            format!("{:<8} {}", title, value_display)
        };

        Row::new(vec![Cell::from(line)]).style(style)
    });

    let mut title = format!(
        "Filter ({})  j/k:move  Enter:edit  h/l:op  Esc:close",
        if modal.editing { "edit" } else { "nav" }
    );
    if let Some(err) = modal.error.as_deref() {
        title.push_str(&format!("  error: {err}"));
    } else if selected_row.is_metric() {
        title.push_str("  units: B/KB/MB/GB (1024-base), cpu: % optional");
    }

    let table = Table::new(rows, [Constraint::Min(52)]).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().bg(COLUMN_PICKER_BACKGROUND))
            .border_style(Style::default().fg(COLUMN_PICKER_BORDER))
            .title_style(
                Style::default()
                    .fg(COLUMN_PICKER_BORDER)
                    .bg(COLUMN_PICKER_BACKGROUND)
                    .add_modifier(Modifier::BOLD),
            ),
    );

    frame.render_widget(table, area);
}

fn render_sort_picker(frame: &mut Frame<'_>, app: &AppState) {
    let area = centered_rect(frame.area(), 36, 12);
    frame.render_widget(Clear, area);
    let sort_state = app.sort_state();
    let sort_keys = app.sort_picker_keys();

    let rows = sort_keys.iter().enumerate().map(|(index, sort_key)| {
        let selected = index == app.sort_picker_index();
        let active = *sort_key == sort_state.key;
        let active_mark = if active { ">" } else { " " };
        let direction = if active {
            sort_state.direction
        } else {
            sort_key.default_direction()
        };
        let label = format!(
            "{active_mark} {:<8} {}",
            sort_key.title(),
            direction.as_ref()
        );
        let style = if selected {
            Style::default()
                .bg(COLUMN_PICKER_SELECTED_BACKGROUND)
                .fg(Color::White)
        } else if active {
            Style::default()
                .bg(COLUMN_PICKER_BACKGROUND)
                .fg(Color::Yellow)
        } else {
            Style::default().bg(COLUMN_PICKER_BACKGROUND)
        };
        Row::new(vec![Cell::from(label)]).style(style)
    });

    let table = Table::new(rows, [Constraint::Min(28)])
        .block(
            Block::default()
                .title("Sort  j/k:select  Enter/Space:apply  Esc/s:close")
                .borders(Borders::ALL)
                .style(Style::default().bg(COLUMN_PICKER_BACKGROUND))
                .border_style(Style::default().fg(COLUMN_PICKER_BORDER))
                .title_style(
                    Style::default()
                        .fg(COLUMN_PICKER_BORDER)
                        .bg(COLUMN_PICKER_BACKGROUND)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .column_spacing(0);

    frame.render_widget(table, area);
}

fn process_table_cell(
    column: ProcessColumn,
    row: &crate::snapshot::ProcessRow,
    name_cell: &str,
    app: &AppState,
) -> Cell<'static> {
    match column {
        ProcessColumn::Pid => Cell::from(row.pid.to_string()),
        ProcessColumn::Ppid => Cell::from(row.ppid.to_string()),
        ProcessColumn::Owner => Cell::from(app.owner_name(row.owner_uid)),
        ProcessColumn::Thread => Cell::from(row.threads.to_string()),
        ProcessColumn::Name => Cell::from(name_cell.to_owned()),
        ProcessColumn::Command => Cell::from(row.command.clone()),
        ProcessColumn::Rss => Cell::from(format_bytes(row.rss_bytes)),
        ProcessColumn::Uss => option_cell(row.uss_bytes),
        ProcessColumn::Pss => option_cell(row.pss_bytes),
        ProcessColumn::Swap => Cell::from(format_bytes(row.swap_bytes)),
        ProcessColumn::Cpu => Cell::from(format_percent(row.cpu_percent)),
    }
}

fn render_column_picker(frame: &mut Frame<'_>, app: &AppState) {
    let area = centered_rect(frame.area(), 36, 15);
    frame.render_widget(Clear, area);
    let visible_columns = app.visible_columns();
    let rows = app
        .column_picker_columns()
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let selected = index == app.column_picker_index();
            let enabled = visible_columns.contains(column);
            let toggle_mark = if enabled { "[x]" } else { "[ ]" };
            let mut label = format!("{toggle_mark} {}", column.title());
            if !column.is_toggleable() {
                label.push_str(" (fixed)");
            }
            let style = if selected {
                Style::default()
                    .bg(COLUMN_PICKER_SELECTED_BACKGROUND)
                    .fg(Color::White)
            } else if enabled {
                Style::default().bg(COLUMN_PICKER_BACKGROUND)
            } else {
                Style::default()
                    .bg(COLUMN_PICKER_BACKGROUND)
                    .fg(Color::DarkGray)
            };
            Row::new(vec![Cell::from(label)]).style(style)
        });

    let table = Table::new(rows, [Constraint::Min(28)])
        .block(
            Block::default()
                .title("Columns  j/k:select  Shift+Up/Down or J/K:move  Enter/Space:toggle  Esc/v:close")
                .borders(Borders::ALL)
                .style(Style::default().bg(COLUMN_PICKER_BACKGROUND))
                .border_style(Style::default().fg(COLUMN_PICKER_BORDER))
                .title_style(
                    Style::default()
                        .fg(COLUMN_PICKER_BORDER)
                        .bg(COLUMN_PICKER_BACKGROUND)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .column_spacing(0);

    frame.render_widget(table, area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let popup_width = width.min(area.width.saturating_sub(2)).max(1);
    let popup_height = height.min(area.height.saturating_sub(2)).max(1);
    let x = area.x + area.width.saturating_sub(popup_width) / 2;
    let y = area.y + area.height.saturating_sub(popup_height) / 2;
    Rect::new(x, y, popup_width, popup_height)
}

/// Styles an optional byte value for display in the process table.
/// If the value is `Some`, it's formatted as bytes with default styling,
/// but if it's `None`, it shows a placeholder with dimmed styling to indicate missing data.
fn option_cell(value: Option<u64>) -> Cell<'static> {
    let style = if value.is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Cell::from(format_option_bytes(value)).style(style)
}

fn format_tree_name(entry: &crate::app::TreeRow, name: &str) -> String {
    let mut prefix = String::new();
    for has_next in entry.ancestor_has_next_sibling.iter() {
        prefix.push_str(if *has_next { "│  " } else { "   " });
    }

    if entry.depth > 0 {
        prefix.push_str(if entry.is_last_sibling {
            "└─"
        } else {
            "├─"
        });
    }

    if entry.has_children {
        prefix.push_str(if entry.expanded { "[-] " } else { "[+] " });
    } else if entry.depth > 0 {
        prefix.push(' ');
    }

    prefix.push_str(name);
    prefix
}

#[cfg(test)]
mod tests {
    use super::{
        COLUMN_PICKER_BACKGROUND, centered_rect, chart_y_upper_bound, format_tree_name, render,
    };
    use crate::{app::TreeRow, collector::ProcfsCollector};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn buffer_text(buffer: &Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    #[test]
    fn chart_upper_bound_uses_total_when_non_zero() {
        assert_eq!(chart_y_upper_bound(1024), 1024);
    }

    #[test]
    fn chart_upper_bound_falls_back_to_one_for_zero() {
        assert_eq!(chart_y_upper_bound(0), 1);
    }

    #[test]
    fn format_tree_name_renders_root_toggle_without_leading_padding() {
        let root = TreeRow {
            process_index: 0,
            depth: 0,
            has_children: true,
            expanded: false,
            parent_index: None,
            is_last_sibling: false,
            ancestor_has_next_sibling: Vec::new(),
        };

        assert_eq!(format_tree_name(&root, "init"), "[+] init");
    }

    #[test]
    fn format_tree_name_uses_branch_markers() {
        let parent = TreeRow {
            process_index: 0,
            depth: 1,
            has_children: true,
            expanded: false,
            parent_index: Some(0),
            is_last_sibling: false,
            ancestor_has_next_sibling: vec![true],
        };
        let leaf = TreeRow {
            process_index: 1,
            depth: 2,
            has_children: false,
            expanded: false,
            parent_index: Some(0),
            is_last_sibling: true,
            ancestor_has_next_sibling: vec![true, false],
        };

        assert_eq!(format_tree_name(&parent, "bash"), "│  ├─[+] bash");
        assert_eq!(format_tree_name(&leaf, "worker"), "│     └─ worker");
    }

    #[test]
    fn centered_rect_stays_within_frame() {
        assert_eq!(
            centered_rect(Rect::new(0, 0, 20, 8), 36, 15),
            Rect::new(1, 1, 18, 6)
        );
    }

    #[test]
    fn column_picker_clears_background_from_selected_row() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = crate::app::AppState::new(ProcfsCollector::new());
        app.handle_key(key(KeyCode::Char('v')));

        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        let buffer = terminal.backend().buffer();
        let area = centered_rect(Rect::new(0, 0, 80, 24), 36, 15);
        let sample = buffer.cell((area.x + 2, area.y + 2)).unwrap();

        assert_eq!(sample.style().bg, Some(COLUMN_PICKER_BACKGROUND));
    }

    #[test]
    fn paused_state_is_rendered() {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = crate::app::AppState::new(ProcfsCollector::new());
        app.handle_key(key(KeyCode::Char('p')));

        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("status paused"));
        assert!(text.contains("p:resume"));
    }
}
