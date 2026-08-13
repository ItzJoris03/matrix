use super::controller::{ProjectsController, SelectionItem};
use super::model::ProjectsModel;
use crate::common::theme::*;
use crate::engine::{ProcessManager, ProcessStatus};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    model: &ProjectsModel,
    manager: &ProcessManager,
    is_command_mode: bool,
) {
    let statuses = manager.get_statuses();
    if statuses.is_empty() {
        let empty = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .title(Span::styled(
                " Projects ",
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ));
        frame.render_widget(empty, area);
        return;
    }

    let items = ProjectsController::get_all_items(model, manager);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == model.selected_index && !is_command_mode;

            match item {
                SelectionItem::Group {
                    id,
                    name,
                    expanded,
                    running,
                } => {
                    let style = if is_selected {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(PURPLE).add_modifier(Modifier::BOLD)
                    };
                    let expand_indicator = if *expanded { "▾" } else { "▸" };
                    let dot = if *running { "●" } else { "○" };
                    let dot_color = if *running { GREEN } else { GRAY };
                    let caret = if is_selected { "▌" } else { " " };
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            caret,
                            Style::default().fg(if is_selected { PURPLE } else { GRAY_DIM }),
                        ),
                        Span::styled(
                            format!(" {} ", expand_indicator),
                            Style::default().fg(TEXT_DIM),
                        ),
                        Span::styled(dot, Style::default().fg(dot_color)),
                        Span::raw(" "),
                        Span::styled(name.clone(), style),
                        Span::styled(
                            format!("  [{}]", id),
                            Style::default().fg(if is_selected { Color::White } else { TEXT_DIM }),
                        ),
                    ]))
                }
                SelectionItem::StandaloneHeader => ListItem::new(Line::from(vec![
                    Span::styled("── ", Style::default().fg(GRAY)),
                    Span::styled(
                        "Standalone",
                        Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" ──", Style::default().fg(GRAY)),
                ])),
                SelectionItem::Project {
                    original_index,
                    group_id,
                    is_infra,
                } => {
                    let (config, status) = &statuses[*original_index];

                    let (chip_text, chip_color) = match status {
                        ProcessStatus::Stopped => (" OFF ", GRAY),
                        ProcessStatus::Starting => (" START ", YELLOW),
                        ProcessStatus::Running(_) => (" RUN ", GREEN),
                        ProcessStatus::Crashed(_e) => (" FAIL ", RED),
                    };

                    let sel_bg = if is_selected { PURPLE } else { GRAY_DIM };
                    let base_fg = if is_selected { Color::White } else { TEXT };

                    let indent = if group_id.is_some() { "    " } else { "  " };
                    let prefix = if is_selected { "▌" } else { " " };

                    let display_name = if config.id.starts_with("engine:") {
                        format!("{} (engine)", config.get_name())
                    } else if *is_infra {
                        format!("{} [infra]", config.get_name())
                    } else {
                        config.get_name()
                    };

                    let id_color = if is_selected {
                        Color::White
                    } else if *is_infra {
                        MAGENTA
                    } else if config.id.starts_with("engine:") {
                        TEXT_DIM
                    } else {
                        PURPLE
                    };

                    let port_display = if is_selected && model.editing_port.is_some() {
                        match &model.editing_port {
                            Some(p) => format!(":{}", p),
                            None => "".to_string(),
                        }
                    } else {
                        match config.port {
                            Some(p) => format!(":{}", p),
                            None => "".to_string(),
                        }
                    };
                    let port_style = if is_selected && model.editing_port.is_some() {
                        Style::default().fg(YELLOW).add_modifier(Modifier::BOLD)
                    } else if is_selected {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(TEXT_DIM)
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(sel_bg)),
                        Span::raw(indent),
                        Span::styled(
                            chip_text,
                            Style::default()
                                .fg(Color::Black)
                                .bg(chip_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(format!("{:<22}", config.id), Style::default().fg(id_color)),
                        Span::styled(display_name, Style::default().fg(base_fg)),
                        Span::styled(format!("  {}", port_display), port_style),
                    ]))
                }
            }
        })
        .collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .title(Span::styled(
                " Projects ",
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            )),
    );
    frame.render_widget(list, area);
}
