#![cfg(test)]

use crate::config::Project;
use crate::engine::{is_port_available, ProcessManager, ProcessStatus};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

fn project(id: &str, command: &str) -> Project {
    Project {
        id: id.to_string(),
        name: Some(id.to_string()),
        path: ".".to_string(),
        port: None,
        command: Some(command.to_string()),
        env_only: false,
        category: None,
        deps: vec![],
        backend: None,
        env: vec![],
    }
}

#[test]
fn test_port_availability() {
    // Test IPv4 loopback binding
    let listener_v4 = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener_v4.local_addr().unwrap().port();
    assert!(!is_port_available(port));

    // Test IPv6 loopback binding
    if let Ok(listener_v6) = TcpListener::bind(("[::1]", 0)) {
        let port_v6 = listener_v6.local_addr().unwrap().port();
        assert!(!is_port_available(port_v6));
    }
}

#[tokio::test]
async fn test_process_exit_success() {
    let root = PathBuf::from(".");
    let manager = ProcessManager::new(
        vec![project("test_exit_success", "sleep 0.1")],
        vec![],
        vec![],
        root,
    );

    // Start project
    manager.start("test_exit_success").unwrap();

    // Should be running initially
    let statuses = manager.get_statuses();
    assert!(matches!(statuses[0].1, ProcessStatus::Running(_)));

    // Wait for it to exit
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Should have updated status to Stopped
    let statuses = manager.get_statuses();
    assert_eq!(statuses[0].1, ProcessStatus::Stopped);
}

#[tokio::test]
async fn test_process_exit_failure() {
    let root = PathBuf::from(".");
    let manager = ProcessManager::new(
        vec![project("test_exit_failure", "sh -c 'exit 42'")],
        vec![],
        vec![],
        root,
    );

    // Start project
    manager.start("test_exit_failure").unwrap();

    // Wait for it to exit
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Should have updated status to Crashed
    let statuses = manager.get_statuses();
    if let ProcessStatus::Crashed(err) = &statuses[0].1 {
        assert!(err.contains("42"), "Expected exit code 42 but got: {}", err);
    } else {
        panic!(
            "Expected status to be Crashed, but got: {:?}",
            statuses[0].1
        );
    }
}

#[tokio::test]
async fn test_process_graceful_stop() {
    let root = PathBuf::from(".");
    let manager = ProcessManager::new(
        vec![project("test_graceful_stop", "sleep 5")],
        vec![],
        vec![],
        root,
    );

    // Start
    manager.start("test_graceful_stop").unwrap();

    // Verify running
    let statuses = manager.get_statuses();
    assert!(matches!(statuses[0].1, ProcessStatus::Running(_)));

    // Stop
    manager.stop("test_graceful_stop").await.unwrap();

    // Verify stopped
    let statuses = manager.get_statuses();
    assert_eq!(statuses[0].1, ProcessStatus::Stopped);
}

#[test]
fn backend_declaration_registers_virtual_engine() {
    use crate::config::{BackendSpec, EnvSpec};

    let mut site = project("site.example.com", "pnpm dev");
    site.backend = Some(BackendSpec {
        path: "platform/engine".to_string(),
        command: Some("cargo run".to_string()),
        port: None,
        deps: vec!["plugin-manager".to_string()],
        env: vec![EnvSpec {
            key: "MONGODB_URI".to_string(),
            value: None,
            file: vec![".env".to_string()],
            default: Some("mongodb://127.0.0.1:27017/{{dbname}}".to_string()),
            if_running: None,
            else_value: None,
        }],
        category: None,
    });
    let plain = project("plain-app", "npm run dev");

    let root = PathBuf::from(".");
    let manager = ProcessManager::new(vec![site, plain], vec![], vec![], root);

    let statuses = manager.get_statuses();
    let engine = statuses
        .iter()
        .find(|(p, _)| p.id == "engine:site.example.com")
        .expect("virtual engine registered");
    assert_eq!(engine.0.path, "platform/engine");
    assert_eq!(engine.0.command.as_deref(), Some("cargo run"));
    assert_eq!(engine.0.category.as_deref(), Some("platform"));
    assert_eq!(engine.0.deps, vec!["plugin-manager"]);
    assert_eq!(engine.0.env.len(), 1);

    // Projects without a backend must not get a virtual engine.
    assert!(!statuses.iter().any(|(p, _)| p.id == "engine:plain-app"));
}

