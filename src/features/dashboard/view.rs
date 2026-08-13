use super::model::DashboardModel;
use crate::common::theme::*;
use crate::engine::{ProcessManager, ProcessStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, model: &DashboardModel, manager: &ProcessManager) {
    let statuses = manager.get_statuses();

    let total = statuses.len();
    let running = statuses
        .iter()
        .filter(|(_, s)| matches!(s, ProcessStatus::Running(_)))
        .count();
    let crashed = statuses
        .iter()
        .filter(|(_, s)| matches!(s, ProcessStatus::Crashed(_)))
        .count();
    let starting = statuses
        .iter()
        .filter(|(_, s)| matches!(s, ProcessStatus::Starting))
        .count();

    // ── Top summary bar ──
    let summary = Line::from(vec![
        Span::styled(
            " MATRIX ",
            Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} services", total),
            Style::default().fg(Color::Gray),
        ),
        Span::raw("   "),
        Span::styled("● ", Style::default().fg(Color::Green)),
        Span::styled(format!("{} up", running), Style::default().fg(Color::Green)),
        Span::raw("   "),
        Span::styled("● ", Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{} starting", starting),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("   "),
        Span::styled("● ", Style::default().fg(Color::Red)),
        Span::styled(
            format!("{} crashed", crashed),
            Style::default().fg(Color::Red),
        ),
    ]);

    // ── Services table (ALL services, grouped by category) ──
    let header = Row::new(vec![
        Cell::from(Span::styled(
            " SERVICE",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            " STATE",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            " PORT",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            " MODE",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )),
    ]);

    let mut rows: Vec<Row> = Vec::new();
    for (config, status) in statuses.iter() {
        let (dot, state_text, state_color) = match status {
            ProcessStatus::Stopped => ("○", "OFF", Color::DarkGray),
            ProcessStatus::Starting => ("●", "STARTING", Color::Yellow),
            ProcessStatus::Running(_) => ("●", "RUNNING", Color::Green),
            ProcessStatus::Crashed(_) => ("●", "CRASHED", Color::Red),
        };

        let port = config.port.map(|p| format!(":{}", p)).unwrap_or_default();
        let is_host = manager.is_host_mode(&config.id);
        let is_prod = manager.is_prod_mode(&config.id);
        let mode = if is_prod { "PROD" } else { "DEV" };
        let mode_color = if is_prod { Color::Magenta } else { Color::Cyan };
        let name = config.get_name();
        let host_badge = if is_host {
            Span::styled(" [H]", Style::default().fg(Color::Red))
        } else {
            Span::raw("")
        };

        rows.push(Row::new(vec![
            Cell::from(Span::raw(format!(" {} {}", dot, name)))
                .style(Style::default().fg(Color::White)),
            Cell::from(Span::styled(
                format!(" {}", state_text),
                Style::default()
                    .fg(state_color)
                    .add_modifier(Modifier::BOLD),
            )),
            Cell::from(Span::raw(port)),
            Cell::from(Line::from(vec![
                Span::styled(format!(" {}", mode), Style::default().fg(mode_color)),
                host_badge,
            ])),
        ]));
    }

    let widths = &[
        Constraint::Percentage(45),
        Constraint::Percentage(20),
        Constraint::Percentage(15),
        Constraint::Percentage(20),
    ];

    let table = Table::new(rows, *widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(GRAY))
                .title(Span::styled(" Services ", Style::default().fg(PURPLE))),
        )
        .highlight_style(Style::default());

    // ── Control / hints panel ──
    let hints = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            " Quick actions ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  [Enter] start/stop · [r] restart · [e] expand group"),
        Line::from("  [h] host mode (LAN) · [p] dev/prod"),
        Line::from("  [:] command mode · e.g. :restart my-service"),
        Line::from(""),
        Line::from(Span::styled(
            " CLI (from any shell) ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  matrix restart <id> · matrix status · matrix stop <id>"),
        Line::from(""),
        Line::from(Span::styled(
            " Logs view ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Shows ALL processes — pick any to inspect output,"),
        Line::from("  crashed/stopped included. [c] copy · [o] open URL."),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(GRAY))
            .title(Span::styled(" Control ", Style::default().fg(PURPLE))),
    );

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    frame.render_widget(summary, outer[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(outer[1]);

    frame.render_widget(table, body[0]);
    frame.render_widget(hints, body[1]);

    let _ = model;
}
