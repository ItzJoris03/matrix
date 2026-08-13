//! Generic environment resolution for projects.
//!
//! Declarative [`EnvSpec`]s (from config) are turned into concrete
//! `(key, value)` pairs, with `{{placeholder}}` templating in values and
//! defaults. File fallbacks read `.env`-style files relative to the project
//! path. For a virtual backend project the context is the PARENT project:
//! `{{id}}`, `{{path}}` and `{{parent_port}}` refer to the parent, while
//! `{{port}}` is always the port of the project receiving the env.

use crate::config::EnvSpec;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use super::{find_available_port, read_env_var_from_file};

/// Context available to placeholder resolution.
#[derive(Debug, Clone, Default)]
pub struct EnvCtx {
    /// Project id (parent id for backend envs).
    pub id: String,
    /// Absolute project path (parent path for backend envs).
    pub path: String,
    /// Resolved port of the project receiving the env (backend's own port).
    pub port: Option<u16>,
    /// Parent project's resolved port (None for non-backend projects).
    pub parent_port: Option<u16>,
    /// Backend project's resolved port (for parent envs).
    pub backend_port: Option<u16>,
    /// Ports already claimed by other projects (for `{{port+N}}`).
    pub used_ports: Vec<u16>,
    /// Env vars resolved earlier in the same list (`{{env:KEY}}`).
    pub resolved: HashMap<String, String>,
}

/// Generic database-name derivation: split on non-alphanumerics, title-case
/// each segment (first char upper, rest lower), concatenate.
/// e.g. `my-site.example.com` -> `MySiteExampleCom`.
pub fn derive_dbname(id: &str) -> String {
    id.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
            }
        })
        .collect::<String>()
}

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{\{([^{}]+)\}\}").unwrap())
}

/// Resolve a single `{{...}}` placeholder against the context.
/// Returns `None` for unknown placeholders (left as literal text).
fn resolve_placeholder(inner: &str, ctx: &EnvCtx) -> Option<String> {
    match inner {
        "id" => Some(ctx.id.clone()),
        "path" => Some(ctx.path.clone()),
        "port" => ctx.port.map(|p| p.to_string()),
        "parent_port" => ctx.parent_port.map(|p| p.to_string()),
        "backend_port" => ctx.backend_port.map(|p| p.to_string()),
        "dbname" => Some(derive_dbname(&ctx.id)),
        "dbname_upper" => Some(derive_dbname(&ctx.id).to_uppercase()),
        _ => {
            if let Some(key) = inner.strip_prefix("env:") {
                return ctx.resolved.get(key).cloned();
            }
            if let Some((key, default)) = inner.split_once('|') {
                if key == "parent_port" {
                    return ctx
                        .parent_port
                        .map(|p| p.to_string())
                        .or_else(|| Some(default.to_string()));
                }
            }
            if let Some((port_part, delta)) = inner.split_once('+') {
                if port_part == "port" {
                    if let (Some(base), Ok(delta)) = (ctx.port, delta.parse::<u16>()) {
                        if let Some(start) = base.checked_add(delta) {
                            return Some(find_available_port(start, &ctx.used_ports).to_string());
                        }
                    }
                }
            }
            None
        }
    }
}

