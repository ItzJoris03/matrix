use super::controller::{LogSelectionItem, LogsController};
use super::model::LogsModel;
use crate::common::get_local_ip;
use crate::common::strip_ansi;
use crate::common::theme::*;
use crate::engine::ProcessManager;
use crate::engine::ProcessStatus;
use crate::url::UrlMatch;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

/// Style for URL spans — bright blue + underlined so they stand out.
fn url_style() -> Style {
    Style::default()
        .fg(Color::Rgb(80, 160, 255))
        .add_modifier(Modifier::UNDERLINED)
}

pub fn render(frame: &mut Frame, area: Rect, model: &mut LogsModel, manager: &ProcessManager) {
    let log_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25), // Log sidebar
            Constraint::Percentage(75), // Log content
        ])
        .split(area);

    let active_sources = LogsController::get_active_sources(manager);
    let items = LogsController::get_all_items(manager);

    // Internal Log Sidebar
    let sidebar_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == model.log_sidebar_index;
            match item {
                LogSelectionItem::Category(name) => {
                    let style = if is_selected {
                        Style::default()
                            .bg(PURPLE)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    };
                    let content = if is_selected {
                        format!(">> {} <<", name.to_uppercase())
                    } else {
                        format!("── {} ──", name.to_uppercase())
                    };
                    ListItem::new(content).style(style)
                }
                LogSelectionItem::Source(idx) => {
                    let (config, status) = &active_sources[*idx];
                    let (dot, dot_color) = match status {
                        ProcessStatus::Running(_) => ("●", Color::Green),
                        ProcessStatus::Crashed(_) => ("●", Color::Red),
                        ProcessStatus::Starting => ("●", Color::Yellow),
                        ProcessStatus::Stopped => ("○", Color::DarkGray),
                    };
                    let style = if is_selected {
                        Style::default()
                            .bg(PURPLE)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Green)
                    };
                    let prefix = if is_selected { ">> " } else { "   " };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{} ", dot), Style::default().fg(dot_color)),
                        Span::styled(format!("{}{}", prefix, config.id), style),
                    ]))
                }
            }
        })
        .collect();

    let sidebar = List::new(sidebar_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(GRAY))
            .title(" Active Sources "),
    );
    frame.render_widget(sidebar, log_chunks[0]);

    // Calculate Inner Content Area
    let log_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GRAY));

    let inner_log_area = log_block.inner(log_chunks[1]);
    model.last_log_area = inner_log_area;

    // Log Content
    let current_item = items.get(model.log_sidebar_index);
    if let Some(LogSelectionItem::Source(active_idx)) = current_item {
        let (config, status) = &active_sources[*active_idx];
        let raw_logs = manager.get_logs(&config.id);

        let width = inner_log_area.width as usize;
        let logs_len = raw_logs.len();
        let rev = manager.get_log_rev(&config.id);

        let cache_valid = {
            let proc_cache = model.processor.snapshot();
            proc_cache.is_valid(&config.id, width, logs_len, rev)
        };

        if !cache_valid {
            // Trigger background processing (non-blocking). Only enqueue when the
            // request actually changes — otherwise the log revision bumping per line
            // would flood the processor channel every frame and the worker would
            // fall behind. The worker thread also coalesces queued requests, so a
            // burst of N new lines becomes a single recompute.
            if model.req_rev != rev || model.req_project_id != config.id || model.req_width != width
            {
                model.req_rev = rev;
                model.req_project_id = config.id.clone();
                model.req_width = width;
                model
                    .processor
                    .process(config.id.clone(), width, raw_logs.clone(), rev);
            }

            // Fall back to the stale inline cache if available. This is what kills
            // the flicker: while the background worker is recomputing, we keep showing
            // the previous frame's wrapped lines (which are visually identical to what
            // the user was already looking at). The new line(s) appear on the next
            // frame once the worker catches up. We do NOT gate this on the log
            // revision — a new line bumps `rev`, but the old lines are still valid to
            // display, so gating on `rev` here was the root cause of the
            // "Processing logs..." flash on every incoming line.
            let cache_hit = model.cache_project_id == config.id
                && model.cache_width == width
                && model.cache_logs_len > 0;

            if cache_hit {
                // Use existing cached rendered lines — no heavy recompute this frame.
                // Scroll math uses the CURRENT log length so auto-scroll pinning and
                // selection offsets stay correct even before the worker catches up.
                let total_screen_lines = logs_len;
                let max_scroll =
                    total_screen_lines.saturating_sub(inner_log_area.height as usize) as u16;
                let scroll_pos = if model.auto_scroll_logs {
                    max_scroll
                } else {
                    model.log_scroll.min(max_scroll)
                };
                model.log_scroll = scroll_pos;
                model.last_rendered_scroll = scroll_pos as usize;
                model.last_logs_len = logs_len;

                let (sel_start, sel_end) =
                    if let (Some(s), Some(e)) = (model.selection_start, model.selection_end) {
                        if s < e {
                            (s, e)
                        } else {
                            (e, s)
                        }
                    } else {
                        (usize::MAX, usize::MAX)
                    };

                let final_lines: Vec<Line> = render_cached_lines(
                    &model.cache_wrapped,
                    &model.cache_urls,
                    &model.cache_project_id,
                    &raw_logs,
                    sel_start,
                    sel_end,
                );

                let state_word = match status {
                    ProcessStatus::Running(_) => "RUNNING",
                    ProcessStatus::Crashed(_) => "CRASHED",
                    ProcessStatus::Starting => "STARTING",
                    ProcessStatus::Stopped => "STOPPED",
                };
                let title_text = format!(" Logs: {} [{}] ", config.get_name(), state_word);
                let host_mode_id = if config.id.starts_with("engine:") {
                    &config.id[7..]
                } else {
                    &config.id
                };
                let title = if manager.is_host_mode(host_mode_id) {
                    build_host_title(&title_text, config, manager)
                } else {
                    Line::from(Span::styled(title_text, Style::default().fg(PURPLE)))
                };
                let paragraph = Paragraph::new(final_lines)
                    .block(log_block.title(title))
                    .scroll((scroll_pos, 0));
                frame.render_widget(paragraph, log_chunks[1]);
            } else {
                // No cache yet — first frame ever for this project. Render the raw
                // logs inline (cheap, no wrap/URL pass) so we never show a
                // "Processing logs..." flash. The worker still fills the cache for
                // subsequent frames.
                let total_screen_lines = logs_len;
                let max_scroll =
                    total_screen_lines.saturating_sub(inner_log_area.height as usize) as u16;
                let scroll_pos = if model.auto_scroll_logs {
                    max_scroll
                } else {
                    model.log_scroll.min(max_scroll)
                };
                let (sel_start, sel_end) =
                    if let (Some(s), Some(e)) = (model.selection_start, model.selection_end) {
                        if s < e {
                            (s, e)
                        } else {
                            (e, s)
                        }
                    } else {
                        (usize::MAX, usize::MAX)
                    };

                let final_lines: Vec<Line> = raw_logs
                    .iter()
                    .enumerate()
                    .map(|(i, raw)| {
                        let is_selected = i >= sel_start && i <= sel_end;
                        let content = strip_ansi(raw);
                        let style = if is_selected {
                            Style::default()
                                .fg(Color::White)
                                .bg(GRAY_DIM)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        Line::from(Span::styled(content, style))
                    })
                    .collect();

                model.log_scroll = scroll_pos;
                model.last_rendered_scroll = scroll_pos as usize;
                model.last_logs_len = logs_len;
                model.last_line_map = (0..logs_len).collect();
                model.cache_project_id = config.id.clone();
                model.cache_width = width;
                model.cache_logs_len = logs_len;

                let state_word = match status {
                    ProcessStatus::Running(_) => "RUNNING",
                    ProcessStatus::Crashed(_) => "CRASHED",
                    ProcessStatus::Starting => "STARTING",
                    ProcessStatus::Stopped => "STOPPED",
                };
                let title_text = format!(" Logs: {} [{}] ", config.get_name(), state_word);
                let title = Line::from(Span::styled(title_text, Style::default().fg(PURPLE)));
                let paragraph = Paragraph::new(final_lines)
                    .block(log_block.title(title))
                    .scroll((scroll_pos, 0));
                frame.render_widget(paragraph, log_chunks[1]);
            }
        } else {
            let proc_cache = model.processor.snapshot();
            model.last_logs_len = proc_cache.lines.len();
            model.last_line_map = proc_cache.line_map.clone();

            let total_screen_lines = proc_cache.lines.len();
            let max_scroll =
                total_screen_lines.saturating_sub(inner_log_area.height as usize) as u16;
            let scroll_pos = if model.auto_scroll_logs {
                max_scroll
            } else {
                model.log_scroll.min(max_scroll)
            };
            model.log_scroll = scroll_pos;
            model.last_rendered_scroll = scroll_pos as usize;

            let (sel_start, sel_end) =
                if let (Some(s), Some(e)) = (model.selection_start, model.selection_end) {
                    if s < e {
                        (s, e)
                    } else {
                        (e, s)
                    }
                } else {
                    (usize::MAX, usize::MAX)
                };

            let final_lines: Vec<Line> = proc_cache
                .lines
                .iter()
                .enumerate()
                .map(|(i, proc_line)| {
                    let is_selected = i >= sel_start && i <= sel_end;
                    if !proc_line.urls.is_empty() && !proc_line.content.is_empty() {
                        build_url_spans(proc_line, is_selected)
                    } else {
                        let style = if is_selected {
                            proc_line.style.bg(GRAY_DIM).add_modifier(Modifier::BOLD)
                        } else {
                            proc_line.style
                        };
                        Line::from(Span::styled(proc_line.content.clone(), style))
                    }
                })
                .collect();

            // Update inline cache with the processed data for fallback next time.
            model.cache_project_id = config.id.clone();
            model.cache_width = width;
            model.cache_logs_len = logs_len;
            model.cache_wrapped = proc_cache
                .lines
                .iter()
                .map(|l| (l.orig_idx, l.content.clone(), l.style))
                .collect();
            model.cache_urls = proc_cache
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| (i, l.urls.clone()))
                .collect();

            let title_text = format!(" Logs: {} ", config.get_name());
            let host_mode_id = if config.id.starts_with("engine:") {
                &config.id[7..]
            } else {
                &config.id
            };
            let title = if manager.is_host_mode(host_mode_id) {
                build_host_title(&title_text, config, manager)
            } else {
                Line::from(Span::styled(title_text, Style::default().fg(PURPLE)))
            };
            let paragraph = Paragraph::new(final_lines)
                .block(log_block.title(title))
                .scroll((scroll_pos, 0));
            frame.render_widget(paragraph, log_chunks[1]);
        }
    } else {
        model.last_logs_len = 0;
        model.last_rendered_scroll = 0;
        model.last_line_map.clear();
        model.cache_urls.clear();
        let title = if let Some(LogSelectionItem::Category(name)) = current_item {
            format!(" Category: {} ", name)
        } else {
            " Logs ".to_string()
        };
        frame.render_widget(log_block.title(title), log_chunks[1]);
    }
}

