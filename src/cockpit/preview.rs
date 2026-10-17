use crate::cockpit::notes;
use crate::rpc_client;
use crate::theme::{BOLD, DIM, RESET, ThemePalette, strip_ansi};
use crate::tmux_cmd::{tmux_out, tmux_out_raw};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn box_line(card_border: &str, w: usize, content: &str) -> String {
    let clean = strip_ansi(content);
    let len = clean.chars().count();
    let max_content = w.saturating_sub(4);
    let pad = max_content.saturating_sub(len);
    format!(
        "{}│{} {}{} {}│{}\n",
        card_border,
        RESET,
        content,
        " ".repeat(pad),
        card_border,
        RESET
    )
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else if max_chars <= 1 {
        "…".to_string()
    } else {
        let prefix: String = s.chars().take(max_chars - 1).collect();
        format!("{}…", prefix)
    }
}

pub fn run_cli(args: &[String]) -> i32 {
    let session = args.first().map(|s| s.as_str()).unwrap_or("");
    let idx = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let mut pane_id = args.get(2).map(|s| s.as_str()).unwrap_or("").to_string();
    let mut ppath = args.get(3).map(|s| s.as_str()).unwrap_or("").to_string();
    let mut ai_raw = args.get(4).map(|s| s.as_str()).unwrap_or("").to_string();
    let mut agent_name = args.get(5).map(|s| s.as_str()).unwrap_or("").to_string();
    let mut age_str = args.get(6).map(|s| s.as_str()).unwrap_or("").to_string();
    let mut branch = args.get(7).map(|s| s.as_str()).unwrap_or("").to_string();
    let upstream_div = args.get(8).map(|s| s.as_str()).unwrap_or("");

    if session.is_empty() || idx.is_empty() || idx == "-" {
        println!("  \x1b[38;2;146;131;116m(no active window selected)\x1b[0m");
        return 0;
    }

    let target_win = format!("{}:{}", session, idx);
    if pane_id.is_empty() || pane_id == "-" {
        pane_id = tmux_out(&["display-message", "-p", "-t", &target_win, "#{pane_id}"])
            .unwrap_or_default();
    }
    if ppath.is_empty() || ppath == "-" {
        ppath = tmux_out(&[
            "display-message",
            "-p",
            "-t",
            &target_win,
            "#{pane_current_path}",
        ])
        .unwrap_or_default();
    }

    let theme = ThemePalette::load();
    let c_orange = &theme.c_perm;
    let c_yellow = &theme.c_busy;
    let c_red = &theme.c_error;
    let c_magenta = &theme.c_question;
    let c_cyan = &theme.c_cyan;
    let c_blue = &theme.c_branch;
    let c_fg = &theme.c_name;
    let c_gray = &theme.c_muted;

    let mut is_git = false;
    if !branch.is_empty() && branch != "-" && branch != "non-git" {
        is_git = true;
    } else if branch.is_empty() && !ppath.is_empty() && Path::new(&ppath).is_dir() {
        if Path::new(&ppath).join(".git").exists() {
            is_git = true;
        } else if let Ok(o) = Command::new("git")
            .args(["-C", &ppath, "rev-parse", "--is-inside-work-tree"])
            .stderr(Stdio::null())
            .output()
            && o.status.success()
        {
            is_git = true;
        }
        if is_git {
            branch = Command::new("git")
                .args(["-C", &ppath, "rev-parse", "--abbrev-ref", "HEAD"])
                .stderr(Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "detached".to_string());
        }
    }

    let mut commit = "n/a".to_string();
    let mut churn_info = "working tree clean".to_string();
    let mut churn_files = String::new();

    if is_git && !ppath.is_empty() {
        if let Ok(o) = Command::new("git")
            .args(["-C", &ppath, "log", "-1", "--format=%h · %s (%cr)"])
            .stderr(Stdio::null())
            .output()
            && o.status.success()
        {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() {
                commit = s;
            }
        }
        if let Ok(o) = Command::new("git")
            .args(["-C", &ppath, "status", "--short"])
            .stderr(Stdio::null())
            .output()
            && o.status.success()
        {
            let stat_str = String::from_utf8_lossy(&o.stdout);
            let non_empty: Vec<&str> = stat_str.lines().filter(|l| !l.trim().is_empty()).collect();
            if !non_empty.is_empty() {
                churn_info = format!("{} files modified", non_empty.len());
                churn_files = non_empty
                    .iter()
                    .take(4)
                    .map(|l| format!("    {}", l))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
    }

    if ai_raw.is_empty() || ai_raw == "-" {
        let states = rpc_client::fetch_agent_states();
        if let Some(entry) = states.get(&pane_id) {
            ai_raw = entry.state.clone();
        }
        if ai_raw.is_empty() {
            ai_raw = tmux_out(&[
                "show-option",
                "-wqv",
                "-t",
                &target_win,
                "@ai_agent_state_raw",
            ])
            .unwrap_or_default();
        }
    }

    if ai_raw == "-" {
        ai_raw.clear();
    }
    if agent_name == "-" {
        agent_name.clear();
    }
    if age_str == "-" {
        age_str.clear();
    }

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as usize)
        .unwrap_or(0);
    let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spin = frames[(now_ms / 100) % 10];

    let (mut status_badge, card_border) = match ai_raw.as_str() {
        "permission" => (
            format!("{}{}󱅭 PERMISSION REQUIRED{}", c_orange, BOLD, RESET),
            c_orange,
        ),
        "question" | "awaiting_input" => (
            format!("{}{}󱜻 INTERACTIVE QUESTION{}", c_magenta, BOLD, RESET),
            c_magenta,
        ),
        "stalled" | "hanging" => (
            format!("{}{}󱥁 STALLED / HANGING{}", c_orange, BOLD, RESET),
            c_orange,
        ),
        "error" => (
            format!("{}{}󰨄 EXECUTION ERROR{}", c_red, BOLD, RESET),
            c_red,
        ),
        "busy" | "working" => {
            let ag = if agent_name.is_empty() {
                "agent"
            } else {
                &agent_name
            };
            (
                format!("{}{}{} WORKING ({}){}", c_yellow, BOLD, spin, ag, RESET),
                c_yellow,
            )
        }
        "idle" => {
            let ag = if agent_name.is_empty() {
                "agent"
            } else {
                &agent_name
            };
            (
                format!("{}{}󱥂 IDLE / READY ({}){}", c_cyan, BOLD, ag, RESET),
                c_cyan,
            )
        }
        _ => {
            if !agent_name.is_empty() {
                (
                    format!("{}{}󱥂 READY ({}){}", c_cyan, BOLD, agent_name, RESET),
                    c_cyan,
                )
            } else {
                (format!("{}TERMINAL{}", c_gray, RESET), c_gray)
            }
        }
    };

    if !age_str.is_empty() {
        status_badge.push_str(&format!("  {}• {} ago{}", c_gray, age_str, RESET));
    }

    let preview_w: usize = std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80)
        .max(50);
    let w = (preview_w - 2).clamp(48, 76);

    let raw_terminal = tmux_out_raw(&["capture-pane", "-ep", "-S", "-120", "-t", &pane_id])
        .unwrap_or_default()
        .replace("\x1b[4m", "")
        .replace("\x1b[24m", "")
        .replace("\x1b[21m", "");

    let mut out = String::with_capacity(4096);
    let border_dashes = "─".repeat(w - 2);
    out.push_str(&format!("{}╭{}╮{}\n", card_border, border_dashes, RESET));
    out.push_str(&box_line(
        card_border,
        w,
        &format!("{}󰈈 Status:{} {}", BOLD, RESET, status_badge),
    ));

    if !agent_name.is_empty() {
        out.push_str(&box_line(
            card_border,
            w,
            &format!(
                "{}󰑮 Agent:{}  {}{}{}",
                BOLD, RESET, c_yellow, agent_name, RESET
            ),
        ));
    }

    // Extract latest user task prompt from scrollback
    let mut captured_task = String::new();
    for line in raw_terminal.lines() {
        let stripped = strip_ansi(line);
        let trimmed = stripped.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("> ")
            .or_else(|| trimmed.strip_prefix("❯ "))
            .or_else(|| trimmed.strip_prefix("User:"))
            .or_else(|| trimmed.strip_prefix("Input:"))
        {
            let clean = rest.split_whitespace().collect::<Vec<_>>().join(" ");
            if !clean.is_empty() {
                captured_task = clean;
            }
        }
    }

    if !captured_task.is_empty() {
        let max_tk = w.saturating_sub(14).max(5);
        let clean_task = truncate_str(&captured_task, max_tk);
        out.push_str(&box_line(
            card_border,
            w,
            &format!("{}󰞋 Task:{}   {}{}{}", BOLD, RESET, c_fg, clean_task, RESET),
        ));
    }

    if is_git {
        let sync_str = if !upstream_div.is_empty() && upstream_div != "-" {
            if upstream_div.contains("synced") {
                format!("  {}󰄬 Synced{}", c_cyan, RESET)
            } else if upstream_div.contains("ahead") && upstream_div.contains("behind") {
                format!("  {}󰞕 Ahead{} {}󰞒 Behind{}", c_cyan, RESET, c_red, RESET)
            } else if upstream_div.contains("ahead") {
                format!("  {}󰞕 Ahead{}", c_cyan, RESET)
            } else if upstream_div.contains("behind") {
                format!("  {}󰞒 Behind{}", c_red, RESET)
            } else {
                format!("  {}{}{}", c_cyan, upstream_div, RESET)
            }
        } else {
            String::new()
        };

        let max_cnt = w.saturating_sub(4);
        let clean_sync_len = strip_ansi(&sync_str).chars().count();
        let churn_part = format!("    {}󰄧 Churn:{} {}", BOLD, RESET, churn_info);
        let clean_churn_len = strip_ansi(&churn_part).chars().count();
        let avail_b = max_cnt
            .saturating_sub(10 + clean_sync_len + clean_churn_len)
            .max(6);
        let clean_branch = truncate_str(&branch, avail_b);
        out.push_str(&box_line(
            card_border,
            w,
            &format!(
                "{}󰊢 Branch:{} {}{}{}{}{}",
                BOLD, RESET, c_blue, clean_branch, RESET, sync_str, churn_part
            ),
        ));

        if !matches!(branch.as_str(), "main" | "master" | "detached" | "HEAD") {
            let hash_key = format!("{}_{}", ppath.replace('/', "_"), branch.replace('/', "_"));
            let cache_file = format!("/tmp/gh-pr-cache-{}.json", hash_key);
            if let Ok(content) = fs::read_to_string(&cache_file)
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
            {
                let pr_num = match val.get("number") {
                    Some(serde_json::Value::Number(n)) => n.to_string(),
                    Some(serde_json::Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let pr_title = val
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                if !pr_num.is_empty() {
                    let max_pr = w.saturating_sub(24).max(5);
                    let clean_pr_title = truncate_str(&pr_title, max_pr);
                    out.push_str(&box_line(
                        card_border,
                        w,
                        &format!(
                            "{}󰏫 PR #{}:{}  {}{}{}",
                            BOLD, pr_num, RESET, c_fg, clean_pr_title, RESET
                        ),
                    ));
                }
            }
        }

        let max_c = w.saturating_sub(14);
        let clean_commit = truncate_str(&commit, max_c);
        out.push_str(&box_line(
            card_border,
            w,
            &format!(
                "{}󰜘 Commit:{} {}{}{}",
                BOLD, RESET, DIM, clean_commit, RESET
            ),
        ));
    }

    out.push_str(&format!("{}╰{}╯{}\n", card_border, border_dashes, RESET));

    let notes_section = notes::render_pending_notes(&ppath);
    if !notes_section.is_empty() {
        out.push_str(&notes_section);
    }

    if !churn_files.is_empty() {
        out.push_str(&format!("{}{}{}\n\n", DIM, churn_files, RESET));
    }

    let lines: Vec<&str> = raw_terminal.lines().collect();
    if let Some(last_non_empty) = lines.iter().rposition(|l| !l.trim().is_empty()) {
        for line in &lines[..=last_non_empty] {
            out.push_str(line);
            out.push('\n');
        }
    } else {
        out.push_str(&format!(
            "  {}(pane output empty or detached){}\n",
            c_gray, RESET
        ));
    }

    print!("{}", out);
    0
}
