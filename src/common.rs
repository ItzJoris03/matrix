#[derive(PartialEq, Debug, Clone, Copy)]
pub enum ActiveView {
    Dashboard,
    Projects,
    Logs,
    EnvEditor,
}

// ── Shared theme tokens (single source of truth for the UI) ──
pub mod theme {
    use ratatui::style::Color;

    pub const PURPLE: Color = Color::Rgb(138, 43, 226);
    pub const PURPLE_DIM: Color = Color::Rgb(90, 40, 160);
    pub const GRAY: Color = Color::Rgb(60, 60, 70);
    pub const GRAY_DIM: Color = Color::Rgb(40, 40, 48);
    pub const BORDER: Color = Color::Rgb(48, 48, 58);
    pub const TEXT: Color = Color::Rgb(220, 220, 230);
    pub const TEXT_DIM: Color = Color::Rgb(130, 130, 145);
    pub const GREEN: Color = Color::Rgb(80, 220, 120);
    pub const RED: Color = Color::Rgb(240, 90, 90);
    pub const YELLOW: Color = Color::Rgb(240, 200, 90);
    pub const CYAN: Color = Color::Rgb(90, 200, 230);
    pub const MAGENTA: Color = Color::Rgb(220, 120, 220);
}

pub fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Start of an escape sequence
            match chars.next() {
                Some('[') => {
                    // Control Sequence Introducer (CSI)
                    // Consume everything until a "final character" (@ to ~)
                    for next_c in chars.by_ref() {
                        if (0x40..=0x7E).contains(&(next_c as u8)) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // Operating System Command (OSC)
                    // Consume until BEL (\x07) or ST (ESC \)
                    while let Some(next_c) = chars.next() {
                        if next_c == '\x07' {
                            break;
                        }
                        if next_c == '\x1b' {
                            if let Some('\\') = chars.peek() {
                                chars.next();
                                break;
                            }
                        }
                    }
                }
                Some('(') | Some(')') | Some('*') | Some('+') | Some('-') | Some('.')
                | Some('/') => {
                    // G0-G3 character sets, etc.
                    chars.next();
                }
                _ => {
                    // Other escape sequences (usually 1-2 chars)
                }
            }
        } else {
            output.push(c);
        }
    }
    output
}

/// Get the local LAN IP address by opening a UDP "connection" to a
/// non-routable address and reading the socket's local address.
/// This avoids needing any external command or new dependency.
pub fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // 10.255.255.255 is non-routable; no actual packet is sent
    socket.connect("10.255.255.255:1").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

/// Severity of a toast notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTone {
    Info,
    Success,
    Warn,
    Error,
}

/// A transient notification routed from any actor (TUI key, CLI/socket command)
/// into the live TUI so the user sees external actions immediately.
#[derive(Clone, Debug)]
pub struct ToastEvent {
    /// Origin label shown in the toast, e.g. "tui", "socket".
    pub source: String,
    /// Human-readable message (single line).
    pub message: String,
    pub tone: ToastTone,
}
