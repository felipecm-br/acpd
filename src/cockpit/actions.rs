use crate::tmux_cmd::{tmux_display_msg, tmux_out, tmux_status};
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn is_valid_pane_id(pid: &str) -> bool {
    pid.starts_with('%') && pid.len() > 1 && pid[1..].chars().all(|c| c.is_ascii_digit())
}

pub fn can_confirm_state(state: &str) -> bool {
    matches!(state, "permission" | "question" | "awaiting_input")
}

pub fn run_agent_action(args: &[String]) -> i32 {
    let action = args.first().map(|s| s.as_str()).unwrap_or("");
    let state = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let pid = args.get(2).map(|s| s.as_str()).unwrap_or("");

    if !is_valid_pane_id(pid) {
        return 0;
    }

    match action {
        "approve" => {
            if can_confirm_state(state) {
                let _ = tmux_status(&["send-keys", "-t", pid, "y", "Enter"]);
            } else {
                let st = if state.is_empty() { "normal" } else { state };
                tmux_display_msg(&format!(
                    "Action ignored: Pane {} is not awaiting confirmation (state: {})",
                    pid, st
                ));
            }
            0
        }
        "deny" => {
            if can_confirm_state(state) {
                let _ = tmux_status(&["send-keys", "-t", pid, "n", "Enter"]);
            } else {
                let st = if state.is_empty() { "normal" } else { state };
                tmux_display_msg(&format!(
                    "Action ignored: Pane {} is not awaiting confirmation (state: {})",
                    pid, st
                ));
            }
            0
        }
        "prompt" => {
            if args.len() > 3 {
                let prompt_text = args[3..].join(" ");
                if !prompt_text.is_empty() {
                    let _ = tmux_status(&["send-keys", "-l", "-t", pid, "--", &prompt_text]);
                    let _ = tmux_status(&["send-keys", "-t", pid, "Enter"]);
                }
            }
            0
        }
        _ => {
            eprintln!("Usage: waymux agent-action {{approve|deny|prompt}} <state> <pid> [args...]");
            1
        }
    }
}

pub fn create_window_from_raw(raw_input: &str) -> Option<String> {
    let mut session = String::new();
    let mut row_path = String::new();

    if !raw_input.trim().is_empty()
        && let Some(first_line) = raw_input.lines().next()
    {
        if let Some(stripped) = first_line.strip_prefix('#') {
            let header_sess = stripped.trim();
            if !header_sess.is_empty() && tmux_status(&["has-session", "-t", header_sess]) {
                session = header_sess.to_string();
            }
        } else {
            let cols: Vec<&str> = first_line.split('\t').collect();
            if let Some(s) = cols.get(3) {
                session = (*s).to_string();
            }
            if let Some(p) = cols.get(5)
                && !p.is_empty()
                && Path::new(p).is_dir()
            {
                row_path = (*p).to_string();
            }
        }
    }

    if session.is_empty() || !tmux_status(&["has-session", "-t", &session]) {
        session = std::env::var("TMUX_ORIGIN_SESSION")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| tmux_out(&["display-message", "-p", "#S"]))
            .unwrap_or_default();
    }

    if session.is_empty() {
        return None;
    }

    if row_path.is_empty() {
        let target_sess = format!("{}:", session);
        if let Some(sdir) = tmux_out(&[
            "display-message",
            "-t",
            &target_sess,
            "-p",
            "#{pane_current_path}",
        ]) && Path::new(&sdir).is_dir()
        {
            row_path = sdir;
        }
    }

    let target_sess = format!("{}:", session);
    let new_target = if !row_path.is_empty() && Path::new(&row_path).is_dir() {
        tmux_out(&[
            "new-window",
            "-P",
            "-F",
            "#{session_name}:#{window_index}",
            "-c",
            &row_path,
            "-t",
            &target_sess,
        ])
        .or_else(|| {
            tmux_out(&[
                "new-window",
                "-P",
                "-F",
                "#{session_name}:#{window_index}",
                "-t",
                &target_sess,
            ])
        })
    } else {
        tmux_out(&[
            "new-window",
            "-P",
            "-F",
            "#{session_name}:#{window_index}",
            "-t",
            &target_sess,
        ])
    };

    if let Some(ref nt) = new_target {
        let _ = tmux_status(&["switch-client", "-t", nt]);
    }
    new_target
}

pub fn run_window_create() -> i32 {
    let mut raw = String::new();
    let _ = io::stdin().read_to_string(&mut raw);
    if let Some(target) = create_window_from_raw(&raw) {
        println!("{}", target);
    }
    0
}

