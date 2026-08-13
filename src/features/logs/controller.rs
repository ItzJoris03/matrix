use super::model::LogsModel;
use crate::common::strip_ansi;
use crate::engine::{ProcessManager, ProcessStatus};
use crate::url::{open_in_browser, url_at_column};
use arboard::Clipboard;
use crossterm::event::{KeyCode, MouseEvent, MouseEventKind};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum LogSelectionItem {
    Category(String),
    Source(usize), // Index in the active_sources list
}

pub enum LogAction {
    None,
    Message(String),
}

pub struct LogsController;

impl LogsController {
    pub async fn handle_key(
        key: KeyCode,
        model: &mut LogsModel,
        manager: &ProcessManager,
        clipboard: &mut Option<Clipboard>,
    ) -> LogAction {
        let items = Self::get_all_items(manager);
        let total_items = items.len();
        if total_items == 0 {
            return LogAction::None;
        }

        match key {
            KeyCode::Down | KeyCode::Char('j') => {
                let mut next_idx = (model.log_sidebar_index + 1) % total_items;
                while let Some(LogSelectionItem::Category(_)) = items.get(next_idx) {
                    next_idx = (next_idx + 1) % total_items;
                    if next_idx == model.log_sidebar_index {
                        break;
                    }
                }
                model.log_sidebar_index = next_idx;
                model.log_scroll = 0;
                model.clear_selection();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let mut next_idx = (model.log_sidebar_index + total_items - 1) % total_items;
                while let Some(LogSelectionItem::Category(_)) = items.get(next_idx) {
                    next_idx = (next_idx + total_items - 1) % total_items;
                    if next_idx == model.log_sidebar_index {
                        break;
                    }
                }
                model.log_sidebar_index = next_idx;
                model.log_scroll = 0;
                model.clear_selection();
            }
            KeyCode::Char('r') => {
                let active_sources = Self::get_active_sources(manager);
                if let Some(LogSelectionItem::Source(active_idx)) =
                    items.get(model.log_sidebar_index)
                {
                    if let Some((config, _)) = active_sources.get(*active_idx) {
                        let _ = manager.stop(&config.id).await;
                        if let Err(e) = manager.start(&config.id) {
                            return LogAction::Message(format!(
                                "Failed to restart {}: {}",
                                config.id, e
                            ));
                        }
                        return LogAction::Message(format!("Restarted {}", config.id));
                    }
                }
            }
            KeyCode::Char('c') => {
                return Self::copy_selection(model, manager, clipboard);
            }
            KeyCode::Char('h') => {
                let active_sources = Self::get_active_sources(manager);
                if let Some(LogSelectionItem::Source(active_idx)) =
                    items.get(model.log_sidebar_index)
                {
                    if let Some((config, _)) = active_sources.get(*active_idx) {
                        let project_id = if config.id.starts_with("engine:") {
                            &config.id[7..]
                        } else {
                            &config.id
                        };
                        match manager.toggle_host_mode(project_id) {
                            Ok(new_mode) => {
                                let label = if new_mode { "ENABLED" } else { "DISABLED" };

                                // Restart the process so the bind address takes effect.
                                // Vite needs --host 0.0.0.0 to listen on all interfaces;
                                // the firewall rule alone can't redirect traffic to a 127.0.0.1-only socket.
                                let restart_id = config.id.clone();

                                let _ = manager.stop(&restart_id).await;
                                if let Err(e) = manager.start(&restart_id) {
                                    return LogAction::Message(format!(
                                        "Host {} but restart failed: {}",
                                        label, e
                                    ));
                                }
                                return LogAction::Message(format!(
                                    "Host mode {} for {} (restarting…)",
                                    label,
                                    config.get_name()
                                ));
                            }
                            Err(e) => {
                                return LogAction::Message(format!(
                                    "Host mode not enabled for {}: {}",
                                    config.get_name(),
                                    e
                                ));
                            }
                        }
                    }
                }
            }
            KeyCode::Char('p') => {
                let active_sources = Self::get_active_sources(manager);
                if let Some(LogSelectionItem::Source(active_idx)) =
                    items.get(model.log_sidebar_index)
                {
                    if let Some((config, _)) = active_sources.get(*active_idx) {
                        let project_id = if config.id.starts_with("engine:") {
                            &config.id[7..]
                        } else {
                            &config.id
                        };
                        let new_mode = manager.toggle_prod_mode(project_id);
                        let label = if new_mode { "PROD" } else { "DEV" };

                        // Restart the process so the command change takes effect.
                        let restart_id = config.id.clone();

                        let _ = manager.stop(&restart_id).await;
                        if let Err(e) = manager.start(&restart_id) {
                            return LogAction::Message(format!(
                                "Mode {} but restart failed: {}",
                                label, e
                            ));
                        }
                        return LogAction::Message(format!(
                            "Switched to {} mode for {} (restarting…)",
                            label,
                            config.get_name()
                        ));
                    }
                }
            }
            KeyCode::Char('o') => {
                return Self::open_url_on_selected_line(model, manager);
            }
            KeyCode::PageDown => {
                let mut next_idx = model.log_sidebar_index;
                for _ in 0..5 {
                    next_idx = (next_idx + 1) % total_items;
                    while let Some(LogSelectionItem::Category(_)) = items.get(next_idx) {
                        next_idx = (next_idx + 1) % total_items;
                        if next_idx == model.log_sidebar_index {
                            break;
                        }
                    }
                }
                model.log_sidebar_index = next_idx;
                model.log_scroll = 0;
                model.clear_selection();
            }
            KeyCode::PageUp => {
                let mut next_idx = model.log_sidebar_index;
                for _ in 0..5 {
                    next_idx = (next_idx + total_items - 1) % total_items;
                    while let Some(LogSelectionItem::Category(_)) = items.get(next_idx) {
                        next_idx = (next_idx + total_items - 1) % total_items;
                        if next_idx == model.log_sidebar_index {
                            break;
                        }
                    }
                }
                model.log_sidebar_index = next_idx;
                model.log_scroll = 0;
                model.clear_selection();
            }
            KeyCode::End => {
                let max_scroll = model
                    .last_logs_len
                    .saturating_sub(model.last_log_area.height as usize)
                    as u16;
                model.scroll_to_bottom(max_scroll);
            }
            KeyCode::Home => {
                model.scroll_to_top();
            }
            _ => {}
        }
        LogAction::None
    }

