use super::App;
use crate::common::ActiveView;
use crossterm::event::{self, KeyCode, KeyModifiers};
use std::fs;
use std::path::Path;
use std::time::Instant;

pub const COMMAND_TEMPLATES: &[(&str, &[&str])] = &[
    (
        "template",
        &[
            " [-d <name>]",
            " [-n <name>]",
            " [-a <template> <id>]",
            " <name>",
        ],
    ),
    ("project", &[" [-d]", " <id>", " <abs_path>", " <command>"]),
    ("group", &[" <start|stop>", " <group_id>"]),
    ("start", &[" <project_id>"]),
    ("stop", &[" <project_id>"]),
    ("restart", &[" <project_id>"]),
    ("status", &[]),
    ("cd", &[" <path>"]),
    ("env", &[" <project_id>"]),
    ("open", &[" <url>"]),
    ("detect", &[]),
    ("welcome", &[]),
];

/// Split command arguments into positional values and `-flag` tokens.
/// Flags are standalone booleans (e.g. `-d`); any value they act on is a
/// following positional, so `template -a tpl proj` yields
/// `positionals = ["tpl", "proj"]`, `flags = {"a"}`.
fn split_flags<'a>(args: &'a [&'a str]) -> (Vec<&'a str>, std::collections::HashSet<&'a str>) {
    let mut positionals = Vec::new();
    let mut flags = std::collections::HashSet::new();
    for a in args {
        if let Some(f) = a.strip_prefix('-') {
            flags.insert(f);
        } else {
            positionals.push(*a);
        }
    }
    (positionals, flags)
}

