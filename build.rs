use std::process::Command;

fn main() {
    // Embed the exact git tag (e.g. v2026.08.12.3) at compile time so the TUI
    // always shows the real release revision. Cargo.toml can only carry a
    // 3-part version (cargo rejects a 4th component), so the per-day revision
    // lives in the git tag — this is the single source of truth.
    let tag = Command::new("git")
        .args(["describe", "--tags", "--exact-match"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=MATRIX_BUILD_VERSION={tag}");

    // Rebuild when the tag moves so the baked version follows releases.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/tags");
    println!("cargo:rerun-if-changed=build.rs");
}
