use crate::app::AppState;
use crate::format::{format_bytes, format_option_bytes, format_percent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Borders, Cell, Chart, Dataset, Paragraph, Row, Table};


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
}

fn render_history_chart(frame: &mut Frame<'_>, area: Rect, app: &AppState, rss: bool) {
    let (title, points, latest_value, color) = if rss {
        (
            "RSS History",
            app.rss_chart_points(),
            app.snapshot.system.total_process_rss,
            Color::LightGreen,
        )
    } else {
        (
            "Swap History",
            app.swap_chart_points(),
            app.snapshot.system.total_process_swap,
            Color::LightBlue,
        )
    };

    let x_max = points.last().map(|(x, _)| *x).unwrap_or(180.0).max(1.0);
    let y_max = points.iter().map(|(_, y)| *y).fold(1.0_f64, f64::max);

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
        .y_axis(Axis::default().bounds([0.0, y_max]).labels([
            Line::from("0"),
            Line::from(format_bytes(latest_value.max(1))),
        ]));

    frame.render_widget(chart, area);
}

fn render_summary(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let system = &app.snapshot.system;
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
                "count {} / agg-rss {} / agg-swap {} / sort {:?} / selected {} / age {}s",
                system.process_count,
                format_bytes(system.total_process_rss),
                format_bytes(system.total_process_swap),
                app.sort_key,
                app.selected_pid()
                    .map_or("-".to_string(), |pid| pid.to_string()),
                app.snapshot.captured_at.elapsed().as_secs(),
            )),
        ]),
    ];

    if let Some(error) = &app.last_error {
        lines.push(Line::from(vec![
            Span::styled(
                "Last Error ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(error.clone()),
        ]));
    }

    let paragraph =
        Paragraph::new(lines).block(Block::default().title("System").borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn render_process_table(frame: &mut Frame<'_>, area: Rect, app: &mut AppState) {
    app.set_viewport_rows(area.height.saturating_sub(3) as usize);
    let header = Row::new(vec![
        "PID", "PPID", "OWNER", "THREAD", "NAME", "COMMAND", "RSS", "USS", "PSS", "SWAP", "CPU",
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows = app
        .visible_processes()
        .iter()
        .enumerate()
        .map(|(visible_idx, row)| {
            let style = if app.scroll_offset + visible_idx == app.selected {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(row.pid.to_string()),
                Cell::from(row.ppid.to_string()),
                Cell::from(app.owner_name(row.owner_uid)),
                Cell::from(row.threads.to_string()),
                Cell::from(row.name.clone()),
                Cell::from(row.command.clone()),
                Cell::from(format_bytes(row.rss_bytes)),
                Cell::from(format_option_bytes(row.uss_bytes)),
                Cell::from(format_option_bytes(row.pss_bytes)),
                Cell::from(format_bytes(row.visible_swap_bytes())),
                Cell::from(format_percent(row.cpu_percent)),
            ])
            .style(style)
        });

    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Min(24),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Processes  q:quit  arrows:move  PgUp/PgDn:page  r/s/p/c:sort")
            .borders(Borders::ALL),
    )
    .column_spacing(1);

    frame.render_widget(table, area);
}
