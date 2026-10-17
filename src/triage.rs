use crate::cockpit::items::{contains_agent_name, is_agent_binary_name};
use crate::rpc_client;
use crate::theme::ThemePalette;
use crate::tmux_cmd::{
    tmux_display_msg_timed, tmux_get_global_opt, tmux_out, tmux_set_global_opt, tmux_status,
    tmux_unset_global_opt,
};
use std::path::Path;
use std::process::{Command, Stdio};

fn pane_exists(pane_id: &str) -> bool {
    !pane_id.is_empty()
        && pane_id.starts_with('%')
        && tmux_out(&["display-message", "-t", pane_id, "-p", "#{pane_id}"]).is_some()
}

pub fn collect_notifying_panes() -> Vec<String> {
    let mut notifying = Vec::new();
    let acpd_map = rpc_client::fetch_agent_states();

    if !acpd_map.is_empty() {
        let mut urgent: Vec<(&String, u64)> = acpd_map
            .iter()
            .filter(|(_, v)| {
                matches!(
                    v.state.as_str(),
                    "permission" | "awaiting_input" | "question" | "error" | "stalled"
                )
            })
            .map(|(k, v)| (k, v.last_timestamp))
            .collect();
        urgent.sort_by_key(|b| std::cmp::Reverse(b.1));

        for (p, _) in urgent {
            if pane_exists(p) {
                notifying.push(p.clone());
            }
        }

        if notifying.is_empty() {
            let mut active: Vec<(&String, u64)> = acpd_map
                .iter()
                .filter(|(_, v)| matches!(v.state.as_str(), "working" | "busy"))
                .map(|(k, v)| (k, v.last_timestamp))
                .collect();
            active.sort_by_key(|b| std::cmp::Reverse(b.1));

            for (p, _) in active {
                if pane_exists(p) {
                    notifying.push(p.clone());
                }
            }
        }
    }

    let list_raw = tmux_out(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}\t#{@ai_agent_state_raw}",
    ])
    .unwrap_or_default();

    if notifying.is_empty() {
        for line in list_raw.lines() {
            if let Some((p, st)) = line.split_once('\t')
                && matches!(
                    st.trim(),
                    "question" | "permission" | "error" | "awaiting_input" | "stalled"
                )
            {
                notifying.push(p.to_string());
            }
        }
    }

    if notifying.is_empty() {
        let last_bell = tmux_get_global_opt("@ai_agent_last_bell");
        if pane_exists(&last_bell) {
            notifying.push(last_bell);
        }
    }

    if notifying.is_empty() {
        for line in list_raw.lines() {
            if let Some((p, st)) = line.split_once('\t')
                && matches!(st.trim(), "working" | "busy")
            {
                notifying.push(p.to_string());
            }
        }
    }

    if notifying.is_empty() {
        for line in list_raw.lines() {
            if let Some((p, st)) = line.split_once('\t')
                && matches!(
                    st.trim(),
                    "idle"
                        | "working"
                        | "busy"
                        | "question"
                        | "permission"
                        | "error"
                        | "awaiting_input"
                        | "stalled"
                )
            {
                notifying.push(p.to_string());
            }
        }
    }

    if notifying.is_empty() {
        let cmd_raw = tmux_out(&[
            "list-panes",
            "-a",
            "-F",
            "#{pane_id}\t#{pane_current_command}\t#{pane_title}",
        ])
        .unwrap_or_default();
        for line in cmd_raw.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let p = parts[0];
                let cmd = parts[1];
                let title = parts[2];
                if is_agent_binary_name(cmd) || contains_agent_name(title) {
                    notifying.push(p.to_string());
                }
            }
        }
    }

    notifying
}

