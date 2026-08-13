use super::model::ProjectsModel;
use crate::config::Group;
use crate::engine::{ProcessManager, ProcessStatus};
use crossterm::event::KeyCode;

pub enum ProjectAction {
    None,
    Message(String),
    SaveConfigWithMsg(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectionItem {
    Group {
        id: String,
        name: String,
        expanded: bool,
        running: bool,
    },
    Project {
        original_index: usize,
        group_id: Option<String>,
        is_infra: bool,
    },
    StandaloneHeader,
}

pub struct ProjectsController;

impl ProjectsController {
    /// Whether a group's OWN projects (plus their auto-provisioned engines) are running.
    /// Infrastructure is intentionally excluded — it is a shared global dependency, so a
    /// group is not "running" merely because shared infrastructure is up.
    fn group_has_running_projects(manager: &ProcessManager, group: &Group) -> bool {
        let statuses = manager.get_statuses();
        group.projects.iter().any(|pid| {
            let engine_id = format!("engine:{}", pid);
            statuses.iter().any(|(c, s)| {
                (c.id == *pid || c.id == engine_id) && matches!(s, ProcessStatus::Running(_))
            })
        })
    }

    pub async fn handle_key(
        key: KeyCode,
        model: &mut ProjectsModel,
        manager: &ProcessManager,
    ) -> ProjectAction {
        let items = Self::get_all_items(model, manager);
        let total_items = items.len();

        // Handle Port/Category Editing Mode (only applies when a Project is selected)
        if model.editing_port.is_some() || model.editing_category.is_some() {
            if let Some(SelectionItem::Project { original_index, .. }) =
                items.get(model.selected_index)
            {
                let statuses = manager.get_statuses();
                if let Some((config, _)) = statuses.get(*original_index) {
                    // Port Editing
                    if let Some(ref mut input) = model.editing_port {
                        match key {
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                if input.len() < 5 {
                                    input.push(c);
                                }
                                return ProjectAction::None;
                            }
                            KeyCode::Backspace => {
                                input.pop();
                                return ProjectAction::None;
                            }
                            KeyCode::Char('x') | KeyCode::Delete => {
                                input.clear();
                                return ProjectAction::None;
                            }
                            KeyCode::Enter => match model.stop_editing_port() {
                                Ok(port) => {
                                    manager.update_project_port(&config.id, port);
                                    let msg = if let Some(p) = port {
                                        format!("Port for {} set to {}", config.id, p)
                                    } else {
                                        format!("Port for {} removed", config.id)
                                    };
                                    return ProjectAction::SaveConfigWithMsg(msg);
                                }
                                Err(e) => return ProjectAction::Message(e),
                            },
                            KeyCode::Esc => {
                                model.cancel_editing();
                                return ProjectAction::None;
                            }
                            _ => return ProjectAction::None,
                        }
                    }
                    // Category Editing
                    if let Some(ref mut input) = model.editing_category {
                        match key {
                            KeyCode::Char(c) => {
                                input.push(c);
                                return ProjectAction::None;
                            }
                            KeyCode::Backspace => {
                                input.pop();
                                return ProjectAction::None;
                            }
                            KeyCode::Delete => {
                                input.clear();
                                return ProjectAction::None;
                            }
                            KeyCode::Enter => {
                                let cat = model.stop_editing_category();
                                manager.update_project_category(&config.id, cat.clone());
                                let msg = if let Some(c) = cat {
                                    format!("Category for {} set to {}", config.id, c)
                                } else {
                                    format!("Category for {} removed", config.id)
                                };
                                return ProjectAction::SaveConfigWithMsg(msg);
                            }
                            KeyCode::Esc => {
                                model.cancel_editing();
                                return ProjectAction::None;
                            }
                            _ => return ProjectAction::None,
                        }
                    }
                }
            }
        }

        // Normal Navigation Mode
        match key {
            KeyCode::Down | KeyCode::Char('j') => {
                model.cancel_editing();
                model.next(total_items);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                model.cancel_editing();
                model.prev(total_items);
            }
            KeyCode::Char('p') => {
                if let Some(SelectionItem::Project { original_index, .. }) =
                    items.get(model.selected_index)
                {
                    let statuses = manager.get_statuses();
                    if let Some((config, _)) = statuses.get(*original_index) {
                        model.start_editing_port(config.port);
                    }
                }
            }
            KeyCode::Char('c') => {
                if let Some(SelectionItem::Project { original_index, .. }) =
                    items.get(model.selected_index)
                {
                    let statuses = manager.get_statuses();
                    if let Some((config, _)) = statuses.get(*original_index) {
                        model.start_editing_category(config.category.clone());
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(item) = items.get(model.selected_index) {
                    match item {
                        SelectionItem::Group { id, .. } => {
                            if let Some(group) = manager.get_group(id) {
                                let is_running = Self::group_has_running_projects(manager, &group);
                                if is_running {
                                    let _ = manager.stop_group(id).await;
                                    return ProjectAction::Message(format!("Group {} stopped", id));
                                }
                            }
                            // Start the group
                            match manager.start_group(id).await {
                                Ok(_) => {
                                    return ProjectAction::Message(format!("Group {} started", id))
                                }
                                Err(e) => return ProjectAction::Message(format!("Failed: {}", e)),
                            }
                        }
                        SelectionItem::Project { original_index, .. } => {
                            let statuses = manager.get_statuses();
                            if let Some((config, status)) = statuses.get(*original_index) {
                                if matches!(status, ProcessStatus::Stopped) {
                                    if let Err(e) = manager.start(&config.id) {
                                        return ProjectAction::Message(format!(
                                            "Failed to start: {}",
                                            e
                                        ));
                                    }
                                } else {
                                    let _ = manager.stop(&config.id).await;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(item) = items.get(model.selected_index) {
                    match item {
                        SelectionItem::Group { id, .. } => {
                            // Restart group: stop then start
                            let _ = manager.stop_group(id).await;
                            match manager.start_group(id).await {
                                Ok(_) => {
                                    return ProjectAction::Message(format!(
                                        "Group {} restarted",
                                        id
                                    ))
                                }
                                Err(e) => return ProjectAction::Message(format!("Failed: {}", e)),
                            }
                        }
                        SelectionItem::Project { original_index, .. } => {
                            let statuses = manager.get_statuses();
                            if let Some((config, _)) = statuses.get(*original_index) {
                                let _ = manager.stop(&config.id).await;
                                if let Err(e) = manager.start(&config.id) {
                                    return ProjectAction::Message(format!(
                                        "Failed to start: {}",
                                        e
                                    ));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('e') => {
                if let Some(SelectionItem::Group { id, .. }) = items.get(model.selected_index) {
                    model.toggle_group(id);
                }
            }
            _ => {}
        }
        ProjectAction::None
    }

    pub fn get_all_items(model: &ProjectsModel, manager: &ProcessManager) -> Vec<SelectionItem> {
        let mut items = Vec::new();
        let groups = manager.get_groups();
        let statuses = manager.get_statuses();

        // Build a set of project IDs that belong to any group
        let grouped_project_ids: std::collections::HashSet<String> = groups
            .iter()
            .flat_map(|g| {
                let mut ids: Vec<String> = g.projects.clone();
                ids.extend(g.infrastructure.clone());
                ids
            })
            .collect();

        // Render groups first
        for group in &groups {
            let is_expanded = model.is_group_expanded(&group.id);
            let is_running = Self::group_has_running_projects(manager, group);

            items.push(SelectionItem::Group {
                id: group.id.clone(),
                name: group.name.clone(),
                expanded: is_expanded,
                running: is_running,
            });

            if is_expanded {
                // Infrastructure projects
                for infra_id in &group.infrastructure {
                    if statuses.iter().find(|(c, _)| &c.id == infra_id).is_some() {
                        items.push(SelectionItem::Project {
                            original_index: statuses
                                .iter()
                                .position(|(c, _)| &c.id == infra_id)
                                .unwrap_or(0),
                            group_id: Some(group.id.clone()),
                            is_infra: true,
                        });
                    }
                }
                // Regular projects
                for project_id in &group.projects {
                    if let Some(pos) = statuses.iter().position(|(c, _)| &c.id == project_id) {
                        items.push(SelectionItem::Project {
                            original_index: pos,
                            group_id: Some(group.id.clone()),
                            is_infra: false,
                        });
                    }
                    // Also show the auto-provisioned engine
                    let engine_id = format!("engine:{}", project_id);
                    if let Some(pos) = statuses.iter().position(|(c, _)| c.id == engine_id) {
                        items.push(SelectionItem::Project {
                            original_index: pos,
                            group_id: Some(group.id.clone()),
                            is_infra: false,
                        });
                    }
                }
            }
        }

        // Standalone projects (not in any group)
        let standalone_projects: Vec<_> = statuses
            .iter()
            .filter(|(c, _)| !grouped_project_ids.contains(&c.id) && !c.id.starts_with("engine:"))
            .collect();

        if !standalone_projects.is_empty() {
            items.push(SelectionItem::StandaloneHeader);
            for (config, _) in standalone_projects {
                if let Some(pos) = statuses.iter().position(|(c, _)| c.id == config.id) {
                    items.push(SelectionItem::Project {
                        original_index: pos,
                        group_id: None,
                        is_infra: false,
                    });
                }
            }
        }

        items
    }
}
