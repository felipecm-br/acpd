use crate::tmux_cmd::{tmux_load_and_paste_bracketed, tmux_out, tmux_status};
use std::fs::{self, OpenOptions};
use std::io::Write;

fn log_injector(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/tmux-injector.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}

pub fn sanitize_workspace_key(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub fn run_cli(args: &[String]) -> i32 {
    let workspace_dir = args.first().map(|s| s.as_str()).unwrap_or("");
    let prompt = args.get(1).map(|s| s.as_str()).unwrap_or("");

    log_injector(&format!("{} {}", workspace_dir, prompt));

    if workspace_dir.is_empty() || prompt.is_empty() {
        return 1;
    }

    let safe_path = sanitize_workspace_key(workspace_dir);
    let pane_file = format!("/tmp/agy-active-pane-{}.txt", safe_path);

    let pane = match fs::read_to_string(&pane_file) {
        Ok(s) => s.trim().to_string(),
        Err(_) => return 1,
    };

    if pane.is_empty() {
        return 1;
    }

    if let Some(list_out) = tmux_out(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_id} #{pane_current_command}",
    ]) {
        for line in list_out.lines() {
            if let Some((pid, cmd)) = line.split_once(' ')
                && pid == pane
            {
                let cmd_trimmed = cmd.trim();
                if !cmd_trimmed.is_empty()
                    && !matches!(
                        cmd_trimmed,
                        "agy" | "agy-bin" | "antigravity" | "node" | "bash" | "zsh"
                    )
                {
                    log_injector(&format!(
                        "stale pane ({}) — refusing to inject",
                        cmd_trimmed
                    ));
                    return 1;
                }
            }
        }
    }

    if tmux_load_and_paste_bracketed(None, &pane, prompt) {
        return 0;
    }

    let single_line = prompt.replace(['\n', '\r'], " ");
    let _ = tmux_status(&["send-keys", "-t", &pane, "-l", &single_line]);
    let _ = tmux_status(&["send-keys", "-t", &pane, "Enter"]);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_workspace_key() {
        assert_eq!(
            sanitize_workspace_key("/home/fecavmi/.dotfiles/main"),
            "_home_fecavmi__dotfiles_main"
        );
    }
}
