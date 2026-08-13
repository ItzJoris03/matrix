pub struct DashboardModel {
    pub dashboard_scroll: u16,
}

impl DashboardModel {
    pub fn new() -> Self {
        Self {
            dashboard_scroll: 0,
        }
    }

    pub fn scroll_down(&mut self) {
        self.dashboard_scroll = self.dashboard_scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.dashboard_scroll = self.dashboard_scroll.saturating_sub(1);
    }
}
