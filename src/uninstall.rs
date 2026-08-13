//! Self-uninstall for Matrix.
//!
//! `matrix uninstall` removes the binary and asks an explicit question:
//! wipe everything (binary + `~/.matrix` config) or remove only the TUI
//! binary and keep the config. It runs in CLI mode without a TUI and works
//! without a running instance, exactly like `matrix update`.

use crate::config::default_config_path;
use crate::socket::{is_socket_owned, SOCKET_PATH};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

/// Parse the uninstall choice. `1` = everything (binary + config),
/// `2` = only the binary. Empty input defaults to 2 — config deletion is
/// never implicit.
fn parse_choice(input: &str) -> Option<u8> {
    match input.trim().to_ascii_lowercase().as_str() {
        "" | "2" | "only" | "binary" => Some(2),
        "1" | "everything" | "all" | "full" | "wipe" => Some(1),
        _ => None,
    }
}

pub fn uninstall() -> anyhow::Result<()> {
    if is_socket_owned() {
        anyhow::bail!(
            "Matrix is currently running — quit it first, then run `matrix uninstall` again."
        );
    }

    let exe = std::env::current_exe()?;
    let config_dir = default_config_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    println!("Matrix uninstall");
    println!("  binary: {}", exe.display());
    println!("  config: {}", config_dir.display());
    println!();
    println!("What should I remove?");
    println!("  1) Everything — Matrix AND configuration");
    println!("  2) Only Matrix — keep configuration");

    // Ask first, act after. Empty/EOF keeps config (safe default).
    let choice = loop {
        print!("Choice [1/2]: ");
        io::stdout().flush()?;
        let mut answer = String::new();
        if io::stdin().lock().read_line(&mut answer)? == 0 {
            break 2; // EOF (piped input) — keep config
        }
        if let Some(c) = parse_choice(&answer) {
            break c;
        }
        println!("  (enter 1 or 2)");
    };

    // Remove the running binary, plus the canonical install location when it
    // differs (e.g. running from a cargo build dir while an older copy still
    // sits on PATH).
    let mut paths = vec![exe];
    if let Ok(home) = std::env::var("HOME") {
        let installed = PathBuf::from(home).join(".local/bin/matrix");
        if installed != paths[0] && installed.exists() {
            paths.push(installed);
        }
    }
    for p in &paths {
        match std::fs::remove_file(p) {
            Ok(()) => println!("  removed {}", p.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => anyhow::bail!("could not remove {}: {e}", p.display()),
        }
    }

    // Stale socket from a previous instance (no live instance owns it now).
    if let Err(e) = std::fs::remove_file(SOCKET_PATH) {
        if e.kind() != io::ErrorKind::NotFound {
            println!("  (could not remove stale socket {}: {e})", SOCKET_PATH);
        }
    }

    match choice {
        1 => match std::fs::remove_dir_all(&config_dir) {
            Ok(()) => println!("  removed {}", config_dir.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => println!("  (could not remove {}: {e})", config_dir.display()),
        },
        _ => println!("  kept {}", config_dir.display()),
    }

    println!("Matrix uninstalled.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_choice;

    #[test]
    fn choice_one_wipes_everything() {
        assert_eq!(parse_choice("1"), Some(1));
        assert_eq!(parse_choice("everything"), Some(1));
        assert_eq!(parse_choice(" WIPE \n"), Some(1));
    }

    #[test]
    fn choice_two_keeps_config() {
        assert_eq!(parse_choice("2"), Some(2));
        assert_eq!(parse_choice("only"), Some(2));
        assert_eq!(parse_choice("binary"), Some(2));
    }

    #[test]
    fn empty_defaults_to_keeping_config() {
        assert_eq!(parse_choice(""), Some(2));
        assert_eq!(parse_choice("   \n"), Some(2));
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse_choice("maybe"), None);
        assert_eq!(parse_choice("3"), None);
    }
}
