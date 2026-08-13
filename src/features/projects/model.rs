pub struct ProjectsModel {
    pub selected_index: usize,
    pub editing_port: Option<String>,
    pub editing_category: Option<String>,
    pub expanded_groups: Vec<String>, // Track which groups are expanded
}

impl ProjectsModel {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            editing_port: None,
            editing_category: None,
            expanded_groups: vec![],
        }
    }

    pub fn next(&mut self, count: usize) {
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    pub fn prev(&mut self, count: usize) {
        if count > 0 {
            self.selected_index = (self.selected_index + count - 1) % count;
        }
    }

    pub fn toggle_group(&mut self, group_id: &str) {
        if self.expanded_groups.contains(&group_id.to_string()) {
            self.expanded_groups.retain(|g| g != group_id);
        } else {
            self.expanded_groups.push(group_id.to_string());
        }
    }

    pub fn is_group_expanded(&self, group_id: &str) -> bool {
        self.expanded_groups.contains(&group_id.to_string())
    }

    pub fn start_editing_port(&mut self, current_port: Option<u16>) {
        self.editing_port = Some(current_port.map(|p| p.to_string()).unwrap_or_default());
        self.editing_category = None;
    }

    pub fn stop_editing_port(&mut self) -> Result<Option<u16>, String> {
        let s = self.editing_port.take().unwrap_or_default();
        if s.is_empty() {
            Ok(None)
        } else {
            match s.parse::<u16>() {
                Ok(p) => Ok(Some(p)),
                Err(_) => Err("Invalid port number".to_string()),
            }
        }
    }

    pub fn start_editing_category(&mut self, current_category: Option<String>) {
        self.editing_category = Some(current_category.unwrap_or_default());
        self.editing_port = None;
    }

    pub fn stop_editing_category(&mut self) -> Option<String> {
        let s = self.editing_category.take().unwrap_or_default();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn cancel_editing(&mut self) {
        self.editing_port = None;
        self.editing_category = None;
    }
}
