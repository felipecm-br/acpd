use std::fs;
use std::path::PathBuf;

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";

#[derive(Debug, Clone)]
pub struct ThemePalette {
    pub orange: String,
    pub yellow: String,
    pub red: String,
    pub magenta: String,
    pub cyan: String,
    pub blue: String,
    pub fg: String,
    pub muted: String,

    pub c_perm: String,
    pub c_question: String,
    pub c_error: String,
    pub c_busy: String,
    pub c_idle: String,
    pub c_cyan: String,
    pub c_normal: String,
    pub c_cur: String,
    pub c_name: String,
    pub c_branch: String,
    pub c_muted: String,
    pub c_diff_ins: String,
    pub c_diff_del: String,
    pub c_diff_dirty: String,
    pub c_pr: String,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self::from_hex_values(
            "#e09d7f", "#dbbc7f", "#e67e80", "#d699b6", "#83c092", "#7fbbb3", "#d3c6aa", "#475258",
        )
    }
}

impl ThemePalette {
    #[allow(clippy::too_many_arguments)]
    pub fn from_hex_values(
        orange: &str,
        yellow: &str,
        red: &str,
        magenta: &str,
        cyan: &str,
        blue: &str,
        fg: &str,
        muted: &str,
    ) -> Self {
        let c_perm = hex_to_ansi_fg(orange);
        let c_question = hex_to_ansi_fg(magenta);
        let c_error = hex_to_ansi_fg(red);
        let c_busy = hex_to_ansi_fg(yellow);
        let c_idle = hex_to_ansi_fg(cyan);
        let c_cyan = c_idle.clone();
        let c_normal = hex_to_ansi_fg(muted);
        let c_cur = c_idle.clone();
        let c_name = hex_to_ansi_fg(fg);
        let c_branch = hex_to_ansi_fg(blue);
        let c_muted = c_normal.clone();
        let c_diff_ins = c_idle.clone();
        let c_diff_del = c_error.clone();
        let c_diff_dirty = c_busy.clone();
        let c_pr = c_question.clone();

        Self {
            orange: orange.to_string(),
            yellow: yellow.to_string(),
            red: red.to_string(),
            magenta: magenta.to_string(),
            cyan: cyan.to_string(),
            blue: blue.to_string(),
            fg: fg.to_string(),
            muted: muted.to_string(),
            c_perm,
            c_question,
            c_error,
            c_busy,
            c_idle,
            c_cyan,
            c_normal,
            c_cur,
            c_name,
            c_branch,
            c_muted,
            c_diff_ins,
            c_diff_del,
            c_diff_dirty,
            c_pr,
        }
    }

    pub fn load() -> Self {
        let mut orange = "#e09d7f".to_string();
        let mut yellow = "#dbbc7f".to_string();
        let mut red = "#e67e80".to_string();
        let mut magenta = "#d699b6".to_string();
        let mut cyan = "#83c092".to_string();
        let mut blue = "#7fbbb3".to_string();
        let mut fg = "#d3c6aa".to_string();
        let mut muted = "#475258".to_string();

        if let Ok(home) = std::env::var("HOME") {
            let path = PathBuf::from(home).join(".local/state/omarchy/current/theme/colors.toml");
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    if let Some((k, v)) = trimmed.split_once('=') {
                        let key = k.trim();
                        let val = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
                        if val.is_empty() {
                            continue;
                        }
                        match key {
                            "orange" => orange = val.to_string(),
                            "yellow" => yellow = val.to_string(),
                            "red" => red = val.to_string(),
                            "magenta" => magenta = val.to_string(),
                            "cyan" => cyan = val.to_string(),
                            "blue" => blue = val.to_string(),
                            "foreground" => fg = val.to_string(),
                            "muted" => muted = val.to_string(),
                            _ => {}
                        }
                    }
                }
            }
        }

        Self::from_hex_values(&orange, &yellow, &red, &magenta, &cyan, &blue, &fg, &muted)
    }
}

pub fn hex_to_ansi_fg(hex: &str) -> String {
    let raw = hex.strip_prefix('#').unwrap_or(hex);
    let clean = if raw.len() == 6 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        raw
    } else {
        "89b4fa"
    };
    let r = u8::from_str_radix(&clean[0..2], 16).unwrap_or(137);
    let g = u8::from_str_radix(&clean[2..4], 16).unwrap_or(180);
    let b = u8::from_str_radix(&clean[4..6], 16).unwrap_or(250);
    format!("\x1b[38;2;{};{};{}m", r, g, b)
}

pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if ch != '\r' && ch != '\n' {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_to_ansi_fg() {
        assert_eq!(hex_to_ansi_fg("#e09d7f"), "\x1b[38;2;224;157;127m");
        assert_eq!(hex_to_ansi_fg("83c092"), "\x1b[38;2;131;192;146m");
        assert_eq!(hex_to_ansi_fg("invalid"), "\x1b[38;2;137;180;250m");
    }

    #[test]
    fn test_strip_ansi() {
        let s = "\x1b[1m\x1b[38;2;131;192;146mHello\x1b[0m World";
        assert_eq!(strip_ansi(s), "Hello World");
    }
}