/// Render cached (stale) lines without re-doing word wrap or URL detection.
/// Used as fallback while the background processor is still working.
fn render_cached_lines<'a>(
    cache_wrapped: &'a [(usize, String, Style)],
    cache_urls: &'a [(usize, Vec<UrlMatch>)],
    _cache_project_id: &str,
    _raw_logs: &[String],
    sel_start: usize,
    sel_end: usize,
) -> Vec<Line<'a>> {
    cache_wrapped
        .iter()
        .enumerate()
        .map(|(i, (_, content, base_style))| {
            let is_selected = i >= sel_start && i <= sel_end;
            let urls_for_line = cache_urls
                .iter()
                .find(|(idx, _)| *idx == i)
                .map(|(_, urls)| urls.as_slice())
                .unwrap_or(&[]);

            if !urls_for_line.is_empty() && !content.is_empty() {
                build_url_spans_inner(content, base_style, urls_for_line, is_selected)
            } else {
                let style = if is_selected {
                    base_style.bg(GRAY_DIM).add_modifier(Modifier::BOLD)
                } else {
                    *base_style
                };
                Line::from(Span::styled(content.clone(), style))
            }
        })
        .collect()
}

/// Build styled spans for a processed line that has URLs.
fn build_url_spans<'a>(
    proc_line: &'a super::processor::ProcessedLine,
    is_selected: bool,
) -> Line<'a> {
    build_url_spans_inner(
        &proc_line.content,
        &proc_line.style,
        &proc_line.urls,
        is_selected,
    )
}