pub fn run_popup() -> i32 {
    let notifying = collect_notifying_panes();
    if notifying.is_empty() {
        tmux_display_msg_timed(1500, " 󰮯 No active AI Agent notification.");
        return 0;
    }

    let total = notifying.len();
    let mut curr_idx = tmux_get_global_opt("@ai_agent_bell_ring_idx")
        .parse::<usize>()
        .unwrap_or(0);
    if curr_idx >= total {
        curr_idx = 0;
    }

    let mut pane = notifying[curr_idx].clone();
    let mut next_idx = (curr_idx + 1) % total;
    tmux_set_global_opt("@ai_agent_bell_ring_idx", &next_idx.to_string());

    let current_pane = tmux_out(&["display-message", "-p", "#{pane_id}"]).unwrap_or_default();
    let current_sess = tmux_out(&["display-message", "-p", "#S"]).unwrap_or_default();
    let current_win = tmux_out(&["display-message", "-p", "#I"]).unwrap_or_default();

    let mut sess = tmux_out(&["display-message", "-t", &pane, "-p", "#S"]).unwrap_or_default();
    let mut win_idx = tmux_out(&["display-message", "-t", &pane, "-p", "#I"]).unwrap_or_default();
    let mut win_name = tmux_out(&["display-message", "-t", &pane, "-p", "#W"]).unwrap_or_default();

    if current_pane == pane {
        if total > 1 {
            curr_idx = next_idx;
            pane = notifying[curr_idx].clone();
            next_idx = (curr_idx + 1) % total;
            tmux_set_global_opt("@ai_agent_bell_ring_idx", &next_idx.to_string());
            sess = tmux_out(&["display-message", "-t", &pane, "-p", "#S"]).unwrap_or_default();
            win_idx = tmux_out(&["display-message", "-t", &pane, "-p", "#I"]).unwrap_or_default();
            win_name = tmux_out(&["display-message", "-t", &pane, "-p", "#W"]).unwrap_or_default();
        } else {
            tmux_display_msg_timed(
                1500,
                &format!(
                    " 󰮯 Already focused on active agent ({} › {})",
                    sess, win_name
                ),
            );
            return 0;
        }
    }

    if current_sess == sess && current_win == win_idx {
        let _ = tmux_status(&["select-pane", "-t", &pane]);
        tmux_display_msg_timed(
            1500,
            &format!(" 󰮯 AI Agent: focused split pane ({} › {})", sess, win_name),
        );
        return 0;
    }

    let popup_sess = "_popups";
    if !tmux_status(&["has-session", "-t", popup_sess]) {
        let _ = tmux_status(&["new-session", "-d", "-s", popup_sess, "-n", "_dummy"]);
    }

    if let Some(wins) = tmux_out(&[
        "list-windows",
        "-t",
        popup_sess,
        "-F",
        "#{window_id}:#{window_index}",
    ]) {
        for w in wins.lines() {
            if let Some((w_id, w_i)) = w.split_once(':')
                && w_i != "0"
                && !tmux_status(&["unlink-window", "-t", w_id])
            {
                let _ = tmux_status(&["kill-window", "-t", w_id]);
            }
        }
    }

    let src_win = format!("{}:{}", sess, win_idx);
    let dst_win1 = format!("{}:1", popup_sess);
    let dst_win0 = format!("{}:0", popup_sess);
    if !tmux_status(&["link-window", "-s", &src_win, "-t", &dst_win1]) {
        let _ = tmux_status(&["link-window", "-a", "-s", &src_win, "-t", &dst_win0]);
    }
    let _ = tmux_status(&["select-window", "-t", &dst_win1]);
    let _ = tmux_status(&["select-pane", "-t", &pane]);
    let _ = tmux_status(&["set-option", "-t", popup_sess, "status", "off"]);
    let _ = tmux_status(&["set-option", "-t", popup_sess, "key-table", "popup"]);

    let theme = ThemePalette::load();
    let text_col = std::env::var("TMUX_POPUP_TEXT_COLOR").unwrap_or_else(|_| "default".into());
    let attach_cmd = format!(
        "tmux attach-session -t \"{}:1\"; tmux unlink-window -t \"{}:1\" >/dev/null 2>&1 || true",
        popup_sess, popup_sess
    );

    let _ = tmux_status(&[
        "display-popup",
        "-S",
        &format!("fg={}", theme.yellow),
        "-s",
        &format!("fg={}", text_col),
        "-T",
        " 󰮯 ",
        "-w",
        "80%",
        "-h",
        "75%",
        "-b",
        "rounded",
        "-E",
        &attach_cmd,
    ]);

    0
}

