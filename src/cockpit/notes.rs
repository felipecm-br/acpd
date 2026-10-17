use crate::tmux_cmd::{tmux_display_msg, tmux_load_and_paste_bracketed, tmux_out, tmux_status};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn default_status() -> String {
    "New".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewNote {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub line: u64,
    #[serde(default)]
    pub comment: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotesFile {
    #[serde(default)]
    pub notes: Vec<ReviewNote>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub fn resolve_notes_file(dir: &str) -> Option<PathBuf> {
    if dir.is_empty() || dir == "-" {
        return None;
    }
    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        return None;
    }

    if let Ok(out) = Command::new("git")
        .args(["-C", dir, "rev-parse", "--git-dir"])
        .stderr(Stdio::null())
        .output()
        && out.status.success()
    {
        let raw_git = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !raw_git.is_empty() {
            let git_dir = if Path::new(&raw_git).is_absolute() {
                PathBuf::from(&raw_git)
            } else {
                dir_path.join(&raw_git)
            };
            let info_lines = git_dir.join("info").join("lines.json");
            if info_lines.is_file() {
                return Some(info_lines);
            }
            let direct_lines = git_dir.join("lines.json");
            if direct_lines.is_file() {
                return Some(direct_lines);
            }
        }
    }

    let legacy = dir_path.join(".lines.json");
    if legacy.is_file() {
        return Some(legacy);
    }

    None
}

pub fn load_notes(path: &Path) -> Option<NotesFile> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn count_pending_in_file(path: &Path) -> usize {
    load_notes(path)
        .map(|data| data.notes.iter().filter(|n| n.status == "New").count())
        .unwrap_or(0)
}

pub fn render_pending_notes(ppath: &str) -> String {
    let notes_file = match resolve_notes_file(ppath) {
        Some(p) => p,
        None => return String::new(),
    };
    let data = match load_notes(&notes_file) {
        Some(d) => d,
        None => return String::new(),
    };
    let pending: Vec<&ReviewNote> = data.notes.iter().filter(|n| n.status == "New").collect();
    if pending.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "\x1b[1;33m  󰆈 Pending AI Review Notes ({}):\x1b[0m\n",
        pending.len()
    ));
    for n in pending.iter().take(5) {
        let f = if n.file.is_empty() {
            "unknown"
        } else {
            &n.file
        };
        let l = n.line;
        let clean_c = n.comment.replace(['\n', '\r'], " ");
        let c_trunc = if clean_c.chars().count() > 60 {
            let s: String = clean_c.chars().take(57).collect();
            format!("{}...", s)
        } else {
            clean_c
        };
        out.push_str(&format!(
            "    \x1b[1;36m• {}:{}\x1b[0m \x1b[2m›\x1b[0m \"{}\"\n",
            f, l, c_trunc
        ));
    }
    if pending.len() > 5 {
        out.push_str(&format!(
            "    \x1b[2m... and {} more notes\x1b[0m\n",
            pending.len() - 5
        ));
    }
    out.push_str(
        "    \x1b[2m(Press \x1b[1;36mr\x1b[0;2m to dispatch all review notes to agent)\x1b[0m\n\n",
    );
    out
}

pub fn format_and_mark_sent(data: &mut NotesFile) -> Option<(usize, String)> {
    let pending: Vec<ReviewNote> = data
        .notes
        .iter()
        .filter(|n| n.status == "New")
        .cloned()
        .collect();
    if pending.is_empty() {
        return None;
    }

    let mut lines = vec![format!(
        "Please review and address the following {} code review comments from lazygitrs:\n",
        pending.len()
    )];
    for (i, n) in pending.iter().enumerate() {
        let f = if n.file.is_empty() {
            "unknown"
        } else {
            &n.file
        };
        lines.push(format!(
            "{}. **`{}`** (line {}):\n   > {}\n",
            i + 1,
            f,
            n.line,
            n.comment
        ));
    }
    lines.push("Once addressed, run the relevant tests and summarize your changes.".to_string());

    for n in &mut data.notes {
        if n.status == "New" {
            n.status = "Sent".to_string();
        }
    }

    Some((pending.len(), lines.join("\n")))
}

pub fn run_cli(args: &[String]) -> i32 {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    let ppath = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "count" => {
            let count = resolve_notes_file(ppath)
                .map(|p| count_pending_in_file(&p))
                .unwrap_or(0);
            println!("{}", count);
            0
        }
        "list" => {
            let rendered = render_pending_notes(ppath);
            if !rendered.is_empty() {
                print!("{}", rendered);
            }
            0
        }
        "send" => {
            let session = args.get(2).map(|s| s.as_str()).unwrap_or("");
            let idx = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let pane_id = args.get(4).map(|s| s.as_str()).unwrap_or("");

            let notes_file = match resolve_notes_file(ppath) {
                Some(p) => p,
                None => {
                    tmux_display_msg(&format!("No review notes found for {}", ppath));
                    return 0;
                }
            };

            let mut data = match load_notes(&notes_file) {
                Some(d) => d,
                None => {
                    tmux_display_msg("No pending review notes to send");
                    return 0;
                }
            };

            let (count, prompt) = match format_and_mark_sent(&mut data) {
                Some(res) => res,
                None => {
                    tmux_display_msg("No pending review notes to send");
                    return 0;
                }
            };

            if let Ok(serialized) = serde_json::to_string_pretty(&data) {
                let _ = fs::write(&notes_file, serialized);
            }

            let mut target_pane = pane_id.to_string();
            if !super::actions::is_valid_pane_id(&target_pane)
                && !session.is_empty()
                && !idx.is_empty()
                && idx != "-"
            {
                let target_spec = format!("{}:{}", session, idx);
                target_pane =
                    tmux_out(&["display-message", "-p", "-t", &target_spec, "#{pane_id}"])
                        .unwrap_or_default();
            }

            if !super::actions::is_valid_pane_id(&target_pane) {
                tmux_display_msg("Target agent pane not found");
                return 1;
            }

            if !tmux_load_and_paste_bracketed(Some("cockpit_review_prompt"), &target_pane, &prompt)
            {
                let flat = prompt.replace(['\n', '\r'], " ");
                let _ = tmux_status(&["send-keys", "-l", "-t", &target_pane, "--", &flat]);
                let _ = tmux_status(&["send-keys", "-t", &target_pane, "Enter"]);
            }

            tmux_display_msg(&format!(
                "Dispatched {} review note(s) to agent in pane {}",
                count, target_pane
            ));
            0
        }
        _ => {
            eprintln!(
                "Usage: waymux review-notes {{count|list|send}} <path> [session] [idx] [pane_id]"
            );
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_and_mark_sent() {
        let raw = r#"{
            "notes": [
                {"file": "src/main.rs", "line": 42, "comment": "Fix unwrap", "status": "New"},
                {"file": "src/lib.rs", "line": 10, "comment": "Already sent", "status": "Sent"}
            ]
        }"#;
        let mut data: NotesFile = serde_json::from_str(raw).unwrap();
        let (count, prompt) = format_and_mark_sent(&mut data).unwrap();
        assert_eq!(count, 1);
        assert!(prompt.contains("1. **`src/main.rs`** (line 42):"));
        assert!(prompt.contains("> Fix unwrap"));
        assert_eq!(data.notes[0].status, "Sent");
        assert!(format_and_mark_sent(&mut data).is_none());
    }
}
