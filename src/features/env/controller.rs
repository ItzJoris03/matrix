use super::model::EnvModel;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;

pub enum EnvAction {
    None,
    Exit,
    Message(String),
}

pub struct EnvController;

impl EnvController {
    pub fn handle_key(key: KeyEvent, model: &mut EnvModel) -> EnvAction {
        match key.code {
            KeyCode::Esc => EnvAction::Exit,
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match fs::write(&model.file_path, model.get_content()) {
                    Ok(_) => EnvAction::Message(format!("Saved {}", model.file_path)),
                    Err(e) => EnvAction::Message(format!("Save failed: {}", e)),
                }
            }
            KeyCode::Up => {
                model.move_up();
                EnvAction::None
            }
            KeyCode::Down => {
                model.move_down();
                EnvAction::None
            }
            KeyCode::Left => {
                model.move_left();
                EnvAction::None
            }
            KeyCode::Right => {
                model.move_right();
                EnvAction::None
            }
            KeyCode::Backspace => {
                model.backspace();
                EnvAction::None
            }
            KeyCode::Enter => {
                model.insert_newline();
                EnvAction::None
            }
            KeyCode::Char(c) => {
                model.insert_char(c);
                EnvAction::None
            }
            _ => EnvAction::None,
        }
    }
}