/// Internal helper: build URL-highlighted spans from raw parts.
fn build_url_spans_inner<'a>(
    content: &'a str,
    base_style: &Style,
    urls: &[UrlMatch],
    is_selected: bool,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut cursor = 0usize;

    for url_match in urls {
        if url_match.start > cursor {
            let before = &content[cursor..url_match.start.min(content.len())];
            if !before.is_empty() {
                let style = if is_selected {
                    base_style.bg(GRAY_DIM).add_modifier(Modifier::BOLD)
                } else {
                    *base_style
                };
                spans.push(Span::styled(before.to_string(), style));
            }
        }
        let url_end = url_match.end.min(content.len());
        let url_text = &content[url_match.start..url_end];
        let s = url_style();
        let url_style = if is_selected { s.bg(GRAY_DIM) } else { s };
        spans.push(Span::styled(url_text.to_string(), url_style));
        cursor = url_end;
    }

    if cursor < content.len() {
        let remaining = &content[cursor..];
        let style = if is_selected {
            base_style.bg(GRAY_DIM).add_modifier(Modifier::BOLD)
        } else {
            *base_style
        };
        spans.push(Span::styled(remaining.to_string(), style));
    }
    Line::from(spans)
}

/// Build the title line with host mode indicator and dev/prod badge.
fn build_host_title<'a>(
    title_text: &'a str,
    config: &crate::config::Project,
    manager: &ProcessManager,
) -> Line<'a> {
    let mut spans = Vec::new();
    spans.push(Span::styled(title_text, Style::default().fg(PURPLE)));

    let project_id = if config.id.starts_with("engine:") {
        &config.id[7..]
    } else {
        &config.id
    };
    let mode_label = if manager.is_prod_mode(project_id) {
        "PROD"
    } else {
        "DEV"
    };
    let mode_color = if manager.is_prod_mode(project_id) {
        Color::Magenta
    } else {
        Color::Cyan
    };
    spans.push(Span::styled(
        format!(" [{}] ", mode_label),
        Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
    ));

    spans.push(Span::styled(
        " [HOST] ",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    ));
    if let Some(ip) = get_local_ip() {
        let port = config.port.map(|p| format!(":{}", p)).unwrap_or_default();
        spans.push(Span::styled(
            format!(" http://{}{} ", ip, port),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}
