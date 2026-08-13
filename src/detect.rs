//! Filesystem project detection for the "detect" command.
//!
//! Scans a set of curated roots (not the literal filesystem root — that would
//! hang on /proc, /sys, and mounts) for directories that look like runnable
//! projects, based on the presence of a manifest file. Returns candidates the
//! user can add to Matrix directly from a modal.
//!
//! Design notes / heuristics (tuned against a real pnpm monorepo layout):
//!
//! * A directory that is a **workspace root** (has `pnpm-workspace.yaml`, a
//!   `workspaces` field in package.json, or a monorepo tool config like
//!   `turbo.json`/`lerna.json`/`nx.json`) is NOT itself a project. Its real
//!   runnable leaf apps live in `apps/`, `services/`, `platform/`, etc. — so we
//!   descend into those and never report the root.
//! * `packages/`, `libs/`, `@scope/...`, `plugin-*`, `ui`, `shared`, `common`
//!   are almost always libraries, not runnable apps — skipped as leaf projects
//!   so we don't surface 20 plugin-* dirs from a monorepo.
//! * Version / snapshot folders: `v2_backup`, `old`, `archive`, ... are skipped
//!   entirely. For bare `vN` folders (v1, v2, v3, ...), only the *newest* in a
//!   sibling set is kept — the older ones are historical copies, not live
//!   distinct projects. This keeps `v4` (current) while dropping `v1..v3`.
//! * Dependency / build caches (`node_modules`, `target`, `dist`, ...) are
//!   never descended into.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DetectCandidate {
    pub id: String,
    pub name: String,
    pub path: String,
    /// Best-guess launch command (empty string if unknown).
    pub command: String,
    /// Category: "Rust", "Python", "Go", "HTML", or a Node flavor
    /// ("React", "Vue", "Svelte", "Next", "Vite", "Node", ...).
    pub category: String,
}

#[derive(Clone, Debug)]
pub enum DetectAction {
    None,
    Close,
    Add(DetectCandidate),
}

/// Scan the user's home directory itself so projects are found anywhere in
/// user space; noise (GOPATH, SDKs, caches) is pruned by `is_skipped`.
pub fn default_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home = PathBuf::from(home);
    if home.exists() {
        vec![home]
    } else {
        Vec::new()
    }
}

/// Scan the given roots and return detected project candidates.
pub fn scan_projects(roots: &[PathBuf]) -> Vec<DetectCandidate> {
    let mut out: Vec<DetectCandidate> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for root in roots {
        if let Ok(canon) = std::fs::canonicalize(root) {
            // Depth 8 from ~ reaches `~/Documents/Projects/JHITS/v4/apps/my-app`
            // (home→Documents→Projects→JHITS→v4→apps→my-app = depth 6) with
            // headroom for deeper layouts, while still bounding the walk.
            scan_dir(&canon, 0, 8, &mut out, &mut seen);
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn scan_dir(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<DetectCandidate>,
    seen: &mut HashSet<PathBuf>,
) {
    if depth > max_depth {
        return;
    }

    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if is_skipped(name) {
        return;
    }

    // A workspace root is not itself a project — descend into its runnable
    // member directories instead.
    if is_workspace_root(dir) {
        for member in runnable_member_dirs(dir) {
            scan_dir(&member, depth + 1, max_depth, out, seen);
        }
        return;
    }

    let manifest = detect_manifest(dir);
    if let Some((category, command)) = manifest {
        // Don't surface library packages as runnable projects.
        if dir.join("package.json").exists() && is_library_leaf(dir) {
            return;
        }
        if let Ok(canon) = std::fs::canonicalize(dir) {
            if seen.insert(canon.clone()) {
                let base = name.replace(' ', "-");
                out.push(DetectCandidate {
                    id: make_id(&base, out),
                    name: base.clone(),
                    path: canon.to_string_lossy().to_string(),
                    command,
                    category: category.to_string(),
                });
            }
        }
        // Don't descend into a detected project dir.
        return;
    }

    if depth < max_depth {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if let Some(child) = p.file_name().and_then(|n| n.to_str()) {
                        if is_skipped(child) {
                            continue;
                        }
                        if is_old_version(child, dir) {
                            continue;
                        }
                        scan_dir(&p, depth + 1, max_depth, out, seen);
                    }
                }
            }
        }
    }
}

