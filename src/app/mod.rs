use arboard::Clipboard;
use ratatui::widgets::ListState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, System};

use crate::common::ActiveView;
use crate::config::MatrixConfig;
use crate::engine::ProcessManager;
use crate::features::{
    dashboard::DashboardModel, env::EnvModel, logs::LogsModel, projects::ProjectsModel,
};

/// Version string for the TUI header: the exact git tag when built from a
/// tagged checkout (embedded at compile time by build.rs), else the Cargo
/// version formatted as `2026.08.12.0`.
pub fn matrix_version_display() -> String {
    let built = env!("MATRIX_BUILD_VERSION");
    if built.is_empty() {
        return format_version(env!("CARGO_PKG_VERSION"));
    }
    built.trim_start_matches('v').to_string()
}

/// Whether the first-launch welcome should auto-show: the user has never been
/// shown it. Projects do NOT suppress it — the saved flag is the only gate,
/// so it appears exactly once even for machines that already have projects.
pub fn needs_onboarding(onboarded: bool) -> bool {
    !onboarded
}

/// Pure formatter for the header version. Cargo requires valid semver (no
/// leading zeros), so `2026.8.12` becomes the user-facing `2026.08.12.0`.
fn format_version(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return v.to_string();
    }
    let (y, m, d) = match (
        parts[0].parse::<u32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) {
        (Ok(y), Ok(m), Ok(d)) => (y, m, d),
        _ => return v.to_string(),
    };
    if (2000..=2999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d) {
        format!("{}.{:02}.{:02}.0", y, m, d)
    } else {
        v.to_string()
    }
}

pub mod commands;
pub mod events;
pub mod ui;

pub struct App {
    pub active_view: ActiveView,
    pub previous_view: ActiveView,
    pub sidebar_state: ListState,
    pub system: System,
    pub is_sidebar_visible: bool,
    pub clipboard: Option<Clipboard>,

    // Command Mode
    pub is_command_mode: bool,
    pub command_input: String,
    pub command_cursor_position: usize,
    pub message: Option<(String, Instant)>,

    // Path Autocompletion
    pub path_suggestions: Vec<String>,
    pub selected_suggestion: usize,

    // Command palette: filtered list of (command, usage) shown while typing
    pub command_matches: Vec<(String, String)>,

    // Detect-projects modal
    pub show_detect_modal: bool,
    pub detect_candidates: Vec<crate::detect::DetectCandidate>,
    pub detect_selected: usize,
    /// Sort mode for the detect modal: false = by language (default),
    /// true = by name.
    pub detect_sort_by_name: bool,

    // First-launch onboarding
    /// True once the first-launch guidance has been shown (persisted to
    /// matrix.json at open time). Never auto-shown again once set.
    pub onboarded: bool,
    /// Whether the welcome modal is currently visible (first run or `h`).
    pub show_onboarding: bool,
    /// Selected onboarding action: 0 scan, 1 manual, 2 skip.
    pub onboarding_selected: usize,
    /// Scroll offset into the guidance body (the screen may be taller than
    /// the terminal; ↑/↓ scrolls).
    pub onboarding_scroll: usize,

    // Performance Optimization
    pub last_sys_refresh: Instant,

    // Feature Models
    pub dashboard_model: DashboardModel,
    pub projects_model: ProjectsModel,
    pub logs_model: LogsModel,
    pub env_model: EnvModel,

    // Core
    pub manager: Arc<ProcessManager>,
    pub config_path: String,

    // Toast notifications (live feedback from socket/TUI actions)
    pub toasts: Vec<crate::common::ToastEvent>,
    pub toast_timestamps: Vec<Instant>,
    /// Sender used by background threads (e.g. self-update) to surface toasts.
    pub toast_tx: tokio::sync::mpsc::UnboundedSender<crate::common::ToastEvent>,

    // Update check (set from background thread; rendered bottom-right)
    pub update_available: Option<crate::update::ReleaseInfo>,
    pub update_dismissed: bool,
}

