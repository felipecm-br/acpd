use std::io::Write;
use std::process::{Command, Stdio};

pub fn tmux_out(args: &[&str]) -> Option<String> {
    let output = Command::new("tmux")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(
            String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        )
    } else {
        None
    }
}

pub fn tmux_out_raw(args: &[&str]) -> Option<String> {
    let output = Command::new("tmux")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

pub fn tmux_status(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn tmux_get_global_opt(opt: &str) -> String {
    tmux_out(&["show-option", "-gqv", opt]).unwrap_or_default()
}

pub fn tmux_set_global_opt(opt: &str, val: &str) {
    let _ = tmux_status(&["set-option", "-g", opt, val]);
}

pub fn tmux_unset_global_opt(opt: &str) {
    let _ = tmux_status(&["set-option", "-gu", opt]);
}

pub fn tmux_display_msg(msg: &str) {
    let _ = tmux_status(&["display-message", msg]);
}

pub fn tmux_display_msg_timed(ms: u32, msg: &str) {
    let ms_str = ms.to_string();
    let _ = tmux_status(&["display-message", "-d", &ms_str, msg]);
}

pub fn tmux_load_and_paste_bracketed(
    buf_name: Option<&str>,
    target_pane: &str,
    text: &str,
) -> bool {
    let mut load_args = vec!["load-buffer"];
    if let Some(b) = buf_name {
        load_args.push("-b");
        load_args.push(b);
    }
    load_args.push("-");

    let mut child = match Command::new("tmux")
        .args(&load_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        return false;
    }

    if !child.wait().map(|s| s.success()).unwrap_or(false) {
        return false;
    }

    let mut paste_args = vec!["paste-buffer"];
    if let Some(b) = buf_name {
        paste_args.push("-b");
        paste_args.push(b);
    }
    paste_args.extend(["-t", target_pane, "-p"]);

    if tmux_status(&paste_args) {
        let _ = tmux_status(&["send-keys", "-t", target_pane, "Enter"]);
        true
    } else {
        false
    }
}