#[tokio::test]
async fn deps_and_backend_cascade_start_before_project() {
    use crate::config::BackendSpec;

    let mut site = project("site.example.com", "sleep 5");
    site.backend = Some(BackendSpec {
        path: ".".to_string(),
        command: Some("sleep 5".to_string()),
        port: None,
        deps: vec!["plugin-manager".to_string()],
        env: vec![],
        category: None,
    });
    let pm = project("plugin-manager", "sleep 5");

    let manager = ProcessManager::new(vec![site, pm], vec![], vec![], PathBuf::from("."));
    manager.start("site.example.com").unwrap();

    let statuses = manager.get_statuses();
    // Backend started via the implicit engine dep.
    assert!(statuses.iter().any(|(p, s)| {
        p.id == "engine:site.example.com" && matches!(s, ProcessStatus::Running(_))
    }));
    // Backend's own deps started first.
    assert!(statuses
        .iter()
        .any(|(p, s)| { p.id == "plugin-manager" && matches!(s, ProcessStatus::Running(_)) }));

    manager.stop("site.example.com").await.unwrap();
    manager.stop("plugin-manager").await.unwrap();
}

#[tokio::test]
async fn dependency_cycle_does_not_hang() {
    let mut a = project("cycle-a", "sleep 5");
    a.deps = vec!["cycle-b".to_string()];
    let mut b = project("cycle-b", "sleep 5");
    b.deps = vec!["cycle-a".to_string()];

    let manager = ProcessManager::new(vec![a, b], vec![], vec![], PathBuf::from("."));
    // Must return promptly instead of recursing forever.
    manager.start("cycle-a").unwrap();

    let statuses = manager.get_statuses();
    assert!(statuses
        .iter()
        .any(|(p, s)| { p.id == "cycle-a" && matches!(s, ProcessStatus::Running(_)) }));

    manager.stop("cycle-a").await.unwrap();
    manager.stop("cycle-b").await.unwrap();
}

#[tokio::test]
async fn backend_fixed_port_is_used_for_engine() {
    use crate::config::BackendSpec;

    let mut site = project("site.example.com", "sleep 5");
    site.port = Some(5190);
    site.backend = Some(BackendSpec {
        path: ".".to_string(),
        command: Some("sleep 5".to_string()),
        port: Some(3010),
        deps: vec![],
        env: vec![],
        category: None,
    });

    let manager = ProcessManager::new(vec![site], vec![], vec![], PathBuf::from("."));
    manager.start("site.example.com").unwrap();

    let statuses = manager.get_statuses();
    let engine = statuses
        .iter()
        .find(|(p, _)| p.id == "engine:site.example.com")
        .expect("engine registered");
    assert_eq!(engine.0.port, Some(3010));

    manager.stop("site.example.com").await.unwrap();
}

#[tokio::test]
async fn backend_without_fixed_port_uses_parent_plus_one() {
    use crate::config::BackendSpec;

    let mut site = project("site.example.com", "sleep 5");
    site.port = Some(5191);
    site.backend = Some(BackendSpec {
        path: ".".to_string(),
        command: Some("sleep 5".to_string()),
        port: None,
        deps: vec![],
        env: vec![],
        category: None,
    });

    let manager = ProcessManager::new(vec![site], vec![], vec![], PathBuf::from("."));
    manager.start("site.example.com").unwrap();

    let statuses = manager.get_statuses();
    let engine = statuses
        .iter()
        .find(|(p, _)| p.id == "engine:site.example.com")
        .expect("engine registered");
    assert!(
        engine.0.port.is_some_and(|p| p >= 5192),
        "expected engine port >= 5192, got {:?}",
        engine.0.port
    );

    manager.stop("site.example.com").await.unwrap();
}