/// Resolve `{{placeholders}}` in a template string. Unknown placeholders are
/// left as literal text so a typo never silently becomes an empty string.
pub fn resolve_template(tpl: &str, ctx: &EnvCtx) -> String {
    placeholder_re()
        .replace_all(tpl, |caps: &regex::Captures| {
            resolve_placeholder(&caps[1], ctx).unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
}

/// Resolve an env spec list into concrete (key, value) pairs.
///
/// Resolution order per spec: explicit `value` (or `else_value` when
/// `if_running` is not running) → first matching key in the `file` list →
/// templated `default`. `{{env:KEY}}` in later specs can reference earlier
/// resolved entries.
pub fn resolve_env(
    specs: &[EnvSpec],
    ctx: &EnvCtx,
    file_base: &Path,
    is_running: impl Fn(&str) -> bool,
) -> HashMap<String, String> {
    let mut resolved = ctx.resolved.clone();
    let mut out = HashMap::new();

    for spec in specs {
        let mut c = ctx.clone();
        c.resolved = resolved.clone();

        let value = if let Some(proj) = &spec.if_running {
            if is_running(proj) {
                spec.value.clone()
            } else {
                spec.else_value.clone()
            }
        } else {
            spec.value.clone()
        };

        let value = match value {
            Some(v) => Some(resolve_template(&v, &c)),
            None => {
                let mut from_file = None;
                for f in &spec.file {
                    if let Some(v) = read_env_var_from_file(&file_base.join(f), &spec.key) {
                        from_file = Some(v);
                        break;
                    }
                }
                from_file
            }
        };

        let value = match value {
            Some(v) => Some(v),
            None => spec.default.as_ref().map(|d| resolve_template(d, &c)),
        };

        if let Some(v) = value {
            resolved.insert(spec.key.clone(), v.clone());
            out.insert(spec.key.clone(), v);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EnvSpec;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    fn ctx() -> EnvCtx {
        let mut resolved = HashMap::new();
        resolved.insert(
            "MONGODB_URI".to_string(),
            "mongodb://127.0.0.1:27017/X".to_string(),
        );
        EnvCtx {
            id: "my-site.example.com".to_string(),
            path: "/abs/site".to_string(),
            port: Some(3000),
            parent_port: Some(5173),
            backend_port: Some(3001),
            used_ports: vec![],
            resolved,
        }
    }

    #[test]
    fn derive_dbname_is_generic_title_case() {
        assert_eq!(derive_dbname("my-site.example.com"), "MySiteExampleCom");
        assert_eq!(derive_dbname("shop.example.net"), "ShopExampleNet");
        assert_eq!(derive_dbname("PosSystem"), "Possystem");
        assert_eq!(derive_dbname("a-b_c"), "ABC");
    }

    #[test]
    fn resolve_basic_placeholders() {
        let c = ctx();
        assert_eq!(resolve_template("{{id}}", &c), "my-site.example.com");
        assert_eq!(resolve_template("{{path}}", &c), "/abs/site");
        assert_eq!(resolve_template("{{port}}", &c), "3000");
        assert_eq!(resolve_template("{{parent_port}}", &c), "5173");
        assert_eq!(resolve_template("{{backend_port}}", &c), "3001");
    }

    #[test]
    fn resolve_dbname_placeholders() {
        let c = ctx();
        assert_eq!(resolve_template("{{dbname}}", &c), "MySiteExampleCom");
        assert_eq!(
            resolve_template("{{dbname_upper}}_SECRET", &c),
            "MYSITEEXAMPLECOM_SECRET"
        );
    }

    #[test]
    fn parent_port_default_used_when_missing() {
        let mut c = ctx();
        c.parent_port = None;
        assert_eq!(
            resolve_template("http://localhost:{{parent_port|3000}}", &c),
            "http://localhost:3000"
        );
        c.parent_port = Some(5173);
        assert_eq!(
            resolve_template("http://localhost:{{parent_port|3000}}", &c),
            "http://localhost:5173"
        );
    }

    #[test]
    fn port_plus_delta_allocates_next_free_port() {
        let c = ctx();
        assert_eq!(resolve_template("{{port+5}}", &c), "3005");
        // When the target is taken, find_available_port walks upward.
        let mut c2 = ctx();
        c2.used_ports = vec![3005];
        assert_eq!(resolve_template("{{port+5}}", &c2), "3006");
    }

    #[test]
    fn env_reference_uses_earlier_resolved_entry() {
        let c = ctx();
        assert_eq!(
            resolve_template("{{env:MONGODB_URI}}", &c),
            "mongodb://127.0.0.1:27017/X"
        );
    }

    #[test]
    fn unknown_placeholder_stays_literal() {
        let c = ctx();
        assert_eq!(resolve_template("a{{bogus}}b", &c), "a{{bogus}}b");
        assert_eq!(resolve_template("no placeholders", &c), "no placeholders");
    }

    #[test]
    fn resolve_env_value_template_and_default() {
        let dir = PathBuf::from("/tmp/matrix-env-test-value");
        let _ = fs::create_dir_all(&dir);
        let specs = vec![
            EnvSpec {
                key: "SITE_URL".to_string(),
                value: Some("http://localhost:{{parent_port|3000}}".to_string()),
                file: vec![],
                default: None,
                if_running: None,
                else_value: None,
            },
            EnvSpec {
                key: "JWT_SECRET".to_string(),
                value: None,
                file: vec![],
                default: Some("{{dbname_upper}}_SECRET".to_string()),
                if_running: None,
                else_value: None,
            },
        ];
        let out = resolve_env(&specs, &ctx(), &dir, |_| false);
        assert_eq!(out.get("SITE_URL").unwrap(), "http://localhost:5173");
        assert_eq!(out.get("JWT_SECRET").unwrap(), "MYSITEEXAMPLECOM_SECRET");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_env_file_fallback_reads_dotenv() {
        let dir = PathBuf::from("/tmp/matrix-env-test-file");
        let _ = fs::create_dir_all(&dir);
        fs::write(
            dir.join(".env"),
            "MONGODB_URI=mongodb://127.0.0.1:27017/FromFile\n# comment\n",
        )
        .unwrap();
        let specs = vec![EnvSpec {
            key: "MONGODB_URI".to_string(),
            value: None,
            file: vec![".env".to_string()],
            default: Some("should-not-fire".to_string()),
            if_running: None,
            else_value: None,
        }];
        let out = resolve_env(&specs, &ctx(), &dir, |_| false);
        assert_eq!(
            out.get("MONGODB_URI").unwrap(),
            "mongodb://127.0.0.1:27017/FromFile"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_env_if_running_conditional() {
        let dir = PathBuf::from("/tmp/matrix-env-test-cond");
        let _ = fs::create_dir_all(&dir);
        let specs = vec![EnvSpec {
            key: "PLUGIN_MANAGER_URL".to_string(),
            value: Some("http://localhost:3002".to_string()),
            file: vec![],
            default: None,
            if_running: Some("plugin-manager".to_string()),
            else_value: Some("https://plugins.example.com/".to_string()),
        }];
        let out = resolve_env(&specs, &ctx(), &dir, |id| id == "plugin-manager");
        assert_eq!(
            out.get("PLUGIN_MANAGER_URL").unwrap(),
            "http://localhost:3002"
        );
        let out = resolve_env(&specs, &ctx(), &dir, |_| false);
        assert_eq!(
            out.get("PLUGIN_MANAGER_URL").unwrap(),
            "https://plugins.example.com/"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
