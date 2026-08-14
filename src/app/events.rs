use super::App;
use crate::common::ActiveView;
use crate::features::{
    dashboard::DashboardController,
    env::{EnvAction, EnvController},
    logs::{LogAction, LogsController},
    projects::{ProjectAction, ProjectsController},
};
use crossterm::event::{self, Event, KeyCode};
use std::time::Instant;

impl App {
    pub async fn handle_event(&mut self, event: Event) -> anyhow::Result<bool> {
        match event {
            Event::Key(key) => {
                // Update-available notification: `c` opens the changelog in
                // the browser, `u` triggers the self-update immediately, Esc
                // dismisses. Enter deliberately does nothing — the card must
                // never hijack the universal confirm key.
                if !self.is_command_mode
                    && !self.show_detect_modal
                    && !self.show_onboarding
                    && self.update_available.is_some()
                    && !self.update_dismissed
                {
                    match key.code {
                        KeyCode::Char('c') => {
                            if let Some(info) = &self.update_available {
                                let url = info.url.clone();
                                let _ = crate::url::open_in_browser(&url);
                            }
                            // `c` never dismisses — keep reading the card.
                            return Ok(false);
                        }
                        KeyCode::Char('u') => {
                            // Run the self-update on a background thread so the
                            // TUI stays responsive; the result arrives as a
                            // toast. Dismiss the card — the user acted on it.
                            let tx = self.toast_tx.clone();
                            let current = crate::app::matrix_version_display();
                            std::thread::spawn(move || {
                                let (message, tone) = match crate::update::perform_update(&current)
                                {
                                    Ok(msg) => (msg, crate::common::ToastTone::Success),
                                    Err(msg) => (msg, crate::common::ToastTone::Error),
                                };
                                let _ = tx.send(crate::common::ToastEvent {
                                    source: "update".into(),
                                    message,
                                    tone,
                                });
                            });
                            self.update_dismissed = true;
                            return Ok(false);
                        }
                        KeyCode::Esc => {
                            self.update_dismissed = true;
                            return Ok(false);
                        }
                        _ => {}
                    }
                }
                if self.is_command_mode {
                    self.handle_command_key(key).await?;
                } else if self.show_detect_modal {
                    // `o` toggles sort order (by language <-> by name) — handle
                    // it here because it needs App state, not just the list.
                    if key.code == KeyCode::Char('o') {
                        self.detect_sort_by_name = !self.detect_sort_by_name;
                        self.sort_detect_candidates();
                        return Ok(false);
                    }
                    match crate::detect::DetectController::handle_key(
                        key.code,
                        &mut self.detect_selected,
                        &mut self.detect_candidates,
                    ) {
                        crate::detect::DetectAction::Close => self.show_detect_modal = false,
                        crate::detect::DetectAction::Add(c) => {
                            self.add_detected_project(c);
                            self.push_toast(crate::common::ToastEvent {
                                source: "tui".into(),
                                message: "project added".into(),
                                tone: crate::common::ToastTone::Success,
                            });
                            if self.detect_candidates.is_empty() {
                                self.show_detect_modal = false;
                            }
                        }
                        crate::detect::DetectAction::None => {}
                    }
                } else if self.show_onboarding {
                    // Guidance screen owns the keys: ↑/↓ scroll the body,
                    // ←/→ pick an action, Enter or 1/2/3 runs it, h/Esc closes.
                    const ONBOARDING_ACTIONS: usize = 3;
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.onboarding_scroll = self.onboarding_scroll.saturating_add(1);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.onboarding_scroll = self.onboarding_scroll.saturating_sub(1);
                        }
                        KeyCode::Left => {
                            self.onboarding_selected =
                                (self.onboarding_selected + ONBOARDING_ACTIONS - 1)
                                    % ONBOARDING_ACTIONS;
                        }
                        KeyCode::Right => {
                            self.onboarding_selected =
                                (self.onboarding_selected + 1) % ONBOARDING_ACTIONS;
                        }
                        KeyCode::Char('1') => self.run_onboarding_action(0),
                        KeyCode::Char('2') => self.run_onboarding_action(1),
                        KeyCode::Char('3') => self.run_onboarding_action(2),
                        KeyCode::Enter => {
                            let idx = self.onboarding_selected.min(ONBOARDING_ACTIONS - 1);
                            self.run_onboarding_action(idx);
                        }
                        KeyCode::Char('h') | KeyCode::Esc => self.skip_onboarding(),
                        _ => {}
                    }
                } else if self.active_view == ActiveView::EnvEditor {
                    let action = EnvController::handle_key(key, &mut self.env_model);
                    match action {
                        EnvAction::Exit => self.active_view = self.previous_view,
                        EnvAction::Message(msg) => self.message = Some((msg, Instant::now())),
                        EnvAction::None => {}
                    }
                } else {
                    if !self.is_editing() && key.code == KeyCode::Char('q') {
                        return Ok(true); // Quit
                    }
                    self.handle_normal_key(key).await?;
                }
            }
            Event::Mouse(mouse) => match self.active_view {
                ActiveView::Dashboard => {
                    DashboardController::handle_mouse(mouse, &mut self.dashboard_model)
                }
                ActiveView::Logs => {
                    LogsController::handle_mouse(mouse, &mut self.logs_model, &self.manager)
                }
                _ => {}
            },
            _ => {}
        }
        Ok(false)
    }

    pub async fn handle_normal_key(&mut self, key: event::KeyEvent) -> anyhow::Result<()> {
        if self.is_editing() {
            if self.active_view == ActiveView::Projects {
                let action = ProjectsController::handle_key(
                    key.code,
                    &mut self.projects_model,
                    &self.manager,
                )
                .await;
                match action {
                    ProjectAction::Message(msg) => self.message = Some((msg, Instant::now())),
                    ProjectAction::SaveConfigWithMsg(msg) => {
                        self.save_config();
                        self.message = Some((msg, Instant::now()));
                    }
                    ProjectAction::None => {}
                }
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char(':') => {
                self.is_command_mode = true;
                self.command_input.clear();
                self.command_cursor_position = 0;
                self.selected_suggestion = 0;
                self.compute_command_matches();
            }
            KeyCode::Char('s') => {
                self.is_sidebar_visible = !self.is_sidebar_visible;
            }
            KeyCode::Char('d') => {
                self.open_detect_modal();
            }
            // `h` = help: reopen the first-launch guide on demand whenever
            // the user gets stuck (host mode moved to `H` in the Logs view).
            KeyCode::Char('h') => {
                self.show_onboarding = true;
                self.onboarding_selected = 0;
                self.onboarding_scroll = 0;
            }
            KeyCode::Left => {
                let current = self.sidebar_state.selected().unwrap_or(0);
                let next = (current + 2) % 3;
                self.sidebar_state.select(Some(next));
                self.update_view(next);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let current = self.sidebar_state.selected().unwrap_or(0);
                let next = (current + 1) % 3;
                self.sidebar_state.select(Some(next));
                self.update_view(next);
            }
            _ => match self.active_view {
                ActiveView::Dashboard => {
                    DashboardController::handle_key(key.code, &mut self.dashboard_model)
                }
                ActiveView::Projects => {
                    let action = ProjectsController::handle_key(
                        key.code,
                        &mut self.projects_model,
                        &self.manager,
                    )
                    .await;
                    match action {
                        ProjectAction::Message(msg) => self.message = Some((msg, Instant::now())),
                        ProjectAction::SaveConfigWithMsg(msg) => {
                            self.save_config();
                            self.message = Some((msg, Instant::now()));
                        }
                        ProjectAction::None => {}
                    }
                }
                ActiveView::Logs => {
                    let action = LogsController::handle_key(
                        key.code,
                        &mut self.logs_model,
                        &self.manager,
                        &mut self.clipboard,
                    )
                    .await;
                    if let LogAction::Message(msg) = action {
                        self.message = Some((msg, Instant::now()));
                    }
                }
                _ => {}
            },
        }
        Ok(())
    }
}
