use std::sync::{Arc, Mutex};
use std::thread;

use crate::common::strip_ansi;
use crate::url::{find_urls, UrlMatch};

/// The processed, ready-to-render version of log lines.
#[derive(Clone)]
pub struct ProcessedLine {
    pub orig_idx: usize,
    pub content: String,
    pub style: ratatui::style::Style,
    pub urls: Vec<UrlMatch>,
}

/// Shared cache between the processor thread and the render thread.
#[derive(Clone)]
pub struct SharedLogCache {
    pub project_id: String,
    pub width: usize,
    pub logs_len: usize,
    pub processing_rev: u64,
    pub lines: Vec<ProcessedLine>,
    pub line_map: Vec<usize>, // maps screen row -> original log index
    pub processing: bool,     // true if the worker is currently recomputing
}

impl SharedLogCache {
    pub fn new() -> Self {
        Self {
            project_id: String::new(),
            width: 0,
            logs_len: 0,
            processing_rev: 0,
            lines: Vec::new(),
            line_map: Vec::new(),
            processing: false,
        }
    }

    pub fn is_valid(&self, project_id: &str, width: usize, logs_len: usize, rev: u64) -> bool {
        self.project_id == project_id
            && self.width == width
            && self.logs_len == logs_len
            && self.processing_rev == rev
            && !self.processing
    }
}

/// Command sent to the processor thread.
pub enum ProcessorCommand {
    /// Re-process the given log lines for the given project/width.
    /// `rev` is the engine's log revision at capture time, used for cache validity.
    Process {
        project_id: String,
        width: usize,
        logs: Vec<String>,
        rev: u64,
    },
}

/// Background log processor that pre-computes word wrapping and URL detection.
pub struct LogProcessor {
    sender: std::sync::mpsc::Sender<ProcessorCommand>,
    cache: Arc<Mutex<SharedLogCache>>,
}

impl LogProcessor {
    pub fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::channel::<ProcessorCommand>();
        let cache = Arc::new(Mutex::new(SharedLogCache::new()));
        let cache_clone = Arc::clone(&cache);

        thread::Builder::new()
            .name("log-processor".to_string())
            .spawn(move || {
                // Hold the lock only while writing; process data outside the lock.
                while let Ok(ProcessorCommand::Process {
                    project_id,
                    width,
                    logs,
                    rev,
                }) = receiver.recv()
                {
                    // Coalesce bursts: while we were about to start, more log lines may
                    // have queued additional Process commands. Drain them and keep only
                    // the newest, so a burst of N new lines becomes a single recompute
                    // instead of N — prevents the worker from falling behind and the
                    // on-screen log from stalling under heavy output.
                    let mut latest = (project_id, width, logs, rev);
                    while let Ok(next) = receiver.try_recv() {
                        let ProcessorCommand::Process {
                            project_id: np,
                            width: nw,
                            logs: nl,
                            rev: nr,
                        } = next;
                        latest = (np, nw, nl, nr);
                    }
                    let (project_id, width, logs, rev) = latest;

                    // Mark as "processing" so render knows the cache is stale.
                    {
                        let mut c = cache_clone.lock().unwrap();
                        c.processing = true;
                        c.processing_rev = rev;
                    }

                    // Heavy computation — no lock held.
                    let (lines, line_map) = process_logs(&logs, width);

                    let mut c = cache_clone.lock().unwrap();
                    c.project_id = project_id;
                    c.width = width;
                    c.logs_len = logs.len();
                    c.processing_rev = rev;
                    c.lines = lines;
                    c.line_map = line_map;
                    c.processing = false;
                }
            })
            .expect("failed to spawn log processor thread");

        LogProcessor { sender, cache }
    }

    /// Trigger a re-processing of the logs. Non-blocking.
    pub fn process(&self, project_id: String, width: usize, logs: Vec<String>, rev: u64) {
        let _ = self.sender.send(ProcessorCommand::Process {
            project_id,
            width,
            logs,
            rev,
        });
    }

    /// Get a snapshot of the current cache.
    pub fn snapshot(&self) -> SharedLogCache {
        self.cache.lock().unwrap().clone()
    }
}

