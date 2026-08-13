use crate::config::{Group, Project, Template};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

mod env;

use env::{resolve_env, EnvCtx};

const LOG_DIR: &str = "/tmp/matrix-logs";
const MAX_LOG_FILE_LINES: usize = 2500;

fn is_port_available(port: u16) -> bool {
    if TcpListener::bind(("127.0.0.1", port)).is_err() {
        return false;
    }

    // Check IPv6 binding - if IPv6 is supported on the system, it must also succeed.
    // If it fails with AddrInUse, it's definitely occupied.
    if let Err(e) = TcpListener::bind(("[::1]", port)) {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            return false;
        }
    }

    true
}

fn find_available_port(start_port: u16, skip_ports: &[u16]) -> u16 {
    let mut port = start_port;
    while skip_ports.contains(&port) || !is_port_available(port) {
        port += 1;
        if port == 5000 {
            port = 5001;
        }
    }
    port
}

fn read_env_var_from_file(path: &std::path::Path, key: &str) -> Option<String> {
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    let mut val = v.trim().to_string();
                    if (val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\''))
                    {
                        val.remove(0);
                        val.pop();
                    }
                    return Some(val);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessStatus {
    Stopped,
    Starting,
    Running(u32), // PID
    Crashed(String),
}

pub struct ProcessHandle {
    pub config: Project,
    pub status: ProcessStatus,
    pub logs: Arc<Mutex<VecDeque<String>>>,
    pub join_handle: Option<tokio::task::JoinHandle<()>>,
    pub log_path: PathBuf,
}

pub struct ProcessManager {
    processes: Arc<Mutex<HashMap<String, ProcessHandle>>>,
    templates: Arc<Mutex<Vec<Template>>>,
    groups: Arc<Mutex<Vec<Group>>>,
    root_dir: PathBuf,
    host_mode: Arc<Mutex<HashMap<String, bool>>>,
    prod_mode: Arc<Mutex<HashMap<String, bool>>>,
    /// Per-process log revision counter. Bumped on every appended log line so
    /// views can detect changed content even when the line count is unchanged
    /// (e.g. after truncation/rotation).
    log_rev: Arc<Mutex<HashMap<String, u64>>>,
}

