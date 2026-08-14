use super::App;
use crate::common::theme::*;
use crate::common::ToastTone;
use crate::features::{dashboard, env, logs, projects};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        if self.active_view == crate::common::ActiveView::EnvEditor {
            env::render(frame, frame.size(), &mut self.env_model);
            return;
        }

        // Expire stale toasts (5s TTL).
        self.expire_toasts(std::time::Duration::from_secs(5));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Content
                Constraint::Length(1), // Footer status bar
            ])
            .split(frame.size());

        self.render_header(frame, chunks[0]);
        self.render_body(frame, chunks[1]);

        // ── FOOTER status bar ──
        self.render_footer(frame, chunks[2]);

        // ── TOASTS (live notifications, bottom-right) ──
        if !self.toasts.is_empty() {
            self.render_toasts(frame);
        }

        // ── OVERLAY LAYER ── drawn after header/body/footer so each modal's
        // scrim dims the whole UI behind it and the panel reads as a raised
        // card. Order = stacking: palette, then detect, then update card.
        if self.is_command_mode {
            self.render_command_palette(frame);
        }
        if self.show_detect_modal {
            self.render_detect_modal(frame);
        }
        self.render_update_modal(frame);

        // ── FIRST-LAUNCH WELCOME (drawn last: scrim dims everything else so
        // the guidance screen is the only focus) ──
        if self.show_onboarding {
            self.render_onboarding_modal(frame);
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let cpu = self.system.global_cpu_info().cpu_usage();
        let mem_used = self.system.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
        let mem_total = self.system.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
        let mem_pct = if mem_total > 0.0 {
            mem_used / mem_total
        } else {
            0.0
        };

        let statuses = self.manager.get_statuses();
        let total = statuses.len();
        let running = statuses
            .iter()
            .filter(|(_, s)| matches!(s, crate::engine::ProcessStatus::Running(_)))
            .count();

        let cpu_color = if cpu > 80.0 {
            RED
        } else if cpu > 40.0 {
            YELLOW
        } else {
            GREEN
        };
        let mem_color = if mem_pct > 0.85 {
            RED
        } else if mem_pct > 0.6 {
            YELLOW
        } else {
            GREEN
        };

        let meter = |pct: f64, color: Color| {
            let filled = (pct * 10.0).round() as usize;
            let bar: String = (0..10)
                .map(|i| if i < filled { '█' } else { '░' })
                .collect();
            Span::styled(bar, Style::default().fg(color))
        };

        let view_label = match self.active_view {
            crate::common::ActiveView::Dashboard => "DASHBOARD",
            crate::common::ActiveView::Projects => "PROJECTS",
            crate::common::ActiveView::Logs => "LOGS",
            _ => "",
        };

        // ── Left cluster: brand + version + system meters + running count ──
        let left_line = Line::from(vec![
            Span::styled(
                " MATRIX ",
                Style::default()
                    .fg(Color::White)
                    .bg(PURPLE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("v{}", crate::app::matrix_version_display()),
                Style::default().fg(TEXT_DIM).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled("CPU ", Style::default().fg(TEXT_DIM)),
            meter(cpu as f64 / 100.0, cpu_color),
            Span::styled(format!(" {:>5.1}%", cpu), Style::default().fg(cpu_color)),
            Span::raw("   "),
            Span::styled("RAM ", Style::default().fg(TEXT_DIM)),
            meter(mem_pct, mem_color),
            Span::styled(
                format!(" {:>4.1}/{:.1}G", mem_used, mem_total),
                Style::default().fg(mem_color),
            ),
            Span::raw("   "),
            Span::styled(
                "● ",
                Style::default().fg(if running > 0 { GREEN } else { GRAY }),
            ),
            Span::styled(
                format!("{}", running),
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("/{}", total), Style::default().fg(TEXT_DIM)),
        ]);

        // ── Right cluster: current view (right-aligned) ──
        let right_line = Line::from(vec![
            Span::styled("▏ ", Style::default().fg(GRAY)),
            Span::styled(
                view_label,
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
        ]);

        let block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(BORDER));

        // Border spans the full header; left + right clusters overlay it.
        frame.render_widget(block.clone(), area);
        let inner = block.inner(area);
        frame.render_widget(Paragraph::new(left_line), inner);
        frame.render_widget(
            Paragraph::new(right_line).alignment(ratatui::layout::Alignment::Right),
            inner,
        );
    }

    fn render_body(&mut self, frame: &mut Frame, area: Rect) {
        let sidebar_width = if self.is_sidebar_visible { 20 } else { 0 };

        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(0)])
            .split(area);

        if self.is_sidebar_visible {
            self.render_sidebar(frame, content_chunks[0]);
        }

        let main_area = if self.is_sidebar_visible {
            content_chunks[1]
        } else {
            area
        };

        match self.active_view {
            crate::common::ActiveView::Dashboard => {
                dashboard::render(frame, main_area, &self.dashboard_model, &self.manager)
            }
            crate::common::ActiveView::Projects => projects::render(
                frame,
                main_area,
                &self.projects_model,
                &self.manager,
                self.is_command_mode,
            ),
            crate::common::ActiveView::Logs => {
                logs::render(frame, main_area, &mut self.logs_model, &self.manager)
            }
            _ => {}
        }
    }

    /// Dim the whole frame behind a modal overlay so it is the only focus.
    /// Terminals have no alpha — a solid dark fill IS the scrim. Call at the
    /// START of every modal render, before the panel's `Clear` + block.
    fn draw_scrim(&self, frame: &mut Frame) {
        let scrim = Paragraph::new("").style(Style::default().bg(SCRIM));
        frame.render_widget(scrim, frame.size());
    }

    fn render_command_palette(&self, frame: &mut Frame) {
        let a = frame.size();

        // Scrim dims everything behind the palette; the panel carries its own
        // background so it reads as a raised card.
        self.draw_scrim(frame);

        // ── Responsive geometry, but a FIXED panel height per terminal size.
        // The height depends ONLY on the terminal, never on the match count —
        // that's what stopped the modal from jumping/resizing as you type. ──
        let chrome_top: u16 = 3;
        let chrome_bottom: u16 = 1;
        let avail_h = a.height.saturating_sub(chrome_top + chrome_bottom);
        let avail_y = a.y + chrome_top;

        let panel_w: u16 = 80.min(a.width.saturating_sub(4)).max(48);

        // Visible list height is fixed for this terminal (capped to fit).
        let max_list_h: u16 = 14.min(avail_h.saturating_sub(5)).max(3);

        // Panel height is constant per terminal: title + input row + list + hint.
        let panel_h: u16 = (max_list_h + 4).min(avail_h).max(5);

        // Centered within the chrome-free region (stable — never moves on typing).
        let x = a.x + (a.width.saturating_sub(panel_w)) / 2;
        let y = avail_y + (avail_h.saturating_sub(panel_h)) / 2;
        let panel = Rect::new(x, y, panel_w, panel_h);

        let border_color = if self.path_suggestions.is_empty() {
            PURPLE
        } else {
            CYAN
        };

        let title: String = if self.path_suggestions.is_empty() {
            if self.command_input.is_empty() {
                " Commands ".to_string()
            } else {
                format!(" Commands: \"{}\" ", self.command_input)
            }
        } else if self.command_input.starts_with("env") || self.command_input.starts_with("project")
        {
            " Project IDs ".to_string()
        } else {
            " Paths ".to_string()
        };

        let hint = if self.path_suggestions.is_empty() {
            " ↑↓ · Tab copy · Enter run · Esc "
        } else {
            " ↑↓ · Enter select · Esc "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(PANEL_BG))
            .title(Span::styled(title, Style::default().fg(border_color)))
            .title_bottom(Span::styled(hint, Style::default().fg(TEXT_DIM)));

        frame.render_widget(Clear, panel);
        frame.render_widget(block.clone(), panel);
        let inner = block.inner(panel);

        // inner[0] = input row (with cursor); inner[1] = list.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);
        let input_area = chunks[0];
        let list_area = chunks[1];

        let before =
            &self.command_input[..self.command_cursor_position.min(self.command_input.len())];
        let after =
            &self.command_input[self.command_cursor_position.min(self.command_input.len())..];
        let input_line = Line::from(vec![
            Span::styled(
                " : ",
                Style::default()
                    .fg(Color::Black)
                    .bg(YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                before.to_string(),
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(YELLOW)),
            Span::styled(
                after.to_string(),
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(input_line).style(Style::default().bg(PANEL_BG)),
            input_area,
        );

        // Real terminal cursor sits on the block cursor so it blinks where you type.
        let cursor_x = input_area.x + 3 + before.chars().count() as u16;
        frame.set_cursor(cursor_x, input_area.y);

        let total = if self.path_suggestions.is_empty() {
            self.command_matches.len()
        } else {
            self.path_suggestions.len()
        };

        let vis = (total as u16).min(max_list_h) as usize;
        let start = if total == 0 {
            0
        } else {
            self.selected_suggestion
                .saturating_sub(vis / 2)
                .min(total.saturating_sub(vis))
        };
        let end = if total == 0 {
            0
        } else {
            (start + vis).min(total)
        };

        let items: Vec<ListItem> = if total == 0 {
            vec![ListItem::new(Line::from(Span::styled(
                "  No matches",
                Style::default().fg(TEXT_DIM),
            )))]
        } else if self.path_suggestions.is_empty() {
            self.command_matches[start..end]
                .iter()
                .enumerate()
                .map(|(i, (cmd, usage))| {
                    let idx = start + i;
                    let sel = idx == self.selected_suggestion;
                    let st = if sel {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(TEXT_DIM)
                    };
                    let cmd_style = if sel {
                        Style::default().fg(PURPLE).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(PURPLE)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(if sel { "› " } else { "  " }, st),
                        Span::styled(format!("{:<12}", cmd), cmd_style),
                        Span::styled(usage.clone(), st),
                    ]))
                })
                .collect()
        } else {
            self.path_suggestions[start..end]
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let idx = start + i;
                    let sel = idx == self.selected_suggestion;
                    let st = if sel {
                        Style::default()
                            .bg(PURPLE)
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(TEXT_DIM)
                    };
                    ListItem::new(format!(" {} ", s)).style(st)
                })
                .collect()
        };

        frame.render_widget(
            List::new(items).style(Style::default().bg(PANEL_BG)),
            list_area,
        );
    }

    fn render_sidebar(&self, frame: &mut Frame, area: Rect) {
        let entries = [
            ("dashboard", "Dashboard", '◧'),
            ("projects", "Projects", '◈'),
            ("logs", "Logs", '☰'),
        ];
        let sel = self.sidebar_state.selected().unwrap_or(0);

        let items: Vec<ListItem> = entries
            .iter()
            .enumerate()
            .map(|(i, (_, label, glyph))| {
                let is_sel = i == sel;
                let style = if is_sel {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(TEXT_DIM)
                };
                let bar = if is_sel { "▌" } else { " " };
                let bar_style = if is_sel {
                    Style::default().fg(PURPLE)
                } else {
                    Style::default().fg(GRAY_DIM)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(bar, bar_style),
                    Span::styled(
                        format!(" {} ", glyph),
                        if is_sel {
                            Style::default().fg(PURPLE)
                        } else {
                            Style::default().fg(PURPLE_DIM)
                        },
                    ),
                    Span::styled(*label, style),
                ]))
            })
            .collect();

        let sidebar = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .border_style(Style::default().fg(BORDER)),
            )
            .highlight_style(Style::default());

        frame.render_widget(sidebar, area);

        // Key legend pinned to bottom of the rail.
        let legend_y = area.y + area.height.saturating_sub(6);
        if legend_y > area.y + 3 {
            let legend_area = Rect::new(area.x, legend_y, area.width, 5);
            let legend = Paragraph::new(vec![
                Line::from(Span::styled(
                    " NAV",
                    Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(" ←/→ switch", Style::default().fg(TEXT_DIM))),
                Line::from(Span::styled(" s sidebar", Style::default().fg(TEXT_DIM))),
                Line::from(Span::styled(" : command", Style::default().fg(TEXT_DIM))),
                Line::from(Span::styled(" q quit", Style::default().fg(TEXT_DIM))),
            ]);
            frame.render_widget(legend, legend_area);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let footer_content = if !self.is_command_mode {
            if let Some((msg, time)) = &self.message {
                if time.elapsed().as_secs() < 5 {
                    Line::from(vec![Span::styled(
                        format!(" ● {}", msg.trim()),
                        Style::default().fg(CYAN),
                    )])
                } else {
                    self.footer_default()
                }
            } else {
                self.footer_default()
            }
        } else {
            // In command mode the input lives inside the palette modal, so the
            // footer just shows the standard hints to avoid a duplicate prompt.
            self.footer_default()
        };

        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(BORDER));
        frame.render_widget(Paragraph::new(footer_content).block(block), area);
    }

    fn footer_default(&self) -> Line<'static> {
        // Context-aware hints: left = view, right = hint cluster.
        let left = match self.active_view {
            crate::common::ActiveView::Projects => " Projects ",
            crate::common::ActiveView::Logs => " Logs ",
            crate::common::ActiveView::Dashboard => " Dashboard ",
            _ => " ",
        };
        Line::from(vec![
            Span::styled(
                left,
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Enter start/stop ", Style::default().fg(TEXT_DIM)),
            Span::styled(" r restart ", Style::default().fg(TEXT_DIM)),
            Span::styled(" H host ", Style::default().fg(TEXT_DIM)),
            Span::styled(" p dev/prod ", Style::default().fg(TEXT_DIM)),
            Span::styled(" o open ", Style::default().fg(TEXT_DIM)),
            Span::styled(" c copy ", Style::default().fg(TEXT_DIM)),
            Span::styled(
                " h help ",
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
        ])
    }

    fn render_update_modal(&self, frame: &mut Frame) {
        let Some(info) = &self.update_available else {
            return;
        };
        if self.update_dismissed {
            return;
        }
        if self.show_onboarding {
            return; // the welcome covers everything anyway
        }
        let area = frame.size();
        // Scrim dims the UI behind the card so the update is the only focus.
        self.draw_scrim(frame);
        // Small notification card, bottom-right (above the toast stack).
        // Deliberately minimal: just "update available". The changelog lives
        // on GitHub — `u` opens it and keeps the card visible.
        let w = 44u16.min(area.width.saturating_sub(2));
        let h = 4u16;
        let x = area.x + area.width.saturating_sub(w + 1);
        let y = area.y + area.height.saturating_sub(h + 2); // 2px above the footer
        let rect = Rect::new(x, y, w, h);

        let title = format!(" ⟳ Update: {}", info.tag);
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                " c changelog  ·  u update  ·  Esc dismiss ",
                Style::default().fg(TEXT_DIM),
            )),
        ];
        let p = Paragraph::new(lines)
            .style(Style::default().fg(TEXT).bg(PANEL_BG))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(YELLOW))
                    .style(Style::default().bg(PANEL_BG))
                    .title(Span::styled(
                        title,
                        Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
                    )),
            );
        frame.render_widget(Clear, rect);
        frame.render_widget(p, rect);
    }

    /// Full first-launch guidance screen. A scrim dims the whole UI behind it
    /// so this screen is the only focus; scales with the terminal, scrolls
    /// (↑/↓), action bar pinned at the bottom. Reopen anytime with `h`.
    fn render_onboarding_modal(&mut self, frame: &mut Frame) {
        let a = frame.size();

        // 1) Scrim: cover the entire screen with a dim fill so nothing behind
        //    the welcome screen distracts.
        self.draw_scrim(frame);

        // 2) Panel: as large as the terminal allows, with its own background
        //    so it reads as a raised card above the scrim.
        let panel_w: u16 = 92u16
            .min(a.width.saturating_sub(4))
            .max(50)
            .min(a.width.saturating_sub(4));
        let panel_h: u16 = 40u16
            .min(a.height.saturating_sub(2))
            .max(12)
            .min(a.height.saturating_sub(2));
        let x = a.x + (a.width.saturating_sub(panel_w)) / 2;
        let y = a.y + (a.height.saturating_sub(panel_h)) / 2;
        let panel = Rect::new(x, y, panel_w, panel_h);

        let body = self.onboarding_body(panel_w);
        let actions = [
            "1 Scan this machine",
            "2 Add a project manually",
            "3 Skip for now",
        ];
        let sel = self
            .onboarding_selected
            .min(actions.len().saturating_sub(1));

        let panel_bg = PANEL_BG;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(PURPLE))
            .style(Style::default().bg(panel_bg))
            .title(Span::styled(
                " Welcome to Matrix ",
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " ↑↓ scroll · ←/→ action · Enter run · h/Esc close ",
                Style::default().fg(TEXT_DIM),
            ));
        frame.render_widget(Clear, panel);
        frame.render_widget(block.clone(), panel);
        let inner = block.inner(panel);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        let body_area = chunks[0];
        let action_area = chunks[1];

        // Render the whole body with built-in wrap + scroll: long lines wrap
        // to the next row instead of clipping, and ↑/↓ scrolls the laid-out
        // rows (ratatui scrolls by wrapped rows, not source lines). Clamp the
        // offset to the body so over-scrolling never blanks the panel.
        let scroll_y = self.onboarding_scroll.min(body.len().saturating_sub(1)) as u16;
        frame.render_widget(
            Paragraph::new(body)
                .style(Style::default().bg(panel_bg))
                .wrap(Wrap { trim: false })
                .scroll((scroll_y, 0)),
            body_area,
        );

        // Action bar: the selected entry is highlighted (←/→ to move, Enter
        // to run, or press 1/2/3 directly).
        let mut action_spans: Vec<Span> = Vec::new();
        for (i, label) in actions.iter().enumerate() {
            let is_sel = i == sel;
            let st = if is_sel {
                Style::default()
                    .bg(PURPLE)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(TEXT)
            };
            action_spans.push(Span::styled(format!(" {} ", label), st));
            action_spans.push(Span::raw("  "));
        }
        frame.render_widget(
            Paragraph::new(Line::from(action_spans)).style(Style::default().bg(panel_bg)),
            action_area,
        );
    }

    /// The guidance body: pixel logo, tagline, and three short sections
    /// (get started / keys / commands). Bright, high-contrast, easy to scan.
    fn onboarding_body(&self, panel_w: u16) -> Vec<Line<'static>> {
        let hdr = |s: &str| {
            Span::styled(
                s.to_string(),
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            )
        };
        let txt = |s: &str| Span::styled(s.to_string(), Style::default().fg(TEXT));
        let key = |s: &str| {
            Span::styled(
                s.to_string(),
                Style::default().fg(Color::Rgb(110, 220, 250)),
            )
        };
        let dim = |s: &str| Span::styled(s.to_string(), Style::default().fg(TEXT_DIM));
        // Brighter purple so the pixel logo pops on the dark panel.
        let logo_style = Style::default()
            .fg(Color::Rgb(176, 96, 255))
            .add_modifier(Modifier::BOLD);

        // Pixel-art "MATRIX" (figlet ANSI Shadow), centered as a block.
        let logo = [
            "███╗   ███╗ █████╗ ████████╗██████╗ ██╗██╗  ██╗",
            "████╗ ████║██╔══██╗╚══██╔══╝██╔══██╗██║╚██╗██╔╝",
            "██╔████╔██║███████║   ██║   ██████╔╝██║ ╚███╔╝ ",
            "██║╚██╔╝██║██╔══██║   ██║   ██╔══██╗██║ ██╔██╗ ",
            "██║ ╚═╝ ██║██║  ██║   ██║   ██║  ██║██║██╔╝ ██╗",
            "╚═╝     ╚═╝╚═╝  ╚═╝   ╚═╝   ╚═╝  ╚═╝╚═╝╚═╝  ╚═╝",
        ];
        let logo_w = logo.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let pad = (panel_w as usize).saturating_sub(logo_w) / 2;

        let mut lines: Vec<Line> = Vec::new();
        for l in logo {
            lines.push(Line::from(Span::styled(
                format!("{}{}", " ".repeat(pad), l),
                logo_style,
            )));
        }

        // Tagline, centered.
        let tagline = "Matrix runs your dev servers — start, stop, restart, and follow logs.";
        let tag_pad = (panel_w as usize).saturating_sub(tagline.chars().count()) / 2;

        lines.extend(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("{}{}", " ".repeat(tag_pad), tagline),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(hdr("  GET STARTED")),
            Line::from(vec![
                key("  d"),
                txt("   scan this machine — add detected projects in one keystroke"),
            ]),
            Line::from(vec![
                key("  :"),
                txt("   add manually — "),
                key("project <id> <abs_path> <command>"),
            ]),
            Line::from(""),
            Line::from(hdr("  CONTROL — KEYS")),
        ]);
        // Two-column key table, aligned with fixed-width cells so the action
        // column starts at the same x on every row.
        let table: [(&str, &str, &str, &str); 5] = [
            ("Enter", "start / stop", "p", "dev / prod"),
            ("r", "restart", "H", "host"),
            ("e", "expand group", "o", "open · c copy"),
            ("←/→", "switch views", "s", "sidebar · d detect"),
            (":", "commands", "h", "help · q quit"),
        ];
        for (k1, a1, k2, a2) in table {
            let k1c = format!("  {:<6}", k1);
            let a1c = format!("{:<17}", a1);
            let k2c = format!("{:<4}", k2);
            lines.push(Line::from(vec![key(&k1c), txt(&a1c), key(&k2c), txt(a2)]));
        }
        lines.extend(vec![
            Line::from(""),
            Line::from(hdr("  COMMANDS")),
            Line::from(vec![
                key("  detect"),
                txt(" · "),
                key("start"),
                txt(" · "),
                key("stop"),
                txt(" · "),
                key("restart"),
                txt(" · "),
                key("env <id>"),
                txt(" · "),
                key("status"),
                txt(" · "),
                key("open <url>"),
                txt(" · "),
                key("cd"),
                dim(" · full list: README"),
            ]),
            Line::from(""),
        ]);

        lines
    }

    fn render_toasts(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let stack = self.toasts.len() as u16;
        let toast_h = 3u16;
        let gap = 1u16;
        let max_w = 54u16.min(area.width.saturating_sub(2));
        let x = area.x + area.width.saturating_sub(max_w + 1);

        for (i, toast) in self.toasts.iter().enumerate() {
            let y = area.y
                + area.height.saturating_sub(
                    toast_h * (stack - i as u16) + gap * (stack - i as u16).saturating_sub(1),
                );
            let rect = Rect::new(x, y, max_w, toast_h);

            let (accent, label, tone_color) = match toast.tone {
                ToastTone::Info => ("ℹ", "INFO", CYAN),
                ToastTone::Success => ("✓", "OK", GREEN),
                ToastTone::Warn => ("⚠", "WARN", YELLOW),
                ToastTone::Error => ("✗", "ERR", RED),
            };

            let text = format!(
                " {} [{}] {}: {}",
                accent, label, toast.source, toast.message
            );
            let p = Paragraph::new(text)
                .style(Style::default().fg(Color::White))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(tone_color)),
                );
            frame.render_widget(Clear, rect);
            frame.render_widget(p, rect);
        }
    }

    fn render_detect_modal(&self, frame: &mut Frame) {
        let a = frame.size();
        // Scrim dims everything behind the modal; the panel carries its own
        // background so it reads as a raised card.
        self.draw_scrim(frame);
        // Scale the list with the terminal: use as much vertical space as is
        // available (up to ~60% of the screen), never smaller than 8 rows.
        let max_list_h = (a.height.saturating_mul(6) / 10).clamp(8, 40);
        let list_h = (self.detect_candidates.len() as u16).clamp(1, max_list_h);
        let panel_w: u16 = 84.min(a.width.saturating_sub(4));
        let panel_h: u16 = list_h + 6;
        let x = a.x + (a.width.saturating_sub(panel_w)) / 2;
        let y = a.y + (a.height.saturating_sub(panel_h)) / 2;
        let panel = Rect::new(x, y, panel_w, panel_h);

        frame.render_widget(Clear, panel);

        if self.detect_candidates.is_empty() {
            // On a truly fresh machine (zero projects) the generic "everything
            // is already listed" line would be wrong — point at manual add.
            let manual = self.manager.get_projects().is_empty();
            let msg = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No new projects found.",
                    Style::default().fg(TEXT_DIM),
                )),
                Line::from(if manual {
                    Span::styled(
                        "  Add one manually:  :project <id> <abs_path> <command>",
                        Style::default().fg(TEXT),
                    )
                } else {
                    Span::styled(
                        "  Everything on disk is already listed in Matrix.",
                        Style::default().fg(TEXT_DIM),
                    )
                }),
                Line::from(""),
                Line::from(Span::styled("  Esc to close", Style::default().fg(PURPLE))),
            ])
            .style(Style::default().bg(PANEL_BG))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(PURPLE))
                    .style(Style::default().bg(PANEL_BG))
                    .title(Span::styled(
                        " Detect Projects ",
                        Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
                    )),
            );
            frame.render_widget(msg, panel);
            return;
        }

        let start = self
            .detect_selected
            .saturating_sub((list_h as usize / 2).min(self.detect_selected));
        let end = (start + list_h as usize).min(self.detect_candidates.len());
        let home = std::env::var("HOME").unwrap_or_default();

        let items: Vec<ListItem> = self.detect_candidates[start..end]
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let idx = start + i;
                let sel = idx == self.detect_selected;
                let row_style = if sel {
                    Style::default()
                        .bg(PURPLE)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(TEXT)
                };
                let name = format!("{:<26}", c.name.chars().take(26).collect::<String>());
                let cat = format!("{:<9}", c.category);
                let path = if sel {
                    c.path.clone()
                } else {
                    c.path.replace(&home, "~")
                };
                ListItem::new(Line::from(vec![
                    Span::styled(if sel { "› " } else { "  " }, row_style),
                    Span::styled(name, row_style),
                    Span::styled(
                        cat,
                        if sel {
                            row_style
                        } else {
                            Style::default().fg(CYAN)
                        },
                    ),
                    Span::styled(
                        path,
                        if sel {
                            row_style
                        } else {
                            Style::default().fg(TEXT_DIM)
                        },
                    ),
                ]))
            })
            .collect();

        let list = List::new(items).style(Style::default().bg(PANEL_BG)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(PURPLE))
                .style(Style::default().bg(PANEL_BG))
                .title(Span::styled(
                    " Detect Projects ",
                    Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
                ))
                .title_bottom(Span::styled(
                    format!(
                        " {} found · {} · ↑↓ navigate · Enter add · o sort · Esc close ",
                        self.detect_candidates.len(),
                        if self.detect_sort_by_name {
                            "sorted by name"
                        } else {
                            "sorted by language"
                        }
                    ),
                    Style::default().fg(TEXT_DIM),
                )),
        );
        frame.render_widget(list, panel);
    }
}
