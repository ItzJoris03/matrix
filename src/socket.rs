use crate::app::commands::COMMAND_TEMPLATES;
use crate::common::{ToastEvent, ToastTone};
use crate::engine::ProcessManager;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc::UnboundedSender;

pub const SOCKET_PATH: &str = "/tmp/matrix-control.sock";

/// Returns true if a live Matrix instance already owns the control socket.
pub fn is_socket_owned() -> bool {
    use std::os::unix::net::UnixStream;
    UnixStream::connect(SOCKET_PATH).is_ok()
}

pub async fn run_socket_server(
    manager: Arc<ProcessManager>,
    toast_tx: UnboundedSender<ToastEvent>,
) -> anyhow::Result<()> {
    // Clean up stale socket
    let _ = tokio::fs::remove_file(SOCKET_PATH).await;

    let listener = UnixListener::bind(SOCKET_PATH)?;

    loop {
        let (stream, _) = listener.accept().await?;
        let manager = manager.clone();
        let toast_tx = toast_tx.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let cmd = line.trim();
                        if cmd.is_empty() {
                            continue;
                        }

                        let response = handle_socket_command(cmd, &manager, &toast_tx).await;
                        let _ = writer.write_all(response.as_bytes()).await;
                        let _ = writer.write_all(b"\n").await;
                        // Terminate the response so clients can stop reading
                        // without waiting for EOF (the connection stays open
                        // to accept further commands).
                        let _ = writer.write_all(b"--END--\n").await;
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

fn emit(tx: &UnboundedSender<ToastEvent>, source: &str, message: String, tone: ToastTone) {
    let _ = tx.send(ToastEvent {
        source: source.to_string(),
        message,
        tone,
    });
}

async fn handle_socket_command(
    cmd: &str,
    manager: &ProcessManager,
    toast_tx: &UnboundedSender<ToastEvent>,
) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return "ERROR: empty command".to_string();
    }

    match parts[0] {
        "start" => {
            if parts.len() < 2 {
                return "ERROR: usage: start <project_id>".to_string();
            }
            let id = parts[1];
            match manager.start(id) {
                Ok(_) => {
                    emit(
                        toast_tx,
                        "socket",
                        format!("started {}", id),
                        ToastTone::Success,
                    );
                    format!("OK: started {}", id)
                }
                Err(e) => {
                    emit(
                        toast_tx,
                        "socket",
                        format!("failed to start {}: {}", id, e),
                        ToastTone::Error,
                    );
                    format!("ERROR: failed to start {}: {}", id, e)
                }
            }
        }
        "stop" => {
            if parts.len() < 2 {
                return "ERROR: usage: stop <project_id>".to_string();
            }
            let id = parts[1];
            let _ = manager.stop(id).await;
            emit(
                toast_tx,
                "socket",
                format!("stopped {}", id),
                ToastTone::Success,
            );
            format!("OK: stopped {}", id)
        }
        "restart" => {
            if parts.len() < 2 {
                return "ERROR: usage: restart <project_id>".to_string();
            }
            let id = parts[1];
            let _ = manager.stop(id).await;
            match manager.start(id) {
                Ok(_) => {
                    emit(
                        toast_tx,
                        "socket",
                        format!("restarted {}", id),
                        ToastTone::Success,
                    );
                    format!("OK: restarted {}", id)
                }
                Err(e) => {
                    emit(
                        toast_tx,
                        "socket",
                        format!("failed to restart {}: {}", id, e),
                        ToastTone::Error,
                    );
                    format!("ERROR: failed to restart {}: {}", id, e)
                }
            }
        }
        "status" => {
            let statuses = manager.get_statuses();
            let mut lines = Vec::new();
            for (config, status) in &statuses {
                let status_str = match status {
                    crate::engine::ProcessStatus::Stopped => "stopped",
                    crate::engine::ProcessStatus::Starting => "starting",
                    crate::engine::ProcessStatus::Running(pid) => {
                        lines.push(format!("{}: running (pid {})", config.id, pid));
                        continue;
                    }
                    crate::engine::ProcessStatus::Crashed(msg) => {
                        lines.push(format!("{}: crashed ({})", config.id, msg));
                        continue;
                    }
                };
                lines.push(format!("{}: {}", config.id, status_str));
            }
            if lines.is_empty() {
                "OK: no projects".to_string()
            } else {
                format!("OK:\n{}", lines.join("\n"))
            }
        }
        "group" => {
            if parts.len() < 3 {
                return "ERROR: usage: group <start|stop> <group_id>".to_string();
            }
            let action = parts[1];
            let group_id = parts[2];
            match action {
                "start" => match manager.start_group(group_id).await {
                    Ok(_) => {
                        emit(
                            toast_tx,
                            "socket",
                            format!("group {} started", group_id),
                            ToastTone::Success,
                        );
                        format!("OK: group {} started", group_id)
                    }
                    Err(e) => {
                        emit(
                            toast_tx,
                            "socket",
                            format!("group {} failed: {}", group_id, e),
                            ToastTone::Error,
                        );
                        format!("ERROR: {}", e)
                    }
                },
                "stop" => {
                    let _ = manager.stop_group(group_id).await;
                    emit(
                        toast_tx,
                        "socket",
                        format!("group {} stopped", group_id),
                        ToastTone::Success,
                    );
                    format!("OK: group {} stopped", group_id)
                }
                _ => "ERROR: usage: group <start|stop> <group_id>".to_string(),
            }
        }
        "groups" => {
            let groups = manager.get_groups();
            if groups.is_empty() {
                "OK: no groups".to_string()
            } else {
                let lines: Vec<String> = groups
                    .iter()
                    .map(|g| format!("{} ({})", g.id, g.name))
                    .collect();
                format!("OK:\n{}", lines.join("\n"))
            }
        }
        "projects" => {
            let projects = manager.get_projects();
            if projects.is_empty() {
                "OK: no projects".to_string()
            } else {
                let lines: Vec<String> = projects
                    .iter()
                    .map(|p| format!("{}: {}", p.id, p.name.as_deref().unwrap_or("-")))
                    .collect();
                format!("OK:\n{}", lines.join("\n"))
            }
        }
        "help" => {
            let entries: Vec<String> = COMMAND_TEMPLATES
                .iter()
                .map(|(cmd, args)| {
                    let usage = args.iter().map(|s| s.trim()).collect::<Vec<_>>().join("");
                    format!("{} {}", cmd, usage)
                })
                .collect();
            format!("OK: commands:\n{}", entries.join("\n"))
        }
        _ => format!(
            "ERROR: unknown command '{}'. Type 'help' for usage.",
            parts[0]
        ),
    }
}
