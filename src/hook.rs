use crate::inject::sanitize_workspace_key;
use crate::rpc_client::{http_get_json, http_post_json, post_agent_status};
use crate::tmux_cmd::{tmux_out, tmux_status};
use std::fs;
use std::io::{self, Read};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn is_stdin_tty() -> bool {
    unsafe { nix::libc::isatty(io::stdin().as_raw_fd()) == 1 }
}

fn read_stdin_json() -> serde_json::Value {
    if is_stdin_tty() {
        return serde_json::json!({});
    }
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    if input.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&input).unwrap_or_else(|_| serde_json::json!({}))
    }
}

pub fn get_active_tmux_pane() -> String {
    if let Ok(p) = std::env::var("TMUX_PANE")
        && !p.trim().is_empty()
    {
        return p.trim().to_string();
    }
    if let Some(p) = tmux_out(&["display-message", "-p", "#{pane_id}"])
        && !p.trim().is_empty()
    {
        return p;
    }
    if let Some(panes) = tmux_out(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_id} #{pane_current_command}",
    ]) {
        for line in panes.lines() {
            if (line.contains("agy") || line.contains("node"))
                && let Some(pid) = line.split_whitespace().next()
            {
                return pid.to_string();
            }
        }
    }
    String::new()
}

fn set_ai_agent_title(pane: &str, title: Option<&str>) {
    if pane.is_empty() {
        return;
    }
    if let Some(t) = title {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            let clean: String = trimmed
                .chars()
                .filter(|c| !matches!(c, '"' | '\\' | '$' | '`'))
                .take(80)
                .collect();
            let _ = tmux_status(&["set-option", "-w", "-t", pane, "@ai_agent_title", &clean]);
            let _ = tmux_status(&["refresh-client", "-S"]);
            return;
        }
    }
    let _ = tmux_status(&["set-option", "-w", "-t", pane, "-u", "@ai_agent_title"]);
    let _ = tmux_status(&["refresh-client", "-S"]);
}

fn set_lazygitrs_icon(pane: &str, icon: Option<&str>) {
    if pane.is_empty() {
        return;
    }
    if let Some(ic) = icon {
        let _ = tmux_status(&["set-option", "-w", "-t", pane, "@lazygitrs_icon", ic]);
    } else {
        let _ = tmux_status(&["set-option", "-w", "-t", pane, "-u", "@lazygitrs_icon"]);
    }
    let _ = tmux_status(&["refresh-client", "-S"]);
}

pub fn extract_session_title(ctx: &serde_json::Value) -> Option<String> {
    if let Some(t) = ctx.get("title").and_then(|v| v.as_str())
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    if let Some(t) = ctx.get("sessionTitle").and_then(|v| v.as_str())
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    if let Some(tp) = ctx.get("transcriptPath").and_then(|v| v.as_str())
        && Path::new(tp).is_file()
        && let Ok(content) = fs::read_to_string(tp)
        && let Some(first_line) = content.lines().next()
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(first_line)
        && let Some(raw_content) = parsed.get("content").and_then(|v| v.as_str())
    {
        let mut raw = if let Some(start) = raw_content.find("<USER_REQUEST>")
            && let Some(end) = raw_content.find("</USER_REQUEST>")
            && start + 14 <= end
        {
            &raw_content[start + 14..end]
        } else {
            raw_content
        };
        raw = raw.trim();
        if let Some(rest) = raw.strip_prefix('/')
            && let Some(idx) = rest.find(char::is_whitespace)
        {
            raw = rest[idx..].trim();
        }
        let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            return Some(collapsed);
        }
    }
    None
}

pub fn classify_pre_tool_state(ctx: &serde_json::Value) -> &'static str {
    let tool_name = ctx
        .get("toolCall")
        .and_then(|tc| tc.get("name"))
        .and_then(|n| n.as_str())
        .or_else(|| ctx.get("tool_name").and_then(|n| n.as_str()))
        .or_else(|| ctx.get("tool").and_then(|n| n.as_str()))
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_ask = tool_name == "ask"
        || tool_name.starts_with("ask_")
        || tool_name.ends_with("_ask")
        || tool_name.contains("_ask_");

    if tool_name.contains("question") || is_ask {
        "awaiting_input"
    } else if tool_name.contains("permission") {
        "permission"
    } else {
        "working"
    }
}

fn write_pane_state_file(pane_id: &str, state: &str) {
    let safe_pane: String = pane_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let data = serde_json::json!({
        "paneId": pane_id,
        "state": state,
        "message": serde_json::Value::Null,
        "timestamp": now_ms,
    });
    let file = std::env::temp_dir().join(format!("acpd-pane-state-{}.json", safe_pane));
    if let Ok(s) = serde_json::to_string(&data) {
        let _ = fs::write(file, s);
    }
}

