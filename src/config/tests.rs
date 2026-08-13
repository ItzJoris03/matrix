#![cfg(test)]

use crate::config::{BackendSpec, EnvSpec, MatrixConfig, Project, Template};
use std::fs;

#[test]
fn test_config_load_save() {
    let config = MatrixConfig {
        projects: vec![Project {
            id: "test".to_string(),
            name: Some("Test App".to_string()),
            path: "/abs/path".to_string(),
            port: Some(8080),
            command: Some("echo test".to_string()),
            env_only: false,
            category: Some("test_cat".to_string()),
            deps: vec![],
            backend: None,
            env: vec![],
        }],
        templates: vec![Template {
            name: "Test Template".to_string(),
            projects: vec!["test".to_string()],
        }],
        groups: vec![],
    };

    let path = "test_matrix.json";
    config.save(path).unwrap();

    let loaded = MatrixConfig::load(path).unwrap();
    assert_eq!(loaded.projects.len(), 1);
    assert_eq!(loaded.projects[0].id, "test");
    assert_eq!(loaded.templates.len(), 1);
    assert_eq!(loaded.templates[0].name, "Test Template");

    fs::remove_file(path).unwrap();
}

#[test]
fn old_shape_config_without_new_fields_loads() {
    // A config written before deps/backend/env existed must still load;
    // unknown "tasks" field is ignored, new fields default to empty.
    let json = r#"{
            "projects": [
                {
                    "id": "legacy",
                    "path": "/legacy",
                    "port": 5173,
                    "command": "npm run dev",
                    "env_only": false,
                    "category": "standalone"
                }
            ],
            "templates": [],
            "tasks": [],
            "groups": []
        }"#;
    let path = "test_old_shape.json";
    fs::write(path, json).unwrap();

    let loaded = MatrixConfig::load(path).unwrap();
    assert_eq!(loaded.projects.len(), 1);
    let p = &loaded.projects[0];
    assert!(p.deps.is_empty());
    assert!(p.backend.is_none());
    assert!(p.env.is_empty());

    fs::remove_file(path).unwrap();
}

#[test]
fn backend_and_env_round_trip() {
    let config = MatrixConfig {
        projects: vec![Project {
            id: "site.example.com".to_string(),
            name: None,
            path: "/abs/site".to_string(),
            port: Some(5173),
            command: Some("pnpm dev".to_string()),
            env_only: false,
            category: Some("website".to_string()),
            deps: vec!["db".to_string()],
            backend: Some(BackendSpec {
                path: "backend".to_string(),
                command: Some("cargo run".to_string()),
                port: Some(3000),
                deps: vec!["plugin-manager".to_string()],
                env: vec![EnvSpec {
                    key: "MONGODB_URI".to_string(),
                    value: None,
                    file: vec![".env.local".to_string(), ".env".to_string()],
                    default: Some("mongodb://127.0.0.1:27017/{{dbname}}".to_string()),
                    if_running: None,
                    else_value: None,
                }],
                category: Some("platform".to_string()),
            }),
            env: vec![EnvSpec {
                key: "NEXT_PUBLIC_DASHBOARD_URL".to_string(),
                value: Some("http://localhost:{{backend_port}}".to_string()),
                file: vec![],
                default: None,
                if_running: Some("engine:site.example.com".to_string()),
                else_value: Some("http://remote.example.com".to_string()),
            }],
        }],
        templates: vec![],
        groups: vec![],
    };

    let path = "test_backend.json";
    config.save(path).unwrap();
    let loaded = MatrixConfig::load(path).unwrap();

    let p = &loaded.projects[0];
    let b = p.backend.as_ref().expect("backend present");
    assert_eq!(b.path, "backend");
    assert_eq!(b.port, Some(3000));
    assert_eq!(b.deps, vec!["plugin-manager"]);
    assert_eq!(
        b.env[0].default.as_deref(),
        Some("mongodb://127.0.0.1:27017/{{dbname}}")
    );
    assert_eq!(p.deps, vec!["db"]);
    assert_eq!(
        p.env[0].if_running.as_deref(),
        Some("engine:site.example.com")
    );
    assert_eq!(
        p.env[0].else_value.as_deref(),
        Some("http://remote.example.com")
    );

    fs::remove_file(path).unwrap();
}

#[test]
fn normalize_paths_resolves_relative_paths_against_base() {
    let mut config = MatrixConfig {
        projects: vec![
            Project {
                id: "abs".to_string(),
                name: None,
                path: "/already/absolute".to_string(),
                port: None,
                command: None,
                env_only: false,
                category: None,
                deps: vec![],
                backend: None,
                env: vec![],
            },
            Project {
                id: "rel".to_string(),
                name: None,
                path: "platform/engine".to_string(),
                port: None,
                command: None,
                env_only: false,
                category: None,
                deps: vec![],
                backend: Some(BackendSpec {
                    path: "services/db".to_string(),
                    command: None,
                    port: None,
                    deps: vec![],
                    env: vec![],
                    category: None,
                }),
                env: vec![],
            },
        ],
        templates: vec![],
        groups: vec![],
    };

    let base = std::path::Path::new("/home/user/.matrix");
    config.normalize_paths(base);

    assert_eq!(config.projects[0].path, "/already/absolute");
    assert_eq!(
        config.projects[1].path,
        "/home/user/.matrix/platform/engine"
    );
    assert_eq!(
        config.projects[1]
            .backend
            .as_ref()
            .expect("backend present")
            .path,
        "/home/user/.matrix/services/db"
    );
}

#[test]
fn default_config_path_points_at_home_matrix_dir() {
    let path = crate::config::default_config_path();
    let s = path.to_string_lossy();
    assert!(
        s.ends_with(".matrix/matrix.json"),
        "unexpected default config path: {s}"
    );
    assert!(
        s.starts_with('/'),
        "expected an absolute per-device path, got: {s}"
    );
}