/// Process raw log lines: strip ANSI, word-wrap, detect URLs, compute styles.
/// Returns (processed_lines, line_map) where line_map maps screen row -> orig index.
pub fn process_logs(raw_logs: &[String], width: usize) -> (Vec<ProcessedLine>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut line_map = Vec::new();

    for (orig_idx, raw_line) in raw_logs.iter().enumerate() {
        let clean = strip_ansi(raw_line);
        if clean.is_empty() {
            lines.push(ProcessedLine {
                orig_idx,
                content: String::new(),
                style: ratatui::style::Style::default().fg(ratatui::style::Color::White),
                urls: Vec::new(),
            });
            line_map.push(orig_idx);
            continue;
        }

        let line_urls = find_urls(&clean);
        let style = get_base_style(&clean, raw_line);

        // Word wrap
        let mut current_line = String::new();
        let mut current_line_len = 0;
        let mut current_col_offset: usize = 0;

        for word in clean.split_inclusive(' ') {
            let word_len = word.len();

            if current_line_len + word_len <= width {
                current_line.push_str(word);
                current_line_len += word_len;
            } else {
                if !current_line.is_empty() {
                    let segment_urls =
                        remap_urls(&line_urls, current_col_offset, current_line.len());
                    lines.push(ProcessedLine {
                        orig_idx,
                        content: current_line.clone(),
                        style,
                        urls: segment_urls,
                    });
                    line_map.push(orig_idx);
                    current_line.clear();
                    current_col_offset += current_line_len;
                }

                if word_len > width {
                    let mut remaining = word;
                    while remaining.len() > width {
                        let (head, tail) = remaining.split_at(width);
                        lines.push(ProcessedLine {
                            orig_idx,
                            content: head.to_string(),
                            style,
                            urls: Vec::new(),
                        });
                        line_map.push(orig_idx);
                        remaining = tail;
                        current_col_offset += width;
                    }
                    current_line.push_str(remaining);
                    current_line_len = remaining.len();
                } else {
                    current_line.push_str(word);
                    current_line_len = word_len;
                }
            }
        }

        if !current_line.is_empty() {
            let segment_urls = remap_urls(&line_urls, current_col_offset, current_line.len());
            lines.push(ProcessedLine {
                orig_idx,
                content: current_line,
                style,
                urls: segment_urls,
            });
            line_map.push(orig_idx);
        }
    }

    (lines, line_map)
}

/// Remap URL positions for a wrapped segment starting at `offset` with length `seg_len`.
fn remap_urls(urls: &[UrlMatch], offset: usize, seg_len: usize) -> Vec<UrlMatch> {
    urls.iter()
        .filter_map(|u| {
            let rel_start = u.start.saturating_sub(offset);
            let rel_end = u.end.saturating_sub(offset);
            if rel_start < seg_len {
                let clamped_end = rel_end.min(seg_len);
                if rel_start < clamped_end {
                    Some(UrlMatch {
                        url: u.url.clone(),
                        start: rel_start,
                        end: clamped_end,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}

fn get_base_style(content: &str, original_full_line: &str) -> ratatui::style::Style {
    let lower = content.to_lowercase();
    let is_err_tagged = original_full_line.contains("[ERR]");

    let fg_color = if lower.contains("compiling")
        || lower.contains("finished")
        || lower.contains("running")
        || lower.contains("building")
    {
        ratatui::style::Color::DarkGray
    } else if lower.contains("error")
        || lower.contains("fail")
        || lower.contains("critical")
        || lower.contains("crashed")
        || lower.contains("panic")
    {
        ratatui::style::Color::Red
    } else if lower.contains("success")
        || lower.contains("done")
        || lower.contains("ok")
        || lower.contains("completed")
    {
        ratatui::style::Color::Green
    } else if lower.contains("warn") || lower.contains("warning") {
        ratatui::style::Color::Yellow
    } else if lower.contains("info") || lower.contains("debug") {
        ratatui::style::Color::Cyan
    } else if is_err_tagged {
        ratatui::style::Color::Red
    } else {
        ratatui::style::Color::White
    };

    ratatui::style::Style::default().fg(fg_color)
}