fn send_acp_state(pane_id: &str, state: &str) {
    if pane_id.is_empty() {
        return;
    }
    write_pane_state_file(pane_id, state);
    if !post_agent_status(pane_id, state, None) && state == "idle" {
        let _ = Command::new("ai-sound-notify")
            .arg("response")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

fn run_tmux_hook(event_type: &str) -> i32 {
    let tmux_pane = get_active_tmux_pane();
    let ctx = read_stdin_json();

    match event_type {
        "SessionStart" | "PreInvocation" => {
            if !tmux_pane.is_empty() {
                let _ = tmux_status(&["rename-window", "-t", &tmux_pane, ""]);
            }
            if let Some(title) = extract_session_title(&ctx) {
                set_ai_agent_title(&tmux_pane, Some(&title));
            }
            send_acp_state(&tmux_pane, "working");
        }
        "PreToolUse" => {
            let state = classify_pre_tool_state(&ctx);
            send_acp_state(&tmux_pane, state);
        }
        "Stop" => {
            send_acp_state(&tmux_pane, "idle");
        }
        "SessionEnd" | "Exit" => {
            set_ai_agent_title(&tmux_pane, None);
            if !tmux_pane.is_empty() {
                let _ = tmux_status(&[
                    "set-window-option",
                    "-t",
                    &tmux_pane,
                    "automatic-rename",
                    "on",
                ]);
            }
            send_acp_state(&tmux_pane, "closed");
        }
        _ => {}
    }

    if event_type == "PreToolUse" {
        println!("{{\"decision\":\"allow\"}}");
    } else {
        println!("{{}}");
    }
    0
}

fn run_lazygit_hook(event_type: &str) -> i32 {
    let tmux_pane = get_active_tmux_pane();
    let ctx = read_stdin_json();

    let conv_id = ctx
        .get("conversationId")
        .or_else(|| ctx.get("sessionId"))
        .or_else(|| ctx.get("session_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("AGY_CONVERSATION_ID").ok())
        .unwrap_or_default();

    let workspace_path = ctx
        .get("workspacePath")
        .or_else(|| ctx.get("cwd"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
        .unwrap_or_default();

    if !tmux_pane.is_empty() && !workspace_path.is_empty() {
        let safe_path = sanitize_workspace_key(&workspace_path);
        let _ = fs::write(
            format!("/tmp/agy-active-pane-{}.txt", safe_path),
            &tmux_pane,
        );
    }

    if matches!(event_type, "SessionEnd" | "Exit") {
        let safe_path = sanitize_workspace_key(&workspace_path);
        let safe_conv = sanitize_workspace_key(&conv_id);
        let _ = fs::remove_file(format!(
            "/tmp/agy-registered-{}-{}.sentinel",
            safe_path, safe_conv
        ));
        set_lazygitrs_icon(&tmux_pane, None);
        println!("{{}}");
        return 0;
    }

    if !conv_id.is_empty() && !workspace_path.is_empty() {
        let safe_path = sanitize_workspace_key(&workspace_path);
        let safe_conv = sanitize_workspace_key(&conv_id);
        let sentinel = format!("/tmp/agy-registered-{}-{}.sentinel", safe_path, safe_conv);

        let mut initial_port = 47657u16;
        let port_file = Path::new(&workspace_path).join(".lazygitrs.port");
        if let Ok(ps) = fs::read_to_string(&port_file)
            && let Ok(p) = ps.trim().parse::<u16>()
        {
            initial_port = p;
        }

        if let Ok(stored) = fs::read_to_string(&sentinel)
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stored)
            && parsed.get("conversationId").and_then(|v| v.as_str()) == Some(conv_id.as_str())
        {
            let sport = parsed
                .get("port")
                .and_then(|v| v.as_u64())
                .map(|p| p as u16)
                .unwrap_or(initial_port);
            if let Some((200, sdata)) = http_get_json(sport, "/session-api/session", 50)
                && sdata.get("sessionId").and_then(|v| v.as_str()) == Some(conv_id.as_str())
            {
                set_lazygitrs_icon(&tmux_pane, Some("#[fg=#a6e3a1]#[fg=default]"));
                if event_type == "PreToolUse" {
                    println!("{{\"decision\":\"allow\"}}");
                } else {
                    println!("{{}}");
                }
                return 0;
            }
        }

        let exe_str = std::env::current_exe()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "waymux".to_string());
        let notify_cmd = format!("{} inject \"{}\" {{prompt}}", exe_str, workspace_path);

        if let Some((200, sdata)) = http_get_json(initial_port, "/session-api/session", 60) {
            let ws_match = sdata
                .get("workspacePath")
                .and_then(|v| v.as_str())
                .is_none_or(|w| w.is_empty() || w == workspace_path);
            if ws_match {
                let reg_payload = serde_json::json!({
                    "sessionId": conv_id,
                    "workspacePath": workspace_path,
                    "notifyCommand": notify_cmd,
                });
                let _ = http_post_json(
                    initial_port,
                    "/session-api/register",
                    None,
                    &reg_payload,
                    80,
                );
                let sentinel_val = serde_json::json!({
                    "conversationId": conv_id,
                    "port": initial_port,
                });
                let _ = fs::write(&sentinel, sentinel_val.to_string());
                set_lazygitrs_icon(&tmux_pane, Some("#[fg=#a6e3a1]#[fg=default]"));
            }
        }
    }

    if event_type == "PreToolUse" {
        println!("{{\"decision\":\"allow\"}}");
    } else {
        println!("{{}}");
    }
    0
}

pub fn run_cli(args: &[String]) -> i32 {
    let kind = args.first().map(|s| s.as_str()).unwrap_or("tmux");
    let event = args.get(1).map(|s| s.as_str()).unwrap_or("");
    match kind {
        "lazygit" => run_lazygit_hook(event),
        _ => run_tmux_hook(event),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_pre_tool_state() {
        let q = serde_json::json!({"toolCall": {"name": "ask_question"}});
        assert_eq!(classify_pre_tool_state(&q), "awaiting_input");

        let p = serde_json::json!({"toolCall": {"name": "request_permission"}});
        assert_eq!(classify_pre_tool_state(&p), "permission");

        let w = serde_json::json!({"toolCall": {"name": "view_file"}});
        assert_eq!(classify_pre_tool_state(&w), "working");
    }

    #[test]
    fn test_extract_session_title() {
        let ctx = serde_json::json!({"title": "Implement Waymux v0.2.0"});
        assert_eq!(
            extract_session_title(&ctx).as_deref(),
            Some("Implement Waymux v0.2.0")
        );
    }
}