impl App {
    pub fn new(
        manager: Arc<ProcessManager>,
        config_path: String,
        toast_tx: tokio::sync::mpsc::UnboundedSender<crate::common::ToastEvent>,
        onboarded: bool,
    ) -> Self {
        let mut sidebar_state = ListState::default();
        sidebar_state.select(Some(1));
        let clipboard = Clipboard::new().ok();
        let show_onboarding = needs_onboarding(onboarded);

        let mut app = Self {
            active_view: ActiveView::Projects,
            previous_view: ActiveView::Projects,
            sidebar_state,
            system: System::new_all(),
            is_sidebar_visible: true,
            clipboard,
            is_command_mode: false,
            command_input: String::new(),
            command_cursor_position: 0,
            message: None,
            path_suggestions: Vec::new(),
            selected_suggestion: 0,
            command_matches: Vec::new(),
            show_detect_modal: false,
            detect_candidates: Vec::new(),
            detect_selected: 0,
            detect_sort_by_name: false,
            onboarded,
            show_onboarding,
            onboarding_selected: 0,
            onboarding_scroll: 0,
            last_sys_refresh: Instant::now() - Duration::from_secs(10),
            dashboard_model: DashboardModel::new(),
            projects_model: ProjectsModel::new(),
            logs_model: LogsModel::new(),
            env_model: EnvModel::new(),
            manager,
            config_path,
            toasts: Vec::new(),
            toast_timestamps: Vec::new(),
            toast_tx,
            update_available: None,
            update_dismissed: false,
        };
        if app.show_onboarding {
            // The welcome is about to be shown for the first (and only) time —
            // persist "shown" immediately so a later launch never auto-shows
            // it again, even if the user quits without touching anything.
            app.onboarded = true;
            app.save_config();
        }
        app
    }

    /// Push a transient toast. Older toasts beyond the cap are dropped.
    pub fn push_toast(&mut self, event: crate::common::ToastEvent) {
        self.toasts.push(event);
        self.toast_timestamps.push(Instant::now());
        const MAX_TOASTS: usize = 4;
        if self.toasts.len() > MAX_TOASTS {
            let drop = self.toasts.len() - MAX_TOASTS;
            self.toasts.drain(0..drop);
            self.toast_timestamps.drain(0..drop);
        }
    }