impl ProcessManager {
    pub fn new(
        projects: Vec<Project>,
        templates: Vec<Template>,
        groups: Vec<Group>,
        root_dir: PathBuf,
    ) -> Self {
        let mut processes = HashMap::new();

        let _ = fs::create_dir_all(LOG_DIR);

        for config in &projects {
            let log_path = PathBuf::from(LOG_DIR).join(format!("{}.log", config.id));
            processes.insert(
                config.id.clone(),
                ProcessHandle {
                    config: config.clone(),
                    status: ProcessStatus::Stopped,
                    logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
                    join_handle: None,
                    log_path,
                },
            );

            // If the project declares a backend, register a virtual engine
            // project (`engine:<id>`) that starts before and stops with it.
            if let Some(backend) = &config.backend {
                let engine_id = format!("engine:{}", config.id);
                let engine_log_path = PathBuf::from(LOG_DIR).join(format!("{}.log", engine_id));
                let engine_project = Project {
                    id: engine_id.clone(),
                    name: Some(format!("{} (Engine)", config.get_name())),
                    path: backend.path.clone(),
                    port: None,
                    command: backend.command.clone(),
                    env_only: false,
                    category: backend
                        .category
                        .clone()
                        .or_else(|| Some("platform".to_string())),
                    deps: backend.deps.clone(),
                    backend: None,
                    env: backend.env.clone(),
                };

                processes.insert(
                    engine_id,
                    ProcessHandle {
                        config: engine_project,
                        status: ProcessStatus::Stopped,
                        logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
                        join_handle: None,
                        log_path: engine_log_path,
                    },
                );
            }
        }
        Self {
            processes: Arc::new(Mutex::new(processes)),
            templates: Arc::new(Mutex::new(templates)),
            groups: Arc::new(Mutex::new(groups)),
            root_dir,
            host_mode: Arc::new(Mutex::new(HashMap::new())),
            prod_mode: Arc::new(Mutex::new(HashMap::new())),
            log_rev: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start a project and anything it depends on. Dependency cycles are
    /// guarded: a project already in the current start chain is skipped.
    pub fn start(&self, id: &str) -> anyhow::Result<()> {
        self.start_core(id, &mut Vec::new())
    }

    fn start_with_guard(&self, id: &str, chain: &mut Vec<String>) -> anyhow::Result<()> {
        if chain.contains(&id.to_string()) {
            return Ok(()); // dependency cycle — already in flight
        }
        chain.push(id.to_string());
        let result = self.start_core(id, chain);
        chain.pop();
        result
    }

    fn start_core(&self, id: &str, chain: &mut Vec<String>) -> anyhow::Result<()> {
        // 1. Resolve startup dependencies (outside processes_lock to prevent deadlocks)
        let deps: Vec<String> = {
            let lock = self.processes.lock().unwrap();
            let mut deps = lock
                .get(id)
                .map(|h| h.config.deps.clone())
                .unwrap_or_default();
            // A project with a backend implicitly depends on its engine.
            if lock
                .get(id)
                .map(|h| h.config.backend.is_some())
                .unwrap_or(false)
            {
                deps.push(format!("engine:{}", id));
            }
            deps
        };

        for dep in deps {
            let needs_start = {
                let lock = self.processes.lock().unwrap();
                lock.get(&dep)
                    .map(|h| matches!(h.status, ProcessStatus::Stopped))
                    .unwrap_or(false)
            };
            if needs_start {
                let _ = self.start_with_guard(&dep, chain);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        let mut processes_lock = self.processes.lock().unwrap();

        // Port conflict check & Auto-assignment
        let mut effective_port = processes_lock.get(id).and_then(|h| h.config.port);

        // If starting a virtual engine project, resolve its port from the
        // parent's backend spec: fixed backend.port, else parent port + 1.
        if effective_port.is_none() {
            if let Some(parent_id) = id.strip_prefix("engine:") {
                let backend_port = processes_lock
                    .get(parent_id)
                    .and_then(|h| h.config.backend.as_ref())
                    .and_then(|b| b.port);

                let resolved_port = if let Some(port) = backend_port {
                    port
                } else {
                    let parent_port = processes_lock.get(parent_id).and_then(|h| h.config.port);
                    let skip_ports: Vec<u16> = processes_lock
                        .values()
                        .filter_map(|h| h.config.port)
                        .collect();
                    let base_port = parent_port.unwrap_or(3000) + 1;
                    find_available_port(base_port, &skip_ports)
                };

                effective_port = Some(resolved_port);

                if let Some(handle) = processes_lock.get_mut(id) {
                    handle.config.port = Some(resolved_port);
                }
            }
        }

        if effective_port.is_none() {
            let used_ports: Vec<u16> = processes_lock
                .values()
                .filter_map(|h| {
                    if h.config.id != id {
                        h.config.port
                    } else {
                        None
                    }
                })
                .collect();

            // Standalone projects (not in any group) start at 5173 and go up
            let is_standalone =
                self.find_group_for_project(id).is_none() && !id.starts_with("engine:");

            let base_port = if is_standalone { 5173 } else { 3000 };
            let next_port = find_available_port(base_port, &used_ports);
            effective_port = Some(next_port);

            if let Some(handle) = processes_lock.get_mut(id) {
                handle.config.port = Some(next_port);
            }
        }

        if let Some(port) = effective_port {
            for (pid, handle) in processes_lock.iter() {
                if pid != id && handle.config.port == Some(port) {
                    if let ProcessStatus::Running(_) = handle.status {
                        return Err(anyhow::anyhow!(
                            "Port {} is already in use by project {}",
                            port,
                            handle.config.get_name()
                        ));
                    }
                }
            }
        }

        // Resolve the process environment from the project's env specs.
        // Backend (engine:<id>) specs resolve against the PARENT project
        // ({{id}}/{{path}}/{{parent_port}} refer to the parent; {{port}} is the
        // backend's own resolved port). Parent specs can reference the backend
        // port via {{backend_port}}.
        let used_ports: Vec<u16> = processes_lock
            .values()
            .filter_map(|h| h.config.port)
            .collect();
        let (env_specs, env_file_base, env_ctx) =
            if let Some(parent_id) = id.strip_prefix("engine:") {
                let parent_id = parent_id.to_string();
                let parent = processes_lock.get(&parent_id).map(|h| h.config.clone());
                let parent_path = parent.as_ref().map(|p| p.path.clone()).unwrap_or_default();
                let parent_port = parent.as_ref().and_then(|p| p.port);
                let specs = processes_lock
                    .get(id)
                    .map(|h| h.config.env.clone())
                    .unwrap_or_default();
                (
                    specs,
                    PathBuf::from(&parent_path),
                    EnvCtx {
                        id: parent_id,
                        path: parent_path,
                        port: effective_port,
                        parent_port,
                        backend_port: None,
                        used_ports,
                        resolved: HashMap::new(),
                    },
                )
            } else {
                let self_path = processes_lock
                    .get(id)
                    .map(|h| h.config.path.clone())
                    .unwrap_or_default();
                let backend_port = processes_lock
                    .get(&format!("engine:{}", id))
                    .and_then(|h| h.config.port);
                let specs = processes_lock
                    .get(id)
                    .map(|h| h.config.env.clone())
                    .unwrap_or_default();
                (
                    specs,
                    PathBuf::from(&self_path),
                    EnvCtx {
                        id: id.to_string(),
                        path: self_path,
                        port: effective_port,
                        parent_port: None,
                        backend_port,
                        used_ports,
                        resolved: HashMap::new(),
                    },
                )
            };
        let resolved_env = resolve_env(&env_specs, &env_ctx, &env_file_base, |pid| {
            processes_lock
                .get(pid)
                .map(|h| matches!(h.status, ProcessStatus::Running(_)))
                .unwrap_or(false)
        });

        if let Some(handle) = processes_lock.get_mut(id) {
            if matches!(handle.status, ProcessStatus::Running(_)) {
                return Ok(());
            }

            if handle.config.env_only || handle.config.command.is_none() {
                return Err(anyhow::anyhow!(
                    "Project {} is set to env_only or has no command",
                    handle.config.get_name()
                ));
            }

            {
                let mut logs = handle.logs.lock().unwrap();
                logs.clear();
                self.bump_log_rev(id);
            }
            {
                let _ = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&handle.log_path);
            }

            // Mark as starting before the spawn attempt so the UI can show a
            // transitional state (the spawn itself is quick, but the status is
            // still truthful and covers the fork/exec window).
            handle.status = ProcessStatus::Starting;

            let mut cmd = Command::new("sh");

            // Host mode: make the dev server bind to 0.0.0.0 so other devices can connect.
            // Vite ignores the HOST env var and needs --host as a CLI flag.
            // Next.js and cargo respect the HOST env var.
            // For engine projects, check host mode on the parent website project.
            let host_mode_id = id.strip_prefix("engine:").unwrap_or(id);
            let mut command_str = if self.is_host_mode(host_mode_id) {
                let raw = handle.config.command.as_ref().unwrap();
                if is_vite_command(raw) {
                    format!("{} --host 0.0.0.0", raw)
                } else {
                    cmd.env("HOST", "0.0.0.0");
                    raw.clone()
                }
            } else {
                handle.config.command.as_ref().unwrap().clone()
            };

            // Prod mode: replace 'dev' with 'preview' in the command for Vite projects.
            // e.g. "pnpm dev --port 3000" -> "pnpm preview --port 3000"
            // Only applies to Vite-related commands.
            if self.is_prod_mode(id) && is_vite_command(&command_str) {
                command_str = command_str
                    .replace("pnpm dev", "pnpm preview")
                    .replace("npm run dev", "npm run preview")
                    .replace("yarn dev", "yarn preview");
            }

            cmd.arg("-c").arg(command_str);

            // Always inject the resolved port, then apply the project's env specs.
            if let Some(port) = effective_port {
                let port_str = port.to_string();
                cmd.env("PORT", &port_str);
                cmd.env("ENGINE_PORT", &port_str);
            }
            for (key, value) in &resolved_env {
                cmd.env(key, value);
            }

            // Start in a new process group so we can kill the entire tree
            unsafe {
                cmd.pre_exec(|| {
                    let _ = nix::unistd::setpgid(Pid::from_raw(0), Pid::from_raw(0));
                    Ok(())
                });
            }

            let path = PathBuf::from(&handle.config.path);
            if path.is_absolute() {
                cmd.current_dir(path);
            } else {
                cmd.current_dir(self.root_dir.join(path));
            }

            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            match cmd.spawn() {
                Ok(mut child) => {
                    let pid = child.id().unwrap();
                    handle.status = ProcessStatus::Running(pid);

                    let logs = handle.logs.clone();
                    let log_path = handle.log_path.clone();
                    let stdout = child.stdout.take().unwrap();
                    let stderr = child.stderr.take().unwrap();

                    // Append a timestamped line to the log file.
                    let append_to_file = |path: &PathBuf, line: &str| {
                        let mut file = match OpenOptions::new().create(true).append(true).open(path)
                        {
                            Ok(f) => f,
                            Err(_) => return,
                        };
                        let ts = chrono::Local::now().format("%H:%M:%S");
                        let _ = writeln!(file, "[{}] {}", ts, line);
                    };

                    let rev_stdout = self.log_rev.clone();
                    let id_stdout = id.to_string();
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stdout).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            let mut logs = logs.lock().unwrap();
                            logs.push_back(line.clone());
                            if logs.len() > 1000 {
                                logs.pop_front();
                            }
                            append_to_file(&log_path, &line);
                            *rev_stdout
                                .lock()
                                .unwrap()
                                .entry(id_stdout.clone())
                                .or_insert(0) += 1;
                        }
                    });

                    let logs_err = handle.logs.clone();
                    let log_path_err = handle.log_path.clone();
                    let rev_stderr = self.log_rev.clone();
                    let id_stderr = id.to_string();
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stderr).lines();
                        while let Ok(Some(line)) = reader.next_line().await {
                            let mut logs = logs_err.lock().unwrap();
                            let lower = line.to_lowercase();
                            let formatted = if lower.contains("compiling")
                                || lower.contains("finished")
                                || lower.contains("running")
                                || lower.contains("building")
                            {
                                line.clone()
                            } else {
                                format!("\x1b[31m[ERR] {}\x1b[0m", line)
                            };
                            logs.push_back(formatted);
                            if logs.len() > 1000 {
                                logs.pop_front();
                            }
                            append_to_file(&log_path_err, &line);
                            *rev_stderr
                                .lock()
                                .unwrap()
                                .entry(id_stderr.clone())
                                .or_insert(0) += 1;
                        }
                    });