    pub fn handle_mouse(mouse: MouseEvent, model: &mut LogsModel, _manager: &ProcessManager) {
        let area = model.last_log_area;
        let max_scroll = model.last_logs_len.saturating_sub(area.height as usize) as u16;

        match mouse.kind {
            MouseEventKind::ScrollDown => {
                model.scroll_down(max_scroll);
            }
            MouseEventKind::ScrollUp => {
                model.scroll_up();
            }
            MouseEventKind::Down(_) => {
                if mouse.column >= area.x
                    && mouse.row >= area.y
                    && mouse.row < (area.y + area.height)
                {
                    let line_in_view = (mouse.row - area.y) as usize;
                    let line_index = model.last_rendered_scroll + line_in_view;

                    if line_index < model.last_logs_len {
                        let col_in_line = (mouse.column - area.x) as usize;
                        let urls_for_line = model
                            .cache_urls
                            .iter()
                            .find(|(idx, _)| *idx == line_index)
                            .map(|(_, urls)| urls.as_slice())
                            .unwrap_or(&[]);

                        if let Some(url_match) = url_at_column(urls_for_line, col_in_line) {
                            let _ = open_in_browser(&url_match.url);
                            return;
                        }
                    }

                    // Normal selection behavior
                    model.selection_start = Some(line_index);
                    model.selection_end = Some(line_index);
                } else {
                    model.clear_selection();
                }
            }
            MouseEventKind::Drag(_) if model.selection_start.is_some() => {
                if mouse.row >= area.y && mouse.row < (area.y + area.height) {
                    let line_in_view = (mouse.row - area.y) as usize;
                    let line_index = model.last_rendered_scroll + line_in_view;

                    if line_index < model.last_logs_len {
                        model.selection_end = Some(line_index);
                    }
                } else if mouse.row < area.y {
                    model.selection_end = Some(model.last_rendered_scroll);
                } else {
                    let max_visible =
                        model.last_rendered_scroll + (area.height as usize).saturating_sub(1);
                    model.selection_end =
                        Some(max_visible.min(model.last_logs_len.saturating_sub(1)));
                }
            }
            _ => {}
        }
    }