pub fn run_window_delete() -> i32 {
    let mut raw = String::new();
    let _ = io::stdin().read_to_string(&mut raw);
    if raw.trim().is_empty() {
        return 0;
    }

    let first_line = raw.lines().next().unwrap_or("");
    if let Some(stripped) = first_line.strip_prefix('#') {
        let sess_name = stripped.trim();
        if sess_name.is_empty() {
            return 0;
        }

        let target_sess = format!("{}:", sess_name);
        let wt_path = tmux_out(&[
            "display-message",
            "-t",
            &target_sess,
            "-p",
            "#{pane_current_path}",
        ])
        .unwrap_or_default();

        if !wt_path.is_empty() && Path::new(&wt_path).is_dir() {
            let git_common = Command::new("git")
                .args(["-C", &wt_path, "rev-parse", "--git-common-dir"])
                .stderr(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            let git_dir = Command::new("git")
                .args(["-C", &wt_path, "rev-parse", "--git-dir"])
                .stderr(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();

            if !git_common.is_empty() && !git_dir.is_empty() && git_common != git_dir {
                let branch = Command::new("git")
                    .args(["-C", &wt_path, "rev-parse", "--abbrev-ref", "HEAD"])
                    .stderr(Stdio::null())
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();

                if !branch.is_empty() && branch != "main" && branch != "master" {
                    let home = std::env::var("HOME").unwrap_or_default();
                    let awt_del = format!("{}/.config/waymaker/scripts/awt-delete.sh", home);
                    if Path::new(&awt_del).is_file() {
                        if std::env::var("TMUX").is_ok() {
                            let cmd_str = format!(
                                "'{}' '{}' '{}' '{}' --confirm",
                                awt_del, sess_name, wt_path, branch
                            );
                            let _ = tmux_status(&[
                                "display-popup",
                                "-b",
                                "rounded",
                                "-w",
                                "60",
                                "-h",
                                "12",
                                "-E",
                                &cmd_str,
                            ]);
                        } else {
                            let _ = Command::new(&awt_del)
                                .args([sess_name, &wt_path, &branch, "--confirm"])
                                .status();
                        }
                        return 0;
                    }
                }
            }
        }

        let cur_sess = tmux_out(&["display-message", "-p", "#{session_name}"]).unwrap_or_default();
        if cur_sess == sess_name && !tmux_status(&["switch-client", "-l"]) {
            let _ = tmux_status(&["switch-client", "-n"]);
        }
        let _ = tmux_status(&["kill-session", "-t", sess_name]);
        return 0;
    }

    let mut targets = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if let (Some(sess), Some(idx)) = (cols.get(3), cols.get(1))
            && !sess.is_empty()
            && !idx.is_empty()
            && *idx != "-"
        {
            targets.push(((*sess).to_string(), (*idx).to_string()));
        }
    }

    if targets.is_empty() {
        return 0;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let record_script = format!("{}/.config/tmux/record-window-state.sh", home);

    for (sess, idx) in targets {
        let target = format!("{}:{}", sess, idx);
        let is_active = tmux_out(&["display-message", "-p", "-t", &target, "#{window_active}"])
            .unwrap_or_default();
        let win_count = tmux_out(&["list-windows", "-t", &sess])
            .map(|s| s.lines().count())
            .unwrap_or(0);

        if is_active == "1"
            && win_count > 1
            && !tmux_status(&["select-window", "-t", &sess, "-l"])
            && !tmux_status(&["previous-window", "-t", &sess])
        {
            let _ = tmux_status(&["next-window", "-t", &sess]);
        }

        if Path::new(&record_script).is_file() {
            let _ = Command::new(&record_script)
                .args(["--push", &target])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = tmux_status(&["kill-window", "-t", &target]);
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_pane_id() {
        assert!(is_valid_pane_id("%0"));
        assert!(is_valid_pane_id("%123"));
        assert!(!is_valid_pane_id(""));
        assert!(!is_valid_pane_id("-"));
        assert!(!is_valid_pane_id("%"));
        assert!(!is_valid_pane_id("%12a"));
    }

    #[test]
    fn test_can_confirm_state() {
        assert!(can_confirm_state("permission"));
        assert!(can_confirm_state("question"));
        assert!(can_confirm_state("awaiting_input"));
        assert!(!can_confirm_state("working"));
        assert!(!can_confirm_state("idle"));
        assert!(!can_confirm_state("normal"));
    }
}