                    let processes = self.processes.clone();
                    let id_str = id.to_string();
                    let join_handle = tokio::spawn(async move {
                        let exit_result = child.wait().await;
                        let mut proc_lock = processes.lock().unwrap();
                        if let Some(h) = proc_lock.get_mut(&id_str) {
                            if let ProcessStatus::Running(running_pid) = h.status {
                                if running_pid == pid {
                                    match exit_result {
                                        Ok(status) if status.success() => {
                                            h.status = ProcessStatus::Stopped;
                                        }
                                        Ok(status) => {
                                            h.status = ProcessStatus::Crashed(format!(
                                                "Exited with code {}",
                                                status.code().unwrap_or(-1)
                                            ));
                                        }
                                        Err(e) => {
                                            h.status = ProcessStatus::Crashed(e.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    });

                    handle.join_handle = Some(join_handle);
                }
                Err(e) => {
                    handle.status = ProcessStatus::Crashed(e.to_string());
                    let mut logs = handle.logs.lock().unwrap();
                    logs.push_back(format!("\x1b[31m[CRITICAL] Failed to spawn: {}\x1b[0m", e));
                    self.bump_log_rev(id);
                    return Err(anyhow::anyhow!("Failed to spawn: {}", e));
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&self, id: &str) -> anyhow::Result<()> {
        let has_backend = {
            let processes = self.processes.lock().unwrap();
            processes
                .get(id)
                .map(|h| h.config.backend.is_some())
                .unwrap_or(false)
        };

        let mut engine_pid = None;
        let mut engine_join_handle = None;
        let mut website_pid = None;
        let mut website_join_handle = None;

        {
            let mut processes = self.processes.lock().unwrap();
            if has_backend {
                let engine_id = format!("engine:{}", id);
                if let Some(engine_handle) = processes.get_mut(&engine_id) {
                    if let ProcessStatus::Running(pid) = engine_handle.status {
                        engine_pid = Some(pid);
                    }
                    engine_join_handle = engine_handle.join_handle.take();
                }
            }
            if let Some(handle) = processes.get_mut(id) {
                if let ProcessStatus::Running(pid) = handle.status {
                    website_pid = Some(pid);
                }
                website_join_handle = handle.join_handle.take();
            }
        }

        // 1. Clean up auxiliary engine process
        if let Some(pid) = engine_pid {
            let pgid = Pid::from_raw(-(pid as i32));
            let _ = signal::kill(pgid, Signal::SIGINT);

            if let Some(jh) = engine_join_handle {
                if tokio::time::timeout(std::time::Duration::from_millis(1000), jh)
                    .await
                    .is_err()
                {
                    let _ = signal::kill(pgid, Signal::SIGKILL);
                }
            }
        }

        // 2. Clean up website process
        if let Some(pid) = website_pid {
            let pgid = Pid::from_raw(-(pid as i32));
            let _ = signal::kill(pgid, Signal::SIGINT);

            if let Some(jh) = website_join_handle {
                if tokio::time::timeout(std::time::Duration::from_millis(1000), jh)
                    .await
                    .is_err()
                {
                    let _ = signal::kill(pgid, Signal::SIGKILL);
                }
            }
        }

        // 3. Update the process status back to Stopped
        {
            let mut processes = self.processes.lock().unwrap();
            if has_backend {
                let engine_id = format!("engine:{}", id);
                if let Some(h) = processes.get_mut(&engine_id) {
                    // Reset even if the engine had already crashed (no live pid) —
                    // a stopped process must not remain flagged Crashed forever.
                    if matches!(
                        h.status,
                        ProcessStatus::Running(_) | ProcessStatus::Crashed(_)
                    ) {
                        h.status = ProcessStatus::Stopped;
                    }
                }
            }
            if let Some(h) = processes.get_mut(id) {
                if matches!(
                    h.status,
                    ProcessStatus::Running(_) | ProcessStatus::Crashed(_)
                ) {
                    h.status = ProcessStatus::Stopped;
                }
            }
        }

        Ok(())
    }

    pub async fn shutdown(&self) {
        self.cleanup_host_rules();

        let keys: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes.keys().cloned().collect()
        };
        for id in keys {
            let _ = self.stop(&id).await;
        }
    }

    pub async fn run_template(&self, template_name: &str) -> anyhow::Result<()> {
        let templates = self.templates.lock().unwrap();
        if let Some(template) = templates.iter().find(|t| t.name == template_name) {
            for project_id in &template.projects {
                let _ = self.start(project_id);
            }
            Ok(())
        } else {
            Err(anyhow::anyhow!("Template {} not found", template_name))
        }
    }

    // ── Group Management ─────────────────────────────────────────────────────

    pub fn get_groups(&self) -> Vec<Group> {
        let groups = self.groups.lock().unwrap();
        groups.clone()
    }

    pub fn get_group(&self, group_id: &str) -> Option<Group> {
        let groups = self.groups.lock().unwrap();
        groups.iter().find(|g| g.id == group_id).cloned()
    }

    /// Start all projects in a group, stopping any other running group first.
    /// Also starts the group's infrastructure projects.
    pub async fn start_group(&self, group_id: &str) -> anyhow::Result<()> {
        let group = self
            .get_group(group_id)
            .ok_or_else(|| anyhow::anyhow!("Group {} not found", group_id))?;

        self.stop_all_groups().await?;

        for infra_id in &group.infrastructure {
            let _ = self.start(infra_id);
        }

        for project_id in &group.projects {
            let _ = self.start(project_id);
        }

        Ok(())
    }

    /// Stop all projects in a group, including their engines and infrastructure.
    pub async fn stop_group(&self, group_id: &str) -> anyhow::Result<()> {
        let group = self
            .get_group(group_id)
            .ok_or_else(|| anyhow::anyhow!("Group {} not found", group_id))?;

        for project_id in &group.projects {
            let _ = self.stop(project_id).await;
            let engine_id = format!("engine:{}", project_id);
            let _ = self.stop(&engine_id).await;
        }

        for infra_id in &group.infrastructure {
            let _ = self.stop(infra_id).await;
        }

        Ok(())
    }

    /// Stop all projects that belong to any group (used for mutual exclusion).
    pub async fn stop_all_groups(&self) -> anyhow::Result<()> {
        let groups = self.groups.lock().unwrap().clone();

        let mut to_stop: Vec<String> = Vec::new();
        for g in &groups {
            for pid in &g.projects {
                let engine_id = format!("engine:{}", pid);
                {
                    let processes = self.processes.lock().unwrap();
                    if let Some(handle) = processes.get(pid) {
                        if matches!(handle.status, ProcessStatus::Running(_)) {
                            to_stop.push(pid.clone());
                        }
                    }
                    if let Some(handle) = processes.get(&engine_id) {
                        if matches!(handle.status, ProcessStatus::Running(_)) {
                            to_stop.push(engine_id);
                        }
                    }
                }
            }
        }
        drop(groups);

        // Stop any running group project (lock released between iterations)
        for id in &to_stop {
            let _ = self.stop(id).await;
        }

        Ok(())
    }

    /// Find the group ID that a project belongs to, if any.
    pub fn find_group_for_project(&self, project_id: &str) -> Option<String> {
        let groups = self.groups.lock().unwrap();
        for g in groups.iter() {
            if g.projects.contains(&project_id.to_string()) {
                return Some(g.id.clone());
            }
        }
        None
    }

    pub fn remove_project(&self, id: &str) {
        let mut processes = self.processes.lock().unwrap();
        processes.remove(id);
    }

    pub fn get_templates(&self) -> Vec<Template> {
        let templates = self.templates.lock().unwrap();
        templates.clone()
    }

    pub fn add_project(&self, project: Project) {
        let mut processes = self.processes.lock().unwrap();
        let log_path = PathBuf::from(LOG_DIR).join(format!("{}.log", project.id));
        // Ensure log directory exists (in case it was cleaned from /tmp)
        let _ = fs::create_dir_all(LOG_DIR);
        processes.insert(
            project.id.clone(),
            ProcessHandle {
                config: project,
                status: ProcessStatus::Stopped,
                logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
                join_handle: None,
                log_path,
            },
        );
    }

    pub fn add_to_template(&self, template_name: &str, project_id: String) -> anyhow::Result<()> {
        let mut templates = self.templates.lock().unwrap();
        if let Some(t) = templates.iter_mut().find(|t| t.name == template_name) {
            if !t.projects.contains(&project_id) {
                t.projects.push(project_id);
            }
            Ok(())
        } else {
            Err(anyhow::anyhow!("Template not found"))
        }
    }

    pub fn get_statuses(&self) -> Vec<(Project, ProcessStatus)> {
        let processes = self.processes.lock().unwrap();
        let mut items: Vec<(Project, ProcessStatus)> = processes
            .values()
            .map(|h| (h.config.clone(), h.status.clone()))
            .collect();
        items.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        items
    }

    pub fn get_projects(&self) -> Vec<Project> {
        let processes = self.processes.lock().unwrap();
        processes
            .values()
            .filter(|h| !h.config.id.starts_with("engine:"))
            .map(|h| h.config.clone())
            .collect()
    }

    pub fn update_project_port(&self, id: &str, port: Option<u16>) {
        let mut processes = self.processes.lock().unwrap();
        if let Some(handle) = processes.get_mut(id) {
            handle.config.port = port;
        }
    }

    pub fn update_project_category(&self, id: &str, category: Option<String>) {
        let mut processes = self.processes.lock().unwrap();
        if let Some(handle) = processes.get_mut(id) {
            handle.config.category = category;
        }
    }

    pub fn delete_template(&self, name: &str) {
        let mut templates = self.templates.lock().unwrap();
        templates.retain(|t| t.name != name);
    }

    pub fn create_template(&self, name: String) {
        let mut templates = self.templates.lock().unwrap();
        templates.push(Template {
            name,
            projects: vec![],
        });
    }

    pub fn get_logs(&self, id: &str) -> Vec<String> {
        let mut processes = self.processes.lock().unwrap();
        if let Some(handle) = processes.get_mut(id) {
            handle.logs.lock().unwrap().iter().cloned().collect()
        } else {
            vec![]
        }
    }

    /// Current log revision for a process (bumped on every appended line).
    pub fn get_log_rev(&self, id: &str) -> u64 {
        let rev = self.log_rev.lock().unwrap();
        *rev.get(id).unwrap_or(&0)
    }

    fn bump_log_rev(&self, id: &str) {
        let mut rev = self.log_rev.lock().unwrap();
        *rev.entry(id.to_string()).or_insert(0) += 1;
    }

    // ── Host Mode ──────────────────────────────────────────────────────────────

    pub fn is_host_mode(&self, id: &str) -> bool {
        let hm = self.host_mode.lock().unwrap();
        *hm.get(id).unwrap_or(&false)
    }

    pub fn toggle_host_mode(&self, id: &str) -> anyhow::Result<bool> {
        let mut hm = self.host_mode.lock().unwrap();
        let current = *hm.get(id).unwrap_or(&false);
        let new = !current;

        // Manage firewall rule at the OS level (no process restart needed).
        // If the rule can't be applied (nft missing / permission denied), the
        // mode must NOT be reported as enabled — surface the failure instead.
        if new {
            self.add_host_port(id)?;
        } else {
            self.remove_host_port(id);
        }

        hm.insert(id.to_string(), new);
        Ok(new)
    }

    /// Add an nftables rule to accept incoming TCP traffic on the project's port.
    /// Errors (nft not installed, missing permissions) are returned so the UI
    /// can tell the user why host mode can't actually open the port.
    fn add_host_port(&self, id: &str) -> anyhow::Result<()> {
        let port = {
            let processes = self.processes.lock().unwrap();
            processes.get(id).and_then(|h| h.config.port)
        };
        if let Some(port) = port {
            let output = std::process::Command::new("nft")
                .args([
                    "add",
                    "rule",
                    "ip",
                    "filter",
                    "INPUT",
                    "tcp",
                    "dport",
                    &port.to_string(),
                    "counter",
                    "accept",
                ])
                .output()
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Cannot open port {}: nftables is not available ({e}). \
                         Install nftables or run Matrix with sudo.",
                        port
                    )
                })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!(
                    "Cannot open port {}: nft rejected the rule: {}",
                    port,
                    stderr.trim()
                ));
            }
        }
        Ok(())
    }

    /// Remove the nftables rule for the project's port.
    fn remove_host_port(&self, id: &str) {
        let port = {
            let processes = self.processes.lock().unwrap();
            processes.get(id).and_then(|h| h.config.port)
        };
        if let Some(port) = port {
            if let Ok(output) = std::process::Command::new("nft")
                .args(["-e", "-a", "list", "chain", "ip", "filter", "INPUT"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.contains(&format!("tcp dport {}", port)) {
                        // Extract the handle number (last word in the line)
                        if let Some(handle) = line.split_whitespace().last() {
                            let handle = handle.trim_end_matches('/');
                            let _ = std::process::Command::new("nft")
                                .args(["delete", "rule", "ip", "filter", "INPUT", "handle", handle])
                                .output();
                        }
                        break;
                    }
                }
            }
        }
    }

    // ── Prod Mode ──────────────────────────────────────────────────────────────

    pub fn is_prod_mode(&self, id: &str) -> bool {
        let pm = self.prod_mode.lock().unwrap();
        *pm.get(id).unwrap_or(&false)
    }

    pub fn toggle_prod_mode(&self, id: &str) -> bool {
        let mut pm = self.prod_mode.lock().unwrap();
        let current = *pm.get(id).unwrap_or(&false);
        let new = !current;
        pm.insert(id.to_string(), new);
        new
    }

    /// Remove all host-mode firewall rules (called on shutdown).
    pub fn cleanup_host_rules(&self) {
        let ports: Vec<u16> = {
            let processes = self.processes.lock().unwrap();
            processes
                .iter()
                .filter(|(id, _)| self.is_host_mode(id))
                .filter_map(|(_, h)| h.config.port)
                .collect()
        };
        for port in ports {
            if let Ok(output) = std::process::Command::new("nft")
                .args(["-e", "-a", "list", "chain", "ip", "filter", "INPUT"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.contains(&format!("tcp dport {}", port)) {
                        if let Some(handle) = line.split_whitespace().last() {
                            let handle = handle.trim_end_matches('/');
                            let _ = std::process::Command::new("nft")
                                .args(["delete", "rule", "ip", "filter", "INPUT", "handle", handle])
                                .output();
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Enforce the log file line cap by truncating to the last MAX_LOG_FILE_LINES lines.
    /// Called periodically from the app's main loop.
    pub fn enforce_log_cap(&self, id: &str) {
        let path = {
            let processes = self.processes.lock().unwrap();
            processes.get(id).map(|h| h.log_path.clone())
        };
        if let Some(path) = path {
            if let Ok(content) = fs::read_to_string(&path) {
                let lines: Vec<&str> = content.lines().collect();
                if lines.len() > MAX_LOG_FILE_LINES {
                    let start = lines.len() - MAX_LOG_FILE_LINES;
                    let truncated = lines[start..].join("\n");
                    let _ = fs::write(&path, truncated + "\n");
                }
            }
        }
    }
}

/// Detect whether a command string ultimately runs Vite's dev server.
/// Matches direct `vite` invocations as well as common package-manager wrappers
/// (`pnpm dev`, `npm run dev`, `yarn dev`) that resolve to Vite.
fn is_vite_command(raw: &str) -> bool {
    if raw.starts_with("vite ") || raw == "vite" {
        return !raw.contains("vitest");
    }
    // Package-manager wrappers that run the "dev" script (which is Vite in Vite projects)
    raw.contains("pnpm dev")
        || raw.contains("pnpm dlx vite")
        || raw.contains("npm run dev")
        || raw.contains("npm exec vite")
        || raw.contains("yarn dev")
        || raw.contains("yarn vite")
}

#[cfg(test)]
mod tests;
