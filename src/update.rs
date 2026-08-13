//! Update checking + self-update for Matrix.
//!
//! The running TUI checks GitHub's `releases/latest` endpoint on startup (in a
//! background thread, non-blocking) and surfaces a small bottom-right modal when
//! a newer *release* exists. `matrix update` performs the actual self-update:
//! it downloads the release binary for the current platform and atomically
//! replaces the running executable.

use serde_json::Value;
use std::io::Write;
use std::time::Duration;

pub const REPO: &str = "ItzJoris03/matrix";

/// Info about the newest published release.
#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    /// Full tag, e.g. `v2026.08.12.3`.
    pub tag: String,
    /// GitHub release page URL (opened with `u`, which never dismisses).
    pub url: String,
}

/// Parse `v2026.08.12.3` (or `2026.08.12.3`) into comparable parts.
/// Returns None for anything that isn't a date-based release tag.
pub fn parse_version(v: &str) -> Option<(u32, u32, u32, u32)> {
    let s = v.trim().trim_start_matches('v');
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let year = parts[0].parse::<u32>().ok()?;
    let month = parts[1].parse::<u32>().ok()?;
    let day = parts[2].parse::<u32>().ok()?;
    let rev = parts[3].parse::<u32>().ok()?;
    Some((year, month, day, rev))
}

/// True when `latest` is strictly newer than `current` (date-scheme compare).
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn api_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

fn user_agent() -> &'static str {
    concat!("matrix/", env!("CARGO_PKG_VERSION"), " (update-check)")
}

/// Fetch the latest release from GitHub. Uses GITHUB_TOKEN when present (needed
/// while the repo is private; harmless once public). Any failure returns None —
/// the TUI must never crash or stall over a network check.
pub fn check_for_update(current: &str) -> Option<ReleaseInfo> {
    let mut req = ureq::get(&api_url())
        .set("User-Agent", user_agent())
        .timeout(Duration::from_secs(6));
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
    }
    let resp = req.call().ok()?;
    let body: Value = resp.into_json().ok()?;
    let tag = body.get("tag_name")?.as_str()?.to_string();
    if !is_newer(&tag, current) {
        return None;
    }
    let url = body
        .get("html_url")
        .and_then(|u| u.as_str())
        .unwrap_or(&format!("https://github.com/{REPO}/releases/tag/{tag}"))
        .to_string();
    Some(ReleaseInfo { tag, url })
}

/// Platform asset name, e.g. `matrix-Linux-x86_64` (matches install.sh).
fn asset_name() -> String {
    let os = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Darwin",
        "windows" => "Windows",
        other => other,
    };
    format!("matrix-{os}-{}", std::env::consts::ARCH)
}

/// Find the download URL for our platform's binary asset in a release's JSON.
/// When a token is present (private repo) prefer the API asset endpoint, which
/// works for both private and public; otherwise use the browser CDN URL.
fn asset_download_url(release: &Value, token: Option<&str>) -> Option<String> {
    let assets = release.get("assets")?.as_array()?;
    let target = asset_name();
    let asset = assets.iter().find(|a| {
        a.get("name")
            .and_then(|n| n.as_str())
            .is_some_and(|n| n == target)
    })?;
    if token.is_some() {
        let id = asset.get("id")?.as_u64()?;
        Some(format!(
            "https://api.github.com/repos/{REPO}/releases/assets/{id}"
        ))
    } else {
        let url = asset.get("browser_download_url")?.as_str()?.to_string();
        Some(url)
    }
}

/// Perform the self-update and return a human-readable outcome message.
/// Never exits the process (unlike the CLI `self_update` wrapper), so the
/// TUI can run it on a background thread and surface the result as a toast.
pub fn perform_update(current: &str) -> Result<String, String> {
    let token = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty());

    let mut req = ureq::get(&api_url())
        .set("User-Agent", user_agent())
        .timeout(Duration::from_secs(10));
    if let Some(t) = &token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let release: Value = match req.call() {
        Ok(r) => r.into_json().unwrap_or(Value::Null),
        Err(ureq::Error::Status(404, _)) => {
            // Repo is private and no (or an invalid) token was given.
            return Err(
                "Could not reach GitHub releases (repo is private?). If the \
                 repository is still private, set GITHUB_TOKEN and try again."
                    .to_string(),
            );
        }
        Err(e) => return Err(format!("Could not reach GitHub: {e}")),
    };

    let tag = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "no release tag found".to_string())?
        .to_string();

    if !is_newer(&tag, current) {
        return Ok(format!("Already up to date (v{current})."));
    }

    let url = asset_download_url(&release, token.as_deref())
        .ok_or_else(|| format!("no prebuilt binary for this platform ({})", asset_name()))?;

    let exe = std::env::current_exe().map_err(|e| format!("cannot locate current exe: {e}"))?;
    let tmp = exe.with_extension("update.tmp");

    let mut dl = ureq::get(&url)
        .set("User-Agent", user_agent())
        .timeout(Duration::from_secs(120));
    if token.is_some() {
        dl = dl.set("Accept", "application/octet-stream");
        if let Some(t) = &token {
            dl = dl.set("Authorization", &format!("Bearer {t}"));
        }
    }
    let resp = dl.call().map_err(|e| format!("download failed: {e}"))?;

    let mut out =
        std::fs::File::create(&tmp).map_err(|e| format!("cannot create temp file: {e}"))?;
    std::io::copy(&mut resp.into_reader(), &mut out).map_err(|e| format!("write failed: {e}"))?;
    out.flush().map_err(|e| format!("flush failed: {e}"))?;
    drop(out);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&tmp)
            .map_err(|e| format!("chmod failed: {e}"))?
            .permissions();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(perms.mode() | 0o111))
            .map_err(|e| format!("chmod failed: {e}"))?;
    }

    std::fs::rename(&tmp, &exe).map_err(|e| format!("replace binary failed: {e}"))?;
    Ok(format!(
        "✔ Updated to {tag}. Restart Matrix to use the new version."
    ))
}

/// CLI entry point for `matrix update`. Prints the result and exits nonzero
/// on failure (a CLI process can afford to die; the TUI cannot).
pub fn self_update(current: &str) -> anyhow::Result<()> {
    match perform_update(current) {
        Ok(msg) => {
            println!("{msg}");
            Ok(())
        }
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_date_tag() {
        assert_eq!(parse_version("v2026.08.12.3"), Some((2026, 8, 12, 3)));
        assert_eq!(parse_version("2026.12.1.0"), Some((2026, 12, 1, 0)));
    }

    #[test]
    fn parse_rejects_non_date() {
        assert_eq!(parse_version("1.1.0"), None);
        assert_eq!(parse_version("v2026.08.12"), None);
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn newer_comparison() {
        assert!(is_newer("v2026.08.12.3", "v2026.08.12.0"));
        assert!(is_newer("v2026.08.13.0", "v2026.08.12.3"));
        assert!(!is_newer("v2026.08.12.0", "v2026.08.12.3"));
        assert!(!is_newer("v2026.08.12.3", "v2026.08.12.3"));
        // Unparseable → never "newer" (fail closed).
        assert!(!is_newer("garbage", "v2026.08.12.3"));
    }
}
