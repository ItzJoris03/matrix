use regex::Regex;
use std::sync::OnceLock;

static URL_RE: OnceLock<Regex> = OnceLock::new();

fn get_url_regex() -> &'static Regex {
    URL_RE.get_or_init(|| Regex::new(r"https?://[^\s\x1b\]\)]+").unwrap())
}

/// Represents a detected URL within a line of text.
#[derive(Clone, Debug)]
pub struct UrlMatch {
    /// The URL string
    pub url: String,
    /// Start column (byte offset) in the rendered line
    pub start: usize,
    /// End column (byte offset, exclusive)
    pub end: usize,
}

/// Find all URLs in a line and return their positions.
pub fn find_urls(line: &str) -> Vec<UrlMatch> {
    let re = get_url_regex();
    re.find_iter(line)
        .map(|m| {
            let url = m
                .as_str()
                .trim_end_matches(['.', ',', ')', ']'])
                .to_string();
            let start = m.start();
            let end = start + url.len();
            UrlMatch { url, start, end }
        })
        .collect()
}

/// Check if a column position falls within any URL in the line.
pub fn url_at_column(urls: &[UrlMatch], col: usize) -> Option<&UrlMatch> {
    urls.iter().find(|u| col >= u.start && col < u.end)
}

/// Open a URL in the default browser using the platform's default opener.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    match std::env::consts::OS {
        "linux" => {
            std::process::Command::new("xdg-open").arg(url).spawn()?;
        }
        "macos" => {
            std::process::Command::new("open").arg(url).spawn()?;
        }
        "windows" => {
            std::process::Command::new("cmd")
                .args(["/C", "start", url])
                .spawn()?;
        }
        _ => {
            std::process::Command::new("xdg-open").arg(url).spawn()?;
        }
    }
    Ok(())
}