#[tokio::test]
async fn standalone_project_without_port_starts_in_dev_range() {
    let p = project("standalone-app", "sleep 5");
    let manager = ProcessManager::new(vec![p], vec![], vec![], PathBuf::from("."));
    manager.start("standalone-app").unwrap();

    let statuses = manager.get_statuses();
    let proj = statuses
        .iter()
        .find(|(p, _)| p.id == "standalone-app")
        .unwrap();
    assert!(
        proj.0.port.is_some_and(|p| p >= 5173),
        "expected standalone port >= 5173, got {:?}",
        proj.0.port
    );

    manager.stop("standalone-app").await.unwrap();
}

#[tokio::test]
async fn backend_env_specs_reach_the_process() {
    use crate::config::{BackendSpec, EnvSpec};

    let mut site = project("site.example.com", "sleep 5");
    site.port = Some(5195);
    site.backend = Some(BackendSpec {
        path: ".".to_string(),
        command: Some(
            "sh -c 'echo URI=$MONGODB_URI SECRET=$JWT_SECRET SITE=$SITE_URL PORT=$PORT'"
                .to_string(),
        ),
        port: Some(3010),
        deps: vec![],
        env: vec![
            EnvSpec {
                key: "MONGODB_URI".to_string(),
                value: None,
                file: vec![],
                default: Some("mongodb://127.0.0.1:27017/MySiteExampleCom".to_string()),
                if_running: None,
                else_value: None,
            },
            EnvSpec {
                key: "JWT_SECRET".to_string(),
                value: None,
                file: vec![],
                default: Some("MYSITEEXAMPLECOM_SECRET".to_string()),
                if_running: None,
                else_value: None,
            },
            EnvSpec {
                key: "SITE_URL".to_string(),
                value: Some("http://localhost:{{parent_port|3000}}".to_string()),
                file: vec![],
                default: None,
                if_running: None,
                else_value: None,
            },
        ],
        category: None,
    });

    let manager = ProcessManager::new(vec![site], vec![], vec![], PathBuf::from("."));
    manager.start("site.example.com").unwrap();

    // The engine process echoes its env then exits; poll the log buffer.
    let engine_id = "engine:site.example.com";
    let mut found = String::new();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let logs = manager.get_logs(engine_id).join("\n");
        if logs.contains("URI=") {
            found = logs;
            break;
        }
    }
    assert!(
        found.contains("URI=mongodb://127.0.0.1:27017/MySiteExampleCom"),
        "logs: {}",
        found
    );
    assert!(
        found.contains("SECRET=MYSITEEXAMPLECOM_SECRET"),
        "logs: {}",
        found
    );
    assert!(
        found.contains("SITE=http://localhost:5195"),
        "logs: {}",
        found
    );
    assert!(found.contains("PORT=3010"), "logs: {}", found);

    manager.stop("site.example.com").await.unwrap();
}

#[tokio::test]
async fn stop_parent_stops_backend() {
    use crate::config::BackendSpec;

    let mut site = project("site.example.com", "sleep 5");
    site.backend = Some(BackendSpec {
        path: ".".to_string(),
        command: Some("sleep 5".to_string()),
        port: None,
        deps: vec![],
        env: vec![],
        category: None,
    });

    let manager = ProcessManager::new(vec![site], vec![], vec![], PathBuf::from("."));
    manager.start("site.example.com").unwrap();

    let statuses = manager.get_statuses();
    assert!(statuses.iter().any(|(p, s)| {
        p.id == "engine:site.example.com" && matches!(s, ProcessStatus::Running(_))
    }));

    manager.stop("site.example.com").await.unwrap();

    let statuses = manager.get_statuses();
    assert!(statuses
        .iter()
        .any(|(p, s)| { p.id == "engine:site.example.com" && *s == ProcessStatus::Stopped }));
    assert!(statuses
        .iter()
        .any(|(p, s)| { p.id == "site.example.com" && *s == ProcessStatus::Stopped }));
}