/// Directory names we never descend into (dependency/build caches, VCS, OS
/// media folders, and version/snapshot/backup folders).
fn is_skipped(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    if name.starts_with('_') {
        return true; // underscore-prefixed: build output (_site), old copies (_OLD)
    }
    if is_snapshot(name) {
        return true;
    }
    if is_versioned_dir(name) {
        return true;
    }
    if name.ends_with("-build") || name.ends_with("_build") {
        return true; // build artifact dirs (ggml-edgetpu-build, foo_build)
    }
    matches!(
        name,
        "node_modules"
            | "target"
            | "dist"
            | "build"
            | "out"
            | "venv"
            | ".venv"
            | "coverage"
            | "__pycache__"
            | ".idea"
            | ".vscode"
            | ".cache"
            | ".cargo"
            | ".rustup"
            | "Library"
            | "Downloads"
            | "Pictures"
            | "Music"
            | "Videos"
            | "Movies"
            | ".npm"
            | ".pnpm-store"
            | "packages_old"
            // Home-level toolchains / SDKs: huge trees of cache and deps that
            // are never runnable projects themselves.
            | "go"
            | "gopath"
            | "android-sdk"
            | "Android"
            | "Applications"
            | "AppData"
            | "snap"
            | "flatpak"
            | "Sdk"
            | "sdk"
            | "rustup"
            | "cargo"
    )
}

/// Version / snapshot / archive folder names: historical copies, not live
/// distinct projects. Bare `vN` folders are handled by `is_old_version`
/// (only the newest `vN` in a sibling set is kept).
fn is_snapshot(name: &str) -> bool {
    if name.ends_with("_backup")
        || name.ends_with("-backup")
        || name.ends_with(".bak")
        || name.ends_with("_old")
        || name.ends_with("_archived")
        || name.ends_with("_archive")
    {
        return true;
    }
    matches!(
        name,
        "backup" | "old" | "archive" | "archived" | "snapshot" | "tmp" | "temp"
    )
}