    /// Prune toasts older than `ttl`. Call once per frame.
    pub fn expire_toasts(&mut self, ttl: Duration) {
        let now = Instant::now();
        let mut i = 0;
        while i < self.toast_timestamps.len() {
            if now.duration_since(self.toast_timestamps[i]) >= ttl {
                self.toasts.remove(i);
                self.toast_timestamps.remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub fn update_system(&mut self) {
        if self.last_sys_refresh.elapsed() >= Duration::from_secs(2) {
            self.system
                .refresh_cpu_specifics(CpuRefreshKind::everything().without_frequency());
            self.system
                .refresh_memory_specifics(MemoryRefreshKind::everything());
            self.last_sys_refresh = Instant::now();
        }
    }

    pub fn is_editing(&self) -> bool {
        if self.active_view == ActiveView::Projects {
            return self.projects_model.editing_port.is_some()
                || self.projects_model.editing_category.is_some();
        }
        if self.active_view == ActiveView::EnvEditor {
            return true;
        }
        false
    }

    pub fn update_view(&mut self, index: usize) {
        match index {
            0 => self.active_view = ActiveView::Dashboard,
            1 => self.active_view = ActiveView::Projects,
            2 => self.active_view = ActiveView::Logs,
            _ => {}
        }
        self.previous_view = self.active_view;
    }

    /// Recompute the command palette list from the current input.
    /// When the input is empty, show every command. Otherwise filter by prefix
    /// and show a usage hint (`cmd <arg> ...`) built from COMMAND_TEMPLATES.
    pub fn compute_command_matches(&mut self) {
        let input = self.command_input.trim();
        let templates = crate::app::commands::COMMAND_TEMPLATES;
        if input.is_empty() {
            self.command_matches = templates
                .iter()
                .map(|(cmd, args)| {
                    let usage = args.iter().map(|s| s.trim()).collect::<Vec<_>>().join(" ");
                    (
                        cmd.to_string(),
                        if usage.is_empty() {
                            "(no args)".into()
                        } else {
                            usage
                        },
                    )
                })
                .collect();
        } else {
            let lower = input.to_lowercase();
            self.command_matches = templates
                .iter()
                .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&lower))
                .map(|(cmd, args)| {
                    let usage = args.iter().map(|s| s.trim()).collect::<Vec<_>>().join(" ");
                    (
                        cmd.to_string(),
                        if usage.is_empty() {
                            "(no args)".into()
                        } else {
                            usage
                        },
                    )
                })
                .collect();
        }
        // Keep the suggestion cursor within range once path suggestions are gone.
        if !self.path_suggestions.is_empty() {
            self.selected_suggestion = self
                .selected_suggestion
                .min(self.path_suggestions.len().saturating_sub(1));
        } else if self.command_matches.is_empty() {
            self.selected_suggestion = 0;
        } else {
            self.selected_suggestion = self.selected_suggestion.min(self.command_matches.len() - 1);
        }
    }
    /// Sort the detect candidates per the current sort mode (by language or
    /// by name). Call after scanning or toggling the mode.
    pub fn sort_detect_candidates(&mut self) {
        if self.detect_sort_by_name {
            self.detect_candidates.sort_by(|a, b| {
                a.name
                    .to_lowercase()
                    .cmp(&b.name.to_lowercase())
                    .then_with(|| a.category.cmp(&b.category))
            });
        } else {
            self.detect_candidates.sort_by(|a, b| {
                a.category
                    .cmp(&b.category)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        // Keep the selection valid after re-sorting.
        self.detect_selected = self
            .detect_selected
            .min(self.detect_candidates.len().saturating_sub(1));
    }

    /// Open the detect-projects modal: scan known roots and drop any candidate
    /// whose path or id already exists in Matrix.
    pub fn open_detect_modal(&mut self) {
        let roots = crate::detect::default_roots();
        let mut candidates = crate::detect::scan_projects(&roots);

        let existing_paths: std::collections::HashSet<String> = self
            .manager
            .get_projects()
            .iter()
            .map(|p| {
                std::fs::canonicalize(&p.path)
                    .map(|c| c.to_string_lossy().to_string())
                    .unwrap_or_else(|_| p.path.clone())
            })
            .collect();
        let existing_ids: std::collections::HashSet<String> = self
            .manager
            .get_projects()
            .iter()
            .map(|p| p.id.clone())
            .collect();

        candidates.retain(|c| {
            let canon = std::fs::canonicalize(&c.path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| c.path.clone());
            !existing_paths.contains(&canon) && !existing_ids.contains(&c.id)
        });

        self.detect_candidates = candidates;
        self.detect_selected = 0;
        self.sort_detect_candidates();
        self.show_detect_modal = true;
    }

    /// Add a detected candidate as a Matrix project.
    pub fn add_detected_project(&mut self, candidate: crate::detect::DetectCandidate) {
        self.manager.add_project(crate::config::Project {
            id: candidate.id,
            name: Some(candidate.name),
            path: candidate.path,
            port: None,
            command: if candidate.command.is_empty() {
                None
            } else {
                Some(candidate.command)
            },
            env_only: false,
            category: Some(candidate.category),
            deps: vec![],
            backend: None,
            env: vec![],
        });
        self.save_config();
    }

    /// Persist config to disk.
    pub fn save_config(&mut self) {
        // Config lives in ~/.matrix/, which may not exist yet on first save.
        if let Some(parent) = std::path::Path::new(&self.config_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let new_config = MatrixConfig {
            projects: self.manager.get_projects(),
            templates: self.manager.get_templates(),
            groups: self.manager.get_groups(),
            onboarded: self.onboarded,
        };
        let _ = new_config.save(&self.config_path);
    }

    /// Dismiss the welcome modal. The shown-state was already persisted the
    /// moment the modal opened, so this only hides it for this session.
    pub fn skip_onboarding(&mut self) {
        self.show_onboarding = false;
    }

    /// Run one of the welcome's actions: 0 = scan for projects, 1 = manual
    /// add via the command palette, anything else = skip/close.
    pub fn run_onboarding_action(&mut self, idx: usize) {
        match idx {
            0 => {
                self.show_onboarding = false;
                self.open_detect_modal();
            }
            1 => {
                self.show_onboarding = false;
                self.is_command_mode = true;
                self.command_input = "project ".into();
                self.command_cursor_position = self.command_input.len();
                self.selected_suggestion = 0;
                self.path_suggestions.clear();
                self.compute_command_matches();
            }
            _ => self.show_onboarding = false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_version, needs_onboarding};

    #[test]
    fn date_version_is_zero_padded_with_revision_slot() {
        // Cargo forbids leading zeros in semver, so "2026.8.12" is the source
        // form; the user-facing scheme is "2026.08.12.0".
        assert_eq!(format_version("2026.8.12"), "2026.08.12.0");
    }

    #[test]
    fn single_digit_month_and_day_are_padded() {
        assert_eq!(format_version("2026.12.1"), "2026.12.01.0");
        assert_eq!(format_version("2027.1.31"), "2027.01.31.0");
    }

    #[test]
    fn non_date_semver_renders_unchanged() {
        // Older tags / dev versions must not be mangled by the formatter.
        assert_eq!(format_version("1.1.0"), "1.1.0");
        assert_eq!(format_version("0.1.0"), "0.1.0");
    }

    #[test]
    fn invalid_date_components_render_unchanged() {
        // Not a plausible date (month 13) → fall back to raw semver.
        assert_eq!(format_version("2026.13.1"), "2026.13.1");
    }

    #[test]
    fn non_three_part_versions_render_unchanged() {
        assert_eq!(format_version("2026.8"), "2026.8");
        assert_eq!(format_version("1.0.0-beta.1"), "1.0.0-beta.1");
    }

    #[test]
    fn onboarding_auto_shows_exactly_once() {
        // Never shown before → auto-show, regardless of projects.
        assert!(needs_onboarding(false));
        // Already shown once → never auto-show again.
        assert!(!needs_onboarding(true));
    }
}
