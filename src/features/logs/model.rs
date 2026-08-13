use super::processor::LogProcessor;
use crate::url::UrlMatch;
use ratatui::layout::Rect;

pub struct LogsModel {
    pub log_sidebar_index: usize,
    pub log_scroll: u16,
    pub auto_scroll_logs: bool,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,

    // Render-synchronized state for mouse mapping
    pub last_log_area: Rect,
    pub last_rendered_scroll: usize,
    pub last_logs_len: usize,
    pub last_line_map: Vec<usize>, // Maps screen row index to original log index

    // Cache fields
    pub cache_project_id: String,
    pub cache_width: usize,
    pub cache_logs_len: usize,
    // Last request dispatched to the processor thread (dedupes per-frame resends).
    pub req_rev: u64,
    pub req_project_id: String,
    pub req_width: usize,
    pub cache_wrapped: Vec<(usize, String, ratatui::style::Style)>,
    pub cache_urls: Vec<(usize, Vec<UrlMatch>)>, // (wrapped_line_index, urls)

    // Background processor for expensive log processing
    pub processor: LogProcessor,
}

impl LogsModel {
    pub fn new() -> Self {
        Self {
            log_sidebar_index: 1,
            log_scroll: 0,
            auto_scroll_logs: true,
            selection_start: None,
            selection_end: None,
            last_log_area: Rect::default(),
            last_rendered_scroll: 0,
            last_logs_len: 0,
            last_line_map: Vec::new(),
            cache_project_id: String::new(),
            cache_width: 0,
            cache_logs_len: 0,
            req_rev: 0,
            req_project_id: String::new(),
            req_width: 0,
            cache_wrapped: Vec::new(),
            cache_urls: Vec::new(),
            processor: LogProcessor::new(),
        }
    }

    pub fn scroll_down(&mut self, max_scroll: u16) {
        if self.log_scroll >= max_scroll {
            self.auto_scroll_logs = true;
        } else {
            self.log_scroll = self.log_scroll.saturating_add(1);
            if self.log_scroll >= max_scroll {
                self.auto_scroll_logs = true;
            }
        }
    }

    pub fn scroll_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(1);
        self.auto_scroll_logs = false;
    }

    pub fn scroll_to_bottom(&mut self, max_scroll: u16) {
        self.log_scroll = max_scroll;
        self.auto_scroll_logs = true;
    }

    pub fn scroll_to_top(&mut self) {
        self.log_scroll = 0;
        self.auto_scroll_logs = false;
    }

    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }
}
