mod app;
mod common;
mod config;
mod detect;
mod engine;
mod features;
mod socket;
mod uninstall;
mod update;
mod url;

use crate::app::App;
use crate::common::ToastEvent;
use crate::config::{default_config_path, MatrixConfig};
use crate::engine::ProcessManager;
use crossterm::{
    event, execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── CLI client mode ──────────────────────────────────────────────────────
    // `matrix <command> [args]` with no TUI: act as a client to the running
    // instance over the existing control socket. `matrix update` self-updates
    // and `matrix uninstall` removes Matrix — both work without a running
    // instance, so they're handled here before the client path.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        if args[0] == "update" {
            return update::self_update(&app::matrix_version_display());
        }
        if args[0] == "uninstall" {
            return uninstall::uninstall();
        }
        return run_client(&args);
    }

    // ── Single-instance guard ─────────────────────────────────────────────────
    // If a live instance already owns the socket, refuse to start a second TUI.
    if socket::is_socket_owned() {
        eprintln!("Matrix is already running.");
        eprintln!("Control the running instance:");
        eprintln!("  matrix status");
        eprintln!("  matrix restart <project_id>");
        eprintln!("  matrix stop <project_id>");
        std::process::exit(0);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Per-device config: `~/.matrix/matrix.json`. One file per machine, read
    // no matter where `matrix` is launched from.
    let config_path = default_config_path();
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut config = MatrixConfig::load(&config_path).unwrap_or_else(|_| MatrixConfig {
        projects: vec![],
        templates: vec![],
        groups: vec![],
    });
    // Relative paths in the config resolve against the config file's directory,
    // so the same file works from any launch location.
    config.normalize_paths(&config_dir);

    let manager = Arc::new(ProcessManager::new(
        config.projects.clone(),
        config.templates.clone(),
        config.groups.clone(),
        config_dir.clone(),
    ));
    // Toast channel: socket commands, TUI actions, and background threads
    // (e.g. self-update) flow through here so results show up live in the TUI.
    let (toast_tx, toast_rx) = mpsc::unbounded_channel::<ToastEvent>();
    let mut app = App::new(
        manager.clone(),
        config_path.to_string_lossy().into_owned(),
        toast_tx.clone(),
    );

    // Background update check: query GitHub once, non-blocking. When a newer
    // release exists the modal appears bottom-right; failures stay silent.
    let (update_tx, mut update_rx) =
        mpsc::unbounded_channel::<Option<crate::update::ReleaseInfo>>();
    {
        let current = app::matrix_version_display();
        std::thread::spawn(move || {
            let info = crate::update::check_for_update(&current);
            let _ = update_tx.send(info);
        });
    }

    let socket_manager = manager.clone();
    let socket_tx = toast_tx.clone();
    tokio::spawn(async move {
        let _ = socket::run_socket_server(socket_manager, socket_tx).await;
    });

    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    let mut last_log_cap = Instant::now();
    let log_cap_interval = Duration::from_secs(30);

    let mut toast_rx = toast_rx;

    loop {
        // Drain any pending toast events into the app (non-blocking).
        while let Ok(ev) = toast_rx.try_recv() {
            app.push_toast(ev);
        }

        // Drain the background update check result (arrives once, at startup).
        while let Ok(info) = update_rx.try_recv() {
            if let Some(info) = info {
                app.update_available = Some(info);
                app.update_dismissed = false;
            }
        }

        terminal.draw(|f| app.render(f))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? && app.handle_event(event::read()?).await? {
            break; // Exit
        }

        if last_tick.elapsed() >= tick_rate {
            app.update_system();
            last_tick = Instant::now();
        }

        // Periodically enforce log file line caps
        if last_log_cap.elapsed() >= log_cap_interval {
            let ids: Vec<String> = {
                let statuses = manager.get_statuses();
                statuses.into_iter().map(|(p, _)| p.id).collect()
            };
            for id in &ids {
                manager.enforce_log_cap(id);
            }
            last_log_cap = Instant::now();
        }
    }

    manager.shutdown().await;
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// CLI client: send the command verbatim to the running instance's socket and
/// print the response. Falls back to a helpful message if no instance is up.
fn run_client(args: &[String]) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let command = args.join(" ");

    let mut stream = match UnixStream::connect(socket::SOCKET_PATH) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("No running Matrix instance. Start one with: matrix");
            std::process::exit(1);
        }
    };

    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.write_all(format!("{}\n", command).as_bytes())?;

    let mut response = String::new();
    // Read lines until the server's --END-- terminator or EOF/timeout.
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.push_str(&String::from_utf8_lossy(&buf[..n]));
                if response.contains("--END--") {
                    break;
                }
            }
            Err(_) => break, // timeout or closed
        }
    }
    let trimmed = response.trim_end().trim_end_matches("--END--").trim_end();
    if !trimmed.is_empty() {
        println!("{}", trimmed);
    }
    Ok(())
}
