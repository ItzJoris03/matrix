use super::model::DashboardModel;
use crossterm::event::{KeyCode, MouseEvent, MouseEventKind};

pub struct DashboardController;

impl DashboardController {
    pub fn handle_key(key: KeyCode, model: &mut DashboardModel) {
        match key {
            KeyCode::Down | KeyCode::Char('j') => {
                model.scroll_down();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                model.scroll_up();
            }
            KeyCode::PageDown => {
                model.dashboard_scroll = model.dashboard_scroll.saturating_add(5);
            }
            KeyCode::PageUp => {
                model.dashboard_scroll = model.dashboard_scroll.saturating_sub(5);
            }
            _ => {}
        }
    }

    pub fn handle_mouse(mouse: MouseEvent, model: &mut DashboardModel) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                model.scroll_down();
            }
            MouseEventKind::ScrollUp => {
                model.scroll_up();
            }
            _ => {}
        }
    }
}
