pub struct EnvModel {
    pub project_id: String,
    pub file_path: String,
    pub lines: Vec<String>,
    pub cursor_y: usize,
    pub cursor_x: usize,
    pub scroll_offset: usize,
}

impl EnvModel {
    pub fn new() -> Self {
        Self {
            project_id: String::new(),
            file_path: String::new(),
            lines: vec![String::new()],
            cursor_y: 0,
            cursor_x: 0,
            scroll_offset: 0,
        }
    }

    pub fn load(&mut self, project_id: String, file_path: String, content: String) {
        self.project_id = project_id;
        self.file_path = file_path;
        self.lines = content.lines().map(|s| s.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_y = 0;
        self.cursor_x = 0;
        self.scroll_offset = 0;
    }

    pub fn move_up(&mut self) {
        if self.cursor_y > 0 {
            self.cursor_y -= 1;
            self.cursor_x = self.cursor_x.min(self.lines[self.cursor_y].len());
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_y < self.lines.len() - 1 {
            self.cursor_y += 1;
            self.cursor_x = self.cursor_x.min(self.lines[self.cursor_y].len());
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
        } else if self.cursor_y > 0 {
            self.cursor_y -= 1;
            self.cursor_x = self.lines[self.cursor_y].len();
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor_x < self.lines[self.cursor_y].len() {
            self.cursor_x += 1;
        } else if self.cursor_y < self.lines.len() - 1 {
            self.cursor_y += 1;
            self.cursor_x = 0;
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.lines[self.cursor_y].insert(self.cursor_x, c);
        self.cursor_x += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor_x > 0 {
            self.lines[self.cursor_y].remove(self.cursor_x - 1);
            self.cursor_x -= 1;
        } else if self.cursor_y > 0 {
            let current_line = self.lines.remove(self.cursor_y);
            self.cursor_y -= 1;
            self.cursor_x = self.lines[self.cursor_y].len();
            self.lines[self.cursor_y].push_str(&current_line);
        }
    }

    pub fn insert_newline(&mut self) {
        let current_line = &mut self.lines[self.cursor_y];
        let new_line = current_line.split_off(self.cursor_x);
        self.lines.insert(self.cursor_y + 1, new_line);
        self.cursor_y += 1;
        self.cursor_x = 0;
    }

    pub fn get_content(&self) -> String {
        self.lines.join("\n")
    }
}