impl App {
    pub async fn handle_command_key(&mut self, key: event::KeyEvent) -> anyhow::Result<()> {
        match key.code {
            KeyCode::Enter => {
                if !self.path_suggestions.is_empty() {
                    self.apply_suggestion();
                } else {
                    let input = self.command_input.clone();
                    self.execute_command(&input).await?;
                    self.command_input.clear();
                    self.command_cursor_position = 0;
                    self.is_command_mode = false;
                }
            }
            KeyCode::Esc => {
                if !self.path_suggestions.is_empty() {
                    self.path_suggestions.clear();
                } else {
                    self.command_input.clear();
                    self.command_cursor_position = 0;
                    self.is_command_mode = false;
                }
            }
            KeyCode::Tab => {
                if !self.path_suggestions.is_empty() {
                    // Autocomplete a project-id / path argument.
                    self.apply_suggestion();
                } else if !self.command_matches.is_empty() {
                    let idx = self.selected_suggestion.min(self.command_matches.len() - 1);
                    let cmd = self.command_matches[idx].0.clone();
                    self.command_input = cmd;
                    self.command_cursor_position = self.command_input.len();
                    self.refresh_suggestions().await;
                }
            }
            KeyCode::Down => {
                if !self.path_suggestions.is_empty() {
                    self.selected_suggestion =
                        (self.selected_suggestion + 1) % self.path_suggestions.len();
                } else if !self.command_matches.is_empty() {
                    let n = self.command_matches.len();
                    self.selected_suggestion = (self.selected_suggestion + 1) % n;
                }
            }
            KeyCode::Up => {
                if !self.path_suggestions.is_empty() {
                    self.selected_suggestion =
                        (self.selected_suggestion + self.path_suggestions.len() - 1)
                            % self.path_suggestions.len();
                } else if !self.command_matches.is_empty() {
                    let n = self.command_matches.len();
                    self.selected_suggestion = (self.selected_suggestion + n - 1) % n;
                }
            }
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.delete_word_backwards().await;
            }
            KeyCode::Char(c) => {
                self.command_input.insert(self.command_cursor_position, c);
                self.command_cursor_position += 1;
                self.refresh_suggestions().await;
            }
            KeyCode::Backspace => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.delete_word_backwards().await;
                } else if self.command_cursor_position > 0 {
                    self.command_input.remove(self.command_cursor_position - 1);
                    self.command_cursor_position -= 1;
                    self.refresh_suggestions().await;
                }
            }
            KeyCode::Delete => {
                if self.command_cursor_position < self.command_input.len() {
                    self.command_input.remove(self.command_cursor_position);
                    self.refresh_suggestions().await;
                }
            }
            KeyCode::Left => {
                if self.command_cursor_position > 0 {
                    self.command_cursor_position -= 1;
                }
            }
            KeyCode::Right if self.command_cursor_position < self.command_input.len() => {
                self.command_cursor_position += 1;
            }
            _ => {}
        }
        self.compute_command_matches();
        Ok(())
    }

    async fn delete_word_backwards(&mut self) {
        if self.command_cursor_position == 0 {
            return;
        }

        let before_cursor = &self.command_input[..self.command_cursor_position];
        let mut last_word_start = 0;
        let mut found_non_space = false;

        for (i, c) in before_cursor.char_indices().rev() {
            if c.is_whitespace() {
                if found_non_space {
                    last_word_start = i + 1;
                    break;
                }
            } else {
                found_non_space = true;
            }
        }

        let removed_len = self.command_cursor_position - last_word_start;
        for _ in 0..removed_len {
            self.command_input.remove(last_word_start);
        }
        self.command_cursor_position = last_word_start;
        self.refresh_suggestions().await;
    }

    pub async fn refresh_suggestions(&mut self) {
        let input = &self.command_input;
        let parts: Vec<&str> = input.split_whitespace().collect();

        if parts.is_empty() {
            self.path_suggestions.clear();
            return;
        }

        let cmd = parts[0];
        let has_trailing_space = input.ends_with(' ');

        // Case 1: Project ID Suggestions (for env, project -d, template -a)
        let is_id_arg = match cmd {
            "env" | "project" if parts.len() == 1 && has_trailing_space => true,
            "env" | "project" if parts.len() == 2 && !has_trailing_space => true,
            "template" if parts.len() == 2 && has_trailing_space => true,
            "template" if parts.len() == 3 && !has_trailing_space => true,
            _ => false,
        };

        if is_id_arg {
            let fragment = if has_trailing_space {
                "".to_string()
            } else {
                parts.last().unwrap_or(&"").to_string()
            };
            let mut ids: Vec<String> = self
                .manager
                .get_projects()
                .into_iter()
                .map(|p| p.id)
                .collect();
            ids.retain(|id| id.starts_with(&fragment));
            ids.sort();
            self.path_suggestions = ids;
            self.selected_suggestion = 0;
            return;
        }

        // Case 2: Path Suggestions (for project <path>, cd) — offloaded to spawn_blocking
        let is_path_arg = match cmd {
            "project"
                if !parts.contains(&"-d")
                    && ((parts.len() == 2 && has_trailing_space)
                        || (parts.len() == 3 && !has_trailing_space)) =>
            {
                true
            }
            "cd" if (parts.len() == 1 && has_trailing_space)
                || (parts.len() == 2 && !has_trailing_space) =>
            {
                true
            }
            _ => false,
        };

        if is_path_arg {
            let fragment = if has_trailing_space {
                "".to_string()
            } else {
                parts.last().unwrap_or(&"").to_string()
            };
            let mut path_to_search = fragment.clone();
            if path_to_search.starts_with('~') {
                if let Some(home) = std::env::var_os("HOME") {
                    path_to_search = path_to_search.replace('~', home.to_str().unwrap_or(""));
                }
            }

            let suggestions = tokio::task::spawn_blocking(move || {
                let path = std::path::Path::new(&path_to_search);
                let (dir_to_read, search_term) =
                    if path_to_search.ends_with('/') || path_to_search.is_empty() {
                        let dir = if path_to_search.is_empty() {
                            std::path::Path::new(".")
                        } else {
                            path
                        };
                        (dir.to_path_buf(), String::new())
                    } else {
                        let dir = path
                            .parent()
                            .unwrap_or_else(|| std::path::Path::new("."))
                            .to_path_buf();
                        let term = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        (dir, term)
                    };

                let mut suggestions = Vec::new();
                if let Ok(entries) = fs::read_dir(&dir_to_read) {
                    for entry in entries.flatten() {
                        if let Ok(file_type) = entry.metadata().map(|m| m.file_type()) {
                            if file_type.is_dir() {
                                let name = entry.file_name().to_string_lossy().to_string();
                                if name.starts_with(&search_term) {
                                    let mut full_path =
                                        dir_to_read.join(&name).to_string_lossy().to_string();
                                    if !full_path.ends_with('/') {
                                        full_path.push('/');
                                    }
                                    suggestions.push(full_path);
                                }
                            }
                        }
                    }
                }
                suggestions.sort();
                suggestions
            })
            .await
            .unwrap_or_default();

            self.path_suggestions = suggestions;
            self.selected_suggestion = 0;
            return;
        }

        self.path_suggestions.clear();
    }

    pub fn apply_suggestion(&mut self) {
        if self.selected_suggestion < self.path_suggestions.len() {
            let suggestion = self.path_suggestions[self.selected_suggestion].clone();
            let mut parts: Vec<String> = self
                .command_input
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();

            if self.command_input.ends_with(' ') {
                parts.push(suggestion);
            } else if !parts.is_empty() {
                let last_idx = parts.len() - 1;
                parts[last_idx] = suggestion;
            }

            self.command_input = parts.join(" ");
            self.command_input.push(' ');
            self.command_cursor_position = self.command_input.len();
            self.path_suggestions.clear();
        }
    }

    pub async fn execute_command(&mut self, input: &str) -> anyhow::Result<()> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }
        let cmd = parts[0];

        match cmd {
            "template" => {
                let args: Vec<&str> = parts[1..].to_vec();
                let (positionals, flags) = split_flags(&args);
                if flags.contains("d") {
                    // template -d <name>  → delete a template
                    if let Some(name) = positionals.first() {
                        self.manager.delete_template(name);
                        self.save_config();
                        self.message = Some((format!("Template {} deleted", name), Instant::now()));
                    } else {
                        self.message =
                            Some(("Usage: template -d <name>".to_string(), Instant::now()));
                    }
                } else if flags.contains("n") {
                    // template -n <name>  → create a new template
                    if let Some(name) = positionals.first() {
                        self.manager.create_template(name.to_string());
                        self.save_config();
                        self.message = Some((format!("Template {} created", name), Instant::now()));
                    } else {
                        self.message =
                            Some(("Usage: template -n <name>".to_string(), Instant::now()));
                    }
                } else if flags.contains("a") {
                    // template -a <template> <id>  → add project to template
                    if positionals.len() >= 2 {
                        let _ = self
                            .manager
                            .add_to_template(positionals[0], positionals[1].to_string());
                        self.save_config();
                    } else {
                        self.message = Some((
                            "Usage: template -a <template_name> <project_id>".to_string(),
                            Instant::now(),
                        ));
                    }
                } else if let Some(name) = positionals.first() {
                    // template <name>  → run the template
                    match self.manager.run_template(name).await {
                        Ok(_) => {
                            self.message =
                                Some((format!("Template {} started", name), Instant::now()))
                        }
                        Err(e) => {
                            self.message = Some((format!("Template failed: {}", e), Instant::now()))
                        }
                    }
                } else {
                    self.message = Some(("Usage: template <name>".to_string(), Instant::now()));
                }
            }
            "project" => {
                let args: Vec<&str> = parts[1..].to_vec();
                let (positionals, flags) = split_flags(&args);
                if flags.contains("d") {
                    // project -d <id>  → remove a project
                    if let Some(id) = positionals.first() {
                        self.manager.remove_project(id);
                        self.save_config();
                        self.message = Some((format!("Project {} removed", id), Instant::now()));
                    } else {
                        self.message = Some(("Usage: project -d <id>".to_string(), Instant::now()));
                    }
                } else if positionals.len() >= 3 {
                    // project <id> <abs_path> <command>
                    let id = positionals[0].to_string();
                    let path_str = positionals[1].to_string();
                    let cmd_str = positionals[2..].join(" ");
                    let path = Path::new(&path_str);
                    let abs_path = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        std::env::current_dir()
                            .unwrap_or_else(|_| std::path::PathBuf::from("."))
                            .join(path)
                    };
                    let final_path = fs::canonicalize(&abs_path).unwrap_or(abs_path);
                    self.manager.add_project(crate::config::Project {
                        id,
                        name: None,
                        path: final_path.to_string_lossy().to_string(),
                        port: None,
                        command: Some(cmd_str),
                        env_only: false,
                        category: None,
                        deps: vec![],
                        backend: None,
                        env: vec![],
                    });
                    self.save_config();
                } else {
                    self.message = Some((
                        "Usage: project <id> <abs_path> <command>  |  project -d <id>".to_string(),
                        Instant::now(),
                    ));
                }
            }
            "env" => {
                if parts.len() >= 2 {
                    let id = parts[1];
                    let projects = self.manager.get_projects();
                    if let Some(project) = projects.into_iter().find(|p| p.id == id) {
                        let env_path = Path::new(&project.path).join(".env");
                        match fs::read_to_string(&env_path) {
                            Ok(content) => {
                                self.env_model.load(
                                    id.to_string(),
                                    env_path.to_string_lossy().to_string(),
                                    content,
                                );
                                self.previous_view = self.active_view;
                                self.active_view = ActiveView::EnvEditor;
                            }
                            Err(e) => {
                                self.message =
                                    Some((format!("Could not read .env: {}", e), Instant::now()));
                            }
                        }
                    } else {
                        self.message = Some((format!("Project {} not found", id), Instant::now()));
                    }
                }
            }
            "open" => {
                let url = parts[1..].join(" ");
                if url.starts_with("http://") || url.starts_with("https://") {
                    match crate::url::open_in_browser(&url) {
                        Ok(_) => self.message = Some((format!("Opened: {}", url), Instant::now())),
                        Err(e) => {
                            self.message = Some((format!("Failed to open: {}", e), Instant::now()))
                        }
                    }
                } else {
                    self.message = Some(("Usage: open <url>".to_string(), Instant::now()));
                }
            }
            "detect" => {
                self.open_detect_modal();
                self.is_command_mode = false;
            }
            "welcome" => {
                self.show_onboarding = true;
                self.onboarding_selected = 0;
                self.onboarding_scroll = 0;
            }
            "group" => {
                if parts.len() >= 3 {
                    let action = parts[1];
                    let group_id = parts[2];
                    match action {
                        "start" => match self.manager.start_group(group_id).await {
                            Ok(_) => {
                                self.message =
                                    Some((format!("Group {} started", group_id), Instant::now()));
                                self.push_toast(crate::common::ToastEvent {
                                    source: "tui".into(),
                                    message: format!("group {} started", group_id),
                                    tone: crate::common::ToastTone::Success,
                                });
                            }
                            Err(e) => {
                                self.message =
                                    Some((format!("Group start failed: {}", e), Instant::now()));
                                self.push_toast(crate::common::ToastEvent {
                                    source: "tui".into(),
                                    message: format!("group {} failed: {}", group_id, e),
                                    tone: crate::common::ToastTone::Warn,
                                });
                            }
                        },
                        "stop" => match self.manager.stop_group(group_id).await {
                            Ok(_) => {
                                self.message =
                                    Some((format!("Group {} stopped", group_id), Instant::now()));
                                self.push_toast(crate::common::ToastEvent {
                                    source: "tui".into(),
                                    message: format!("group {} stopped", group_id),
                                    tone: crate::common::ToastTone::Success,
                                });
                            }
                            Err(e) => {
                                self.message =
                                    Some((format!("Group stop failed: {}", e), Instant::now()));
                                self.push_toast(crate::common::ToastEvent {
                                    source: "tui".into(),
                                    message: format!("group {} failed: {}", group_id, e),
                                    tone: crate::common::ToastTone::Warn,
                                });
                            }
                        },
                        _ => {
                            self.message = Some((
                                "Usage: group <start|stop> <group_id>".to_string(),
                                Instant::now(),
                            ));
                        }
                    }
                } else {
                    // List available groups
                    let groups = self.manager.get_groups();
                    if groups.is_empty() {
                        self.message = Some(("No groups defined".to_string(), Instant::now()));
                    } else {
                        let group_list: Vec<String> = groups
                            .iter()
                            .map(|g| format!("{} ({})", g.id, g.name))
                            .collect();
                        self.message =
                            Some((format!("Groups: {}", group_list.join(", ")), Instant::now()));
                    }
                }
            }
            "start" => {
                if parts.len() >= 2 {
                    let id = parts[1];
                    match self.manager.start(id) {
                        Ok(_) => {
                            self.message = Some((format!("{} started", id), Instant::now()));
                            self.push_toast(crate::common::ToastEvent {
                                source: "tui".into(),
                                message: format!("started {}", id),
                                tone: crate::common::ToastTone::Success,
                            });
                        }
                        Err(e) => {
                            self.message =
                                Some((format!("Failed to start {}: {}", id, e), Instant::now()));
                            self.push_toast(crate::common::ToastEvent {
                                source: "tui".into(),
                                message: format!("failed to start {}: {}", id, e),
                                tone: crate::common::ToastTone::Error,
                            });
                        }
                    }
                } else {
                    self.message = Some(("Usage: start <project_id>".to_string(), Instant::now()));
                }
            }
            "stop" => {
                if parts.len() >= 2 {
                    let id = parts[1];
                    let _ = self.manager.stop(id).await;
                    self.message = Some((format!("{} stopped", id), Instant::now()));
                    self.push_toast(crate::common::ToastEvent {
                        source: "tui".into(),
                        message: format!("stopped {}", id),
                        tone: crate::common::ToastTone::Success,
                    });
                } else {
                    self.message = Some(("Usage: stop <project_id>".to_string(), Instant::now()));
                }
            }
            "restart" => {
                if parts.len() >= 2 {
                    let id = parts[1];
                    let _ = self.manager.stop(id).await;
                    match self.manager.start(id) {
                        Ok(_) => {
                            self.message = Some((format!("{} restarted", id), Instant::now()));
                            self.push_toast(crate::common::ToastEvent {
                                source: "tui".into(),
                                message: format!("restarted {}", id),
                                tone: crate::common::ToastTone::Success,
                            });
                        }
                        Err(e) => {
                            self.message =
                                Some((format!("Failed to restart {}: {}", id, e), Instant::now()));
                            self.push_toast(crate::common::ToastEvent {
                                source: "tui".into(),
                                message: format!("failed to restart {}: {}", id, e),
                                tone: crate::common::ToastTone::Error,
                            });
                        }
                    }
                } else {
                    self.message =
                        Some(("Usage: restart <project_id>".to_string(), Instant::now()));
                }
            }
            "status" => {
                let statuses = self.manager.get_statuses();
                if statuses.is_empty() {
                    self.message = Some(("No projects".to_string(), Instant::now()));
                } else {
                    let lines: Vec<String> = statuses
                        .iter()
                        .map(|(config, status)| {
                            let status_str = match status {
                                crate::engine::ProcessStatus::Stopped => "stopped".to_string(),
                                crate::engine::ProcessStatus::Starting => "starting".to_string(),
                                crate::engine::ProcessStatus::Running(pid) => {
                                    format!("running (pid {})", pid)
                                }
                                crate::engine::ProcessStatus::Crashed(msg) => {
                                    format!("crashed ({})", msg)
                                }
                            };
                            format!("{}: {}", config.id, status_str)
                        })
                        .collect();
                    self.message = Some((lines.join("\n"), Instant::now()));
                }
                self.push_toast(crate::common::ToastEvent {
                    source: "tui".into(),
                    message: "status printed below".into(),
                    tone: crate::common::ToastTone::Info,
                });
            }
            _ => {}
        }
        Ok(())
    }
}
