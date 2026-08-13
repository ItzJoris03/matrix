use super::model::EnvModel;
use crate::common::theme::*;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn render(frame: &mut Frame, area: Rect, model: &mut EnvModel) {
    let height = area.height as usize - 2; // -2 for borders

    if model.cursor_y < model.scroll_offset {
        model.scroll_offset = model.cursor_y;
    } else if model.cursor_y >= model.scroll_offset + height {
        model.scroll_offset = model.cursor_y - height + 1;
    }

    let visible_lines: Vec<Line> = model
        .lines
        .iter()
        .enumerate()
        .skip(model.scroll_offset)
        .take(height)
        .map(|(i, line)| {
            let line_num = Span::styled(
                format!("{:3} | ", i + 1),
                Style::default().fg(Color::DarkGray),
            );

            // Simple syntax highlighting
            let mut spans = vec![line_num];
            if line.starts_with('#') {
                spans.push(Span::styled(line, Style::default().fg(Color::Green)));
            } else if let Some(pos) = line.find('=') {
                let (key, val) = line.split_at(pos);
                spans.push(Span::styled(key, Style::default().fg(Color::Cyan)));
                spans.push(Span::styled("=", Style::default().fg(Color::White)));
                spans.push(Span::styled(
                    val.strip_prefix('=').unwrap_or(val),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                spans.push(Span::raw(line));
            }
            Line::from(spans)
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GRAY))
        .title(Span::styled(
            format!(" Editing .env: {} ", model.project_id),
            Style::default().fg(PURPLE),
        ));

    let inner_area = block.inner(area);
    let p = Paragraph::new(visible_lines).block(block);
    frame.render_widget(p, area);

    frame.set_cursor(
        inner_area.x + (model.cursor_x + 6) as u16, // +6 for line number prefix "123 | "
        inner_area.y + (model.cursor_y - model.scroll_offset) as u16,
    );
}
