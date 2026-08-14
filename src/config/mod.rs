use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub name: Option<String>,
    pub path: String, // Absolute path
    pub port: Option<u16>,
    pub command: Option<String>,
    pub env_only: bool,
    pub category: Option<String>,
    /// Project ids that must be started before this one.
    #[serde(default)]
    pub deps: Vec<String>,
    /// If set, Matrix auto-registers a virtual backend project (`engine:<id>`)
    /// that starts before and stops with this project.
    #[serde(default)]
    pub backend: Option<BackendSpec>,
    /// Environment variables to inject into this project's process.
    #[serde(default)]
    pub env: Vec<EnvSpec>,
}

impl Project {
    pub fn get_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            PathBuf::from(&self.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&self.id)
                .to_string()
        })
    }
}

/// Spec for a project's auto-registered backend process (`engine:<id>`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackendSpec {
    pub path: String,
    pub command: Option<String>,
    /// Fixed port for the backend. If unset, resolves dynamically (parent port + 1).
    #[serde(default)]
    pub port: Option<u16>,
    /// Project ids started before the backend.
    #[serde(default)]
    pub deps: Vec<String>,
    /// Env specs for the backend process. Placeholders resolve against the
    /// PARENT project (e.g. `{{id}}`, `{{path}}`, `{{parent_port}}`).
    #[serde(default)]
    pub env: Vec<EnvSpec>,
    #[serde(default)]
    pub category: Option<String>,
}

/// Declarative environment variable for a process.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvSpec {
    pub key: String,
    /// Static or templated value ({{placeholders}}).
    #[serde(default)]
    pub value: Option<String>,
    /// Files (relative to the project path) to read the key from, in order;
    /// first file that defines the key wins.
    #[serde(default)]
    pub file: Vec<String>,
    /// Templated fallback when neither `value` nor any `file` provides the key.
    #[serde(default)]
    pub default: Option<String>,
    /// When set, `value` is used only while this project id is running;
    /// otherwise `else_value` (if present) is used.
    #[serde(default)]
    pub if_running: Option<String>,
    #[serde(default)]
    pub else_value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Template {
    pub name: String,
    pub projects: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub projects: Vec<String>,
    pub infrastructure: Vec<String>, // Auto-start these too
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MatrixConfig {
    pub projects: Vec<Project>,
    pub templates: Vec<Template>,
    pub groups: Vec<Group>,
    /// First-launch guidance has been shown/completed. Defaults to false, so a
    /// config written before this field existed behaves as a fresh install.
    #[serde(default)]
    pub onboarded: bool,
}

impl MatrixConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: MatrixConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Rewrite relative project and backend paths to absolute against `base`.
    ///
    /// The config lives in a fixed per-device location (`~/.matrix/matrix.json`),
    /// so a relative path must not resolve against wherever `matrix` happens to
    /// be launched from — it always resolves against the config file's directory.
    pub fn normalize_paths(&mut self, base: &Path) {
        let absolutize = |path: &mut String| {
            let p = PathBuf::from(&*path);
            if !p.is_absolute() {
                *path = base.join(p).to_string_lossy().into_owned();
            }
        };
        for project in &mut self.projects {
            absolutize(&mut project.path);
            if let Some(backend) = &mut project.backend {
                absolutize(&mut backend.path);
            }
        }
    }
}

/// Per-device config location: `~/.matrix/matrix.json`.
///
/// Matrix keeps exactly one config per machine (not per folder), so the file
/// lives in the user's home directory rather than the launch directory.
pub fn default_config_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home).join(".matrix").join("matrix.json")
}
mod tests;