/// True when the name looks like `foo-2.1.0` — a versioned source snapshot
/// (AUR checkouts, downloaded tarball dirs) rather than a live project.
fn is_versioned_dir(name: &str) -> bool {
    let Some((_, ver)) = name.rsplit_once('-') else {
        return false;
    };
    let parts: Vec<&str> = ver.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// A `vN` folder is an *old* version (a snapshot to skip) when a higher-numbered
/// sibling `vM` exists in the same parent — only the newest `vN` is the live
/// project. Returns false for the highest `vN`, and for any non-`vN` name.
fn is_old_version(name: &str, parent: &Path) -> bool {
    let lower = name.to_lowercase();
    let n = match lower.strip_prefix('v') {
        Some(rest) => {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() || rest != digits.as_str() {
                return false;
            }
            digits.parse::<u32>().unwrap_or(0)
        }
        None => return false,
    };

    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let sib = match p.file_name().and_then(|f| f.to_str()) {
                Some(s) => s.to_lowercase(),
                None => continue,
            };
            if let Some(rest) = sib.strip_prefix('v') {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if rest == digits.as_str() && !digits.is_empty() {
                    if let Ok(m) = digits.parse::<u32>() {
                        if m > n {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// True if `dir` is a monorepo / workspace root.
fn is_workspace_root(dir: &Path) -> bool {
    if dir.join("pnpm-workspace.yaml").exists() {
        return true;
    }
    if dir.join("turbo.json").exists()
        || dir.join("lerna.json").exists()
        || dir.join("nx.json").exists()
    {
        return true;
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(ws) = json.get("workspaces") {
                match ws {
                    serde_json::Value::Array(a) => {
                        if !a.is_empty() {
                            return true;
                        }
                    }
                    serde_json::Value::Object(o) if !o.is_empty() => {
                        return true;
                    }
                    _ => {}
                }
            }
        }
    }
    false
}

/// Library directory base names — never treated as runnable member apps.
const LIBRARY_BASES: &[&str] = &[
    "packages",
    "libs",
    "lib",
    "node_modules",
    "shared",
    "common",
    "src",
    "internal",
];

/// Given a workspace root, return the member directories that are likely to
/// contain runnable apps (apps/, services/, platform/, web/, ...), skipping
/// library containers (packages/, libs/).
fn runnable_member_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut bases: Vec<String> = Vec::new();

    if let Ok(content) = std::fs::read_to_string(dir.join("pnpm-workspace.yaml")) {
        for line in content.lines() {
            let line = line.trim();
            if line.contains('*') {
                // e.g. 'apps/*' or "packages/*" -> base "apps"/"packages"
                let base = line
                    .trim_matches(|c| c == '\'' || c == '"' || c == '-' || c == ' ')
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !base.is_empty() {
                    bases.push(base);
                }
            }
        }
    }

    for conventional in [
        "apps", "services", "platform", "web", "frontend", "backend", "mobile", "tools", "sites",
    ] {
        if !bases.iter().any(|b| b == conventional) {
            bases.push(conventional.to_string());
        }
    }

    bases
        .into_iter()
        .filter(|b| !LIBRARY_BASES.contains(&b.as_str()))
        .map(|b| dir.join(b))
        .filter(|p| p.is_dir())
        .collect()
}

/// A Web directory that is actually a library, not a runnable app.
fn is_library_leaf(dir: &Path) -> bool {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with("plugin-")
        || name.starts_with('@')
        || name.starts_with("lib")
        || name.starts_with("ui")
        || name.starts_with("shared")
        || name.starts_with("common")
        || name.starts_with("core")
    {
        return true;
    }
    // Skip if any ancestor segment is a library container.
    dir.components().any(|c| {
        if let std::path::Component::Normal(s) = c {
            if let Some(s) = s.to_str() {
                return LIBRARY_BASES.contains(&s);
            }
        }
        false
    })
}

/// Classify a Node project from its package.json dependencies: a web
/// framework (React, Vue, Svelte, Next, ...) when present, "Vite" for
/// Vite-only tooling, "HTML" for plain web pages with no framework, and
/// "Node" for standalone node projects (servers, CLIs, scripts).
fn detect_node_category(dir: &Path) -> &'static str {
    let pkg = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());
    let deps = pkg
        .as_ref()
        .and_then(|j| j.get("dependencies"))
        .and_then(|d| d.as_object());
    let dev_deps = pkg
        .as_ref()
        .and_then(|j| j.get("devDependencies"))
        .and_then(|d| d.as_object());
    let has_dep = |name: &str| {
        deps.is_some_and(|d| d.contains_key(name)) || dev_deps.is_some_and(|d| d.contains_key(name))
    };

    // Framework markers, most specific first (Next is also a React app).
    const FRAMEWORKS: &[(&str, &str)] = &[
        ("next", "Next"),
        ("nuxt", "Nuxt"),
        ("sveltekit", "Svelte"),
        ("astro", "Astro"),
        ("gatsby", "Gatsby"),
        ("react", "React"),
        ("vue", "Vue"),
        ("svelte", "Svelte"),
        ("angular", "Angular"),
        ("preact", "Preact"),
        ("solid-js", "Solid"),
        ("qwik", "Qwik"),
    ];
    for (pkg_name, label) in FRAMEWORKS {
        if has_dep(pkg_name) {
            return label;
        }
    }
    if has_dep("vite") {
        return "Vite";
    }
    if dir.join("index.html").exists() {
        "HTML"
    } else {
        "Node"
    }
}

/// Detect a manifest and return (category, launch_command).
fn detect_manifest(dir: &Path) -> Option<(&'static str, String)> {
    if dir.join("package.json").exists() {
        Some((detect_node_category(dir), detect_node_cmd(dir)))
    } else if dir.join("Cargo.toml").exists() {
        Some(("Rust", "cargo run".to_string()))
    } else if dir.join("pyproject.toml").exists()
        || dir.join("requirements.txt").exists()
        || dir.join("setup.py").exists()
    {
        Some(("Python", detect_py_cmd(dir)))
    } else if dir.join("go.mod").exists() {
        Some(("Go", "go run .".to_string()))
    } else if dir.join("index.html").exists() {
        Some(("HTML", String::new()))
    } else {
        None
    }
}

/// Pick a package manager + a *self-healing* launch command for a Node project.
///
/// The command is built so it works even when dependencies are not yet
/// installed (the common "it failed because I never ran install" case):
///   * detect pnpm/yarn by a lockfile in this dir *or any ancestor* — a member
///     app in a pnpm workspace usually has no lockfile of its own, but the
///     workspace root does;
///   * prepend `if [ ! -d node_modules ]; then <pm> install; fi`;
///   * if the `dev` script uses `concurrently`/`npm-run-all` but the package
///     does not declare it, also `npm install --no-save` it;
///   * if any script uses `npm --prefix ./DIR`, also install in those dirs
///     (monorepos that aren't declared as workspaces, e.g. LFMX).
///
/// npm only special-cases `start`/`test`/`stop`/`restart` (those work without
/// `run`); `dev` does NOT — so we always use `run`.
fn detect_node_cmd(dir: &Path) -> String {
    let mut probe = Some(dir.to_path_buf());
    let mut pm = "npm";
    while let Some(p) = probe {
        if p.join("pnpm-lock.yaml").exists() {
            pm = "pnpm";
            break;
        } else if p.join("yarn.lock").exists() {
            pm = "yarn";
            break;
        }
        probe = p.parent().map(|x| x.to_path_buf());
    }

    let pkg = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());

    let scripts = pkg
        .as_ref()
        .and_then(|j| j.get("scripts"))
        .and_then(|s| s.as_object());
    let deps = pkg
        .as_ref()
        .and_then(|j| j.get("dependencies"))
        .and_then(|d| d.as_object());
    let dev_deps = pkg
        .as_ref()
        .and_then(|j| j.get("devDependencies"))
        .and_then(|d| d.as_object());
    let has_dep = |name: &str| {
        deps.is_some_and(|d| d.contains_key(name)) || dev_deps.is_some_and(|d| d.contains_key(name))
    };

    let mut pre: Vec<String> = Vec::new();
    pre.push(format!("if [ ! -d node_modules ]; then {} install; fi", pm));

    // Does any script use a runner that may be undeclared?
    let uses_runner = |runner: &str| -> bool {
        scripts.is_some_and(|s| {
            s.get("dev")
                .and_then(|v| v.as_str())
                .is_some_and(|d| d.contains(runner))
                || s.values()
                    .any(|v| v.as_str().is_some_and(|x| x.contains(runner)))
        })
    };

    if uses_runner("concurrently") && !has_dep("concurrently") && pm == "npm" {
        pre.push(
            "if [ ! -x node_modules/.bin/concurrently ]; then npm install --no-save concurrently; fi"
                .to_string(),
        );
    }
    if uses_runner("npm-run-all") && !has_dep("npm-run-all") && pm == "npm" {
        pre.push(
            "if [ ! -x node_modules/.bin/npm-run-all ]; then npm install --no-save npm-run-all; fi"
                .to_string(),
        );
    }

    // `npm --prefix ./DIR ...` referenced anywhere -> install in those dirs.
    if let Some(scripts) = scripts {
        let mut dirs: Vec<String> = Vec::new();
        for v in scripts.values() {
            if let Some(s) = v.as_str() {
                for seg in s.split("--prefix") {
                    let token = seg
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .trim_matches(|c| c == '\'' || c == '"');
                    if token.is_empty() {
                        continue;
                    }
                    if dir.join(token).join("package.json").exists()
                        && !dirs.iter().any(|d| d == token)
                    {
                        dirs.push(token.to_string());
                    }
                }
            }
        }
        for d in dirs {
            pre.push(format!(
                "(cd {} && if [ ! -d node_modules ]; then {} install; fi)",
                d, pm
            ));
        }
    }

    let run = if scripts.is_some_and(|s| s.contains_key("dev")) {
        format!("{} run dev", pm)
    } else if scripts.is_some_and(|s| s.contains_key("start")) {
        format!("{} run start", pm)
    } else {
        format!("{} run dev", pm)
    };

    if pre.is_empty() {
        run
    } else {
        format!("{} && {}", pre.join(" && "), run)
    }
}

/// Guess a launch command for a Python project.
fn detect_py_cmd(dir: &Path) -> String {
    if dir.join("manage.py").exists() {
        "python manage.py runserver".to_string()
    } else if dir.join("main.py").exists() {
        "python main.py".to_string()
    } else if dir.join("app.py").exists() {
        "python app.py".to_string()
    } else {
        "python -m flask run".to_string()
    }
}

/// Ensure candidate IDs are unique within the detected set.
fn make_id(base: &str, out: &[DetectCandidate]) -> String {
    let mut id = base.to_string();
    let mut n = 2;
    while out.iter().any(|c| c.id == id) {
        id = format!("{}-{}", base, n);
        n += 1;
    }
    id
}

pub struct DetectController;

impl DetectController {
    pub fn handle_key(
        key: crossterm::event::KeyCode,
        selected: &mut usize,
        candidates: &mut [DetectCandidate],
    ) -> DetectAction {
        if candidates.is_empty() {
            return DetectAction::Close;
        }

        match key {
            crossterm::event::KeyCode::Esc => DetectAction::Close,
            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                if *selected > 0 {
                    *selected -= 1;
                } else {
                    *selected = candidates.len() - 1;
                }
                DetectAction::None
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                if *selected + 1 < candidates.len() {
                    *selected += 1;
                } else {
                    *selected = 0;
                }
                DetectAction::None
            }
            crossterm::event::KeyCode::Enter => {
                if let Some(c) = candidates.get(*selected).cloned() {
                    DetectAction::Add(c)
                } else {
                    DetectAction::None
                }
            }
            _ => DetectAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny fixture tree and assert the detector filters out old
    /// version snapshots, library packages, and node_modules while still
    /// finding the runnable apps (and keeping the NEWEST vN snapshot, which
    /// is the live project). Run with: cargo test -- --nocapture detect
    #[test]
    fn scan_fixture_tree_filters_snapshots_and_libs() {
        use std::fs;

        let fixture_dir =
            std::env::temp_dir().join(format!("matrix-detect-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&fixture_dir);

        // app1, app2: runnable apps that must be detected (app2 also carries a
        // node_modules cache that must never be descended into).
        // v1: old version snapshot; filtered because the newer sibling v2 exists.
        // v2: newest version snapshot; the live project, so it IS kept.
        // packages/lib: a library package; filtered out as a library leaf.
        for d in [
            "app1",
            "app2",
            "v1",
            "v2",
            "packages/lib",
            "app2/node_modules/x",
        ] {
            fs::create_dir_all(fixture_dir.join(d)).unwrap();
            fs::write(
                fixture_dir.join(d).join("package.json"),
                "{\"scripts\":{\"dev\":\"vite\"}}\n",
            )
            .unwrap();
        }

        let found = scan_projects(std::slice::from_ref(&fixture_dir));

        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"app1") && names.contains(&"app2"),
            "runnable apps missing from results: {:?}",
            names
        );
        assert!(
            names.contains(&"v2"),
            "newest vN snapshot should be kept as the live project: {:?}",
            names
        );

        for c in &found {
            assert!(
                !c.path.contains("v1"),
                "old version snapshot leaked into results: {}",
                c.path
            );
            assert!(
                !c.path.contains("packages"),
                "library package leaked into results: {}",
                c.path
            );
            assert!(
                !c.path.contains("node_modules"),
                "node_modules leaked into results: {}",
                c.path
            );
        }

        let _ = fs::remove_dir_all(&fixture_dir);
    }

    /// Home-wide scanning must prune the noise a full `~` walk encounters:
    /// versioned source snapshots (`foo-2.1.0`), underscore-prefixed junk
    /// (`_OLD`), and toolchain caches (`gopath`, `go`) — while still finding
    /// a real project nested deep in a workspace (the depth-8 bound).
    #[test]
    fn home_wide_scan_prunes_noise_and_finds_deep_projects() {
        use std::fs;

        let fixture_dir =
            std::env::temp_dir().join(format!("matrix-detect-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&fixture_dir);

        for d in [
            "real-app",                      // live project at top level
            "paru-2.1.0",                    // versioned snapshot -> skip
            "_OLD",                          // underscore junk -> skip
            "gopath/pkg/mod/github.com/x/y", // Go module cache -> skip at gopath
            "go/pkg/mod/github.com/x/y",     // GOPATH cache -> skip at go
            "monorepo/apps/deep-app",        // deep workspace member (depth 5)
        ] {
            fs::create_dir_all(fixture_dir.join(d)).unwrap();
            fs::write(
                fixture_dir.join(d).join("package.json"),
                "{\"scripts\":{\"dev\":\"vite\"}}\n",
            )
            .unwrap();
        }

        let found = scan_projects(std::slice::from_ref(&fixture_dir));
        let paths: Vec<&str> = found.iter().map(|c| c.path.as_str()).collect();

        assert!(
            found.iter().any(|c| c.name == "real-app"),
            "top-level project missing: {:?}",
            paths
        );
        assert!(
            found.iter().any(|c| c.name == "deep-app"),
            "deep workspace member missing (depth bound too tight?): {:?}",
            paths
        );

        for bad in ["paru-2.1.0", "_OLD", "gopath", "go/pkg"] {
            assert!(
                !found.iter().any(|c| c.path.contains(bad)),
                "'{bad}' leaked into results: {:?}",
                paths
            );
        }

        let _ = fs::remove_dir_all(&fixture_dir);
    }

    #[test]
    fn versioned_dir_names_are_detected() {
        assert!(is_versioned_dir("paru-2.1.0"));
        assert!(is_versioned_dir("app-1.2"));
        assert!(!is_versioned_dir("real-app"));
        assert!(!is_versioned_dir("my-project"));
    }
}