    fn open_url_on_selected_line(model: &LogsModel, manager: &ProcessManager) -> LogAction {
        let active_sources = Self::get_active_sources(manager);
        let items = Self::get_all_items(manager);

        if let Some(LogSelectionItem::Source(active_idx)) = items.get(model.log_sidebar_index) {
            if let Some((config, _)) = active_sources.get(*active_idx) {
                let line_index = if let (Some(start), Some(end)) =
                    (model.selection_start, model.selection_end)
                {
                    if start < end {
                        start
                    } else {
                        end
                    }
                } else {
                    model.last_rendered_scroll
                };

                let urls_for_line = model
                    .cache_urls
                    .iter()
                    .find(|(idx, _)| *idx == line_index)
                    .map(|(_, urls)| urls.as_slice())
                    .unwrap_or(&[]);

                if let Some(first_url) = urls_for_line.first() {
                    return match open_in_browser(&first_url.url) {
                        Ok(_) => LogAction::Message(format!("Opened: {}", first_url.url)),
                        Err(e) => LogAction::Message(format!("Failed to open URL: {}", e)),
                    };
                }

                // Fallback: search the raw log line for a URL
                let logs = manager.get_logs(&config.id);
                if let Some(&orig_idx) = model.last_line_map.get(line_index) {
                    if let Some(raw_line) = logs.get(orig_idx) {
                        let clean = strip_ansi(raw_line);
                        let urls = crate::url::find_urls(&clean);
                        if let Some(first) = urls.first() {
                            return match open_in_browser(&first.url) {
                                Ok(_) => LogAction::Message(format!("Opened: {}", first.url)),
                                Err(e) => LogAction::Message(format!("Failed to open URL: {}", e)),
                            };
                        }
                    }
                }

                return LogAction::Message("No URL found on selected line".to_string());
            }
        }
        LogAction::None
    }

    fn copy_selection(
        model: &mut LogsModel,
        manager: &ProcessManager,
        clipboard: &mut Option<Clipboard>,
    ) -> LogAction {
        if let (Some(start), Some(end)) = (model.selection_start, model.selection_end) {
            let active_sources = Self::get_active_sources(manager);
            let items = Self::get_all_items(manager);

            if let Some(LogSelectionItem::Source(active_idx)) = items.get(model.log_sidebar_index) {
                if let Some((config, _)) = active_sources.get(*active_idx) {
                    let logs = manager.get_logs(&config.id);
                    let (s, e) = if start < end {
                        (start, end)
                    } else {
                        (end, start)
                    };

                    let mut selected_text = String::new();
                    let mut last_added_orig_idx = None;

                    for i in s..=e {
                        if let Some(&orig_idx) = model.last_line_map.get(i) {
                            if Some(orig_idx) != last_added_orig_idx {
                                if let Some(line) = logs.get(orig_idx) {
                                    selected_text.push_str(&strip_ansi(line));
                                    selected_text.push('\n');
                                }
                                last_added_orig_idx = Some(orig_idx);
                            }
                        }
                    }

                    if !selected_text.is_empty() {
                        if let Some(cb) = clipboard {
                            return match cb.set_text(selected_text) {
                                Ok(_) => LogAction::Message(
                                    "Selected logs copied to clipboard".to_string(),
                                ),
                                Err(e) => LogAction::Message(format!(
                                    "Clipboard copy failed: {} (is xclip/xsel installed?)",
                                    e
                                )),
                            };
                        }
                        return LogAction::Message(
                            "Clipboard copy failed: no clipboard backend available".to_string(),
                        );
                    }
                }
            }
        }
        LogAction::None
    }

    pub fn get_active_sources(
        manager: &ProcessManager,
    ) -> Vec<(crate::config::Project, ProcessStatus)> {
        manager
            .get_statuses()
            .into_iter()
            .filter(|(_, status)| {
                matches!(
                    status,
                    ProcessStatus::Running(_) | ProcessStatus::Crashed(_)
                )
            })
            .collect()
    }

    pub fn get_all_items(manager: &ProcessManager) -> Vec<LogSelectionItem> {
        let active_sources = Self::get_active_sources(manager);
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();

        for (idx, (project, _)) in active_sources.into_iter().enumerate() {
            let category = project
                .category
                .clone()
                .unwrap_or_else(|| "Other".to_string());
            groups.entry(category).or_default().push(idx);
        }

        let mut all_items = Vec::new();
        for (category, indices) in groups {
            all_items.push(LogSelectionItem::Category(category));
            for idx in indices {
                all_items.push(LogSelectionItem::Source(idx));
            }
        }
        all_items
    }
}