pub fn run_jump() -> i32 {
    let notifying = collect_notifying_panes();
    let current_pane = tmux_out(&["display-message", "-p", "#{pane_id}"]).unwrap_or_default();
    let current_sess = tmux_out(&["display-message", "-p", "#S"]).unwrap_or_default();
    let current_win = tmux_out(&["display-message", "-p", "#I"]).unwrap_or_default();

    let origin_pane = tmux_get_global_opt("@ai_agent_triage_origin_pane");
    let origin_sess = tmux_get_global_opt("@ai_agent_triage_origin_sess");
    let origin_win = tmux_get_global_opt("@ai_agent_triage_origin_win");

    if notifying.is_empty() {
        if pane_exists(&origin_pane) {
            tmux_unset_global_opt("@ai_agent_triage_origin_pane");
            tmux_unset_global_opt("@ai_agent_triage_origin_sess");
            tmux_unset_global_opt("@ai_agent_triage_origin_win");

            if current_sess != origin_sess {
                let _ = tmux_status(&["switch-client", "-t", &origin_sess]);
            }
            let target_win = format!("{}:{}", origin_sess, origin_win);
            let _ = tmux_status(&["select-window", "-t", &target_win]);
            let _ = tmux_status(&["select-pane", "-t", &origin_pane]);

            tmux_display_msg_timed(
                1500,
                &format!(" 󰌑 AI Triage: Returned to {}:{}", origin_sess, origin_win),
            );
            return 0;
        }
        tmux_display_msg_timed(1500, " 󰮯 No active AI Agent alerts or return origin.");
        return 0;
    }

    let current_is_agent = tmux_out(&[
        "show-option",
        "-p",
        "-t",
        &current_pane,
        "-v",
        "@ai_agent_state_raw",
    ])
    .unwrap_or_default();

    if origin_pane.is_empty() && current_is_agent.is_empty() {
        tmux_set_global_opt("@ai_agent_triage_origin_pane", &current_pane);
        tmux_set_global_opt("@ai_agent_triage_origin_sess", &current_sess);
        tmux_set_global_opt("@ai_agent_triage_origin_win", &current_win);
    }

    let total = notifying.len();
    let mut curr_idx = tmux_get_global_opt("@ai_agent_triage_ring_idx")
        .parse::<usize>()
        .unwrap_or(0);
    if curr_idx >= total {
        curr_idx = 0;
    }

    let target_pane = &notifying[curr_idx];
    let next_idx = (curr_idx + 1) % total;
    tmux_set_global_opt("@ai_agent_triage_ring_idx", &next_idx.to_string());

    let target_sess =
        tmux_out(&["display-message", "-t", target_pane, "-p", "#S"]).unwrap_or_default();
    let target_win =
        tmux_out(&["display-message", "-t", target_pane, "-p", "#I"]).unwrap_or_default();
    let target_win_name =
        tmux_out(&["display-message", "-t", target_pane, "-p", "#W"]).unwrap_or_default();
    let target_state = tmux_out(&[
        "show-option",
        "-p",
        "-t",
        target_pane,
        "-v",
        "@ai_agent_state_raw",
    ])
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "active".to_string());

    if current_sess != target_sess {
        let _ = tmux_status(&["switch-client", "-t", &target_sess]);
    }
    let win_spec = format!("{}:{}", target_sess, target_win);
    let _ = tmux_status(&["select-window", "-t", &win_spec]);
    let _ = tmux_status(&["select-pane", "-t", target_pane]);

    if let Ok(home) = std::env::var("HOME") {
        let sound_rel = match target_state.as_str() {
            "question" | "awaiting_input" => "02-gentle-ping.wav",
            "permission" => "06-cyber-pulse.wav",
            "error" | "stalled" => "10-arcade-blip.wav",
            _ => "04-subtle-bell.wav",
        };
        let sound_file = format!("{}/.local/share/sounds/ai/{}", home, sound_rel);
        if Path::new(&sound_file).is_file()
            && Command::new("pw-play")
                .arg(&sound_file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_err()
        {
            let _ = Command::new("paplay")
                .arg(&sound_file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }

    tmux_display_msg_timed(
        2000,
        &format!(
            " 󰮯 AI Triage: {} › {} ({}/{}) [{}] │ Prefix+I to return",
            target_sess,
            target_win_name,
            curr_idx + 1,
            total,
            target_state
        ),
    );

    0
}

pub fn run_cli(args: &[String]) -> i32 {
    if args
        .iter()
        .any(|a| a == "--jump" || a == "jump" || a == "-j")
    {
        run_jump()
    } else {
        run_popup()
    }
}
