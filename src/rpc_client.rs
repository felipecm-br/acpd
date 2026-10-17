use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn read_token() -> Option<String> {
    let uid = nix::unistd::getuid();
    let mut candidates = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        candidates.push(PathBuf::from(xdg).join("acpd").join("token"));
    }
    candidates.push(PathBuf::from(format!("/run/user/{}/acpd/token", uid)));
    candidates.push(PathBuf::from(format!("/tmp/acpd-{}/token", uid)));
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".cache")
                .join("acpd")
                .join("token"),
        );
    }
    candidates.push(PathBuf::from("/run/user/1001/acpd/token"));

    for path in candidates {
        if let Ok(tok) = fs::read_to_string(&path) {
            let trimmed = tok.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn http_request(
    method: &str,
    path: &str,
    port: u16,
    token: Option<&str>,
    body: Option<&str>,
    timeout_ms: u64,
) -> Option<(u16, String)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let timeout = Duration::from_millis(timeout_ms);
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let body_str = body.unwrap_or("");
    let auth_header = match token {
        Some(t) if !t.is_empty() => format!("Authorization: Bearer {}\r\n", t),
        _ => String::new(),
    };
    let content_headers = if body.is_some() {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body_str.len()
        )
    } else {
        String::new()
    };

    let req = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{}{}Connection: close\r\n\r\n{}",
        method, path, port, auth_header, content_headers, body_str
    );

    stream.write_all(req.as_bytes()).ok()?;
    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp);
    let resp_str = String::from_utf8_lossy(&resp);

    let (headers, resp_body) = resp_str.split_once("\r\n\r\n")?;
    let status_line = headers.lines().next()?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())?;

    Some((status_code, resp_body.to_string()))
}

pub fn http_get_json(port: u16, path: &str, timeout_ms: u64) -> Option<(u16, serde_json::Value)> {
    let (code, body) = http_request("GET", path, port, None, None, timeout_ms)?;
    let val = serde_json::from_str(&body).ok()?;
    Some((code, val))
}

pub fn http_post_json(
    port: u16,
    path: &str,
    token: Option<&str>,
    payload: &serde_json::Value,
    timeout_ms: u64,
) -> Option<(u16, String)> {
    let body = serde_json::to_string(payload).ok()?;
    http_request("POST", path, port, token, Some(&body), timeout_ms)
}

#[derive(Debug, Clone)]
pub struct AgentPaneEntry {
    pub state: String,
    pub last_timestamp: u64,
}

pub fn fetch_agent_states() -> HashMap<String, AgentPaneEntry> {
    let mut map = HashMap::new();
    let token = match read_token() {
        Some(t) => t,
        None => return map,
    };

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "agentState/list",
        "id": 1
    });

    if let Some((200, body)) = http_post_json(4040, "/rpc", Some(&token), &payload, 150)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&body)
        && let Some(res_obj) = val.get("result").and_then(|r| r.as_object())
    {
        for (pane_id, info) in res_obj {
            let state = info
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let ts = info
                .get("last_timestamp")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            if !state.is_empty() {
                map.insert(
                    pane_id.clone(),
                    AgentPaneEntry {
                        state,
                        last_timestamp: ts,
                    },
                );
            }
        }
    }

    map
}

pub fn post_agent_status(pane_id: &str, state: &str, message: Option<&str>) -> bool {
    let token = read_token();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let payload = serde_json::json!({
        "pane_id": pane_id,
        "state": state,
        "message": message,
        "timestamp": now_ms,
    });

    matches!(
        http_post_json(4040, "/api/status", token.as_deref(), &payload, 250),
        Some((code, _)) if (200..300).contains(&code)
    )
}

pub fn check_health_cli() -> i32 {
    match http_request("GET", "/health", 4040, None, None, 1500) {
        Some((200, body)) => {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                println!("healthy");
            } else {
                println!("{}", trimmed);
            }
            0
        }
        Some((code, _)) => {
            eprintln!("unhealthy: unexpected HTTP {} from acpd", code);
            1
        }
        None => {
            eprintln!("acpd is not running or unreachable on port 4040");
            1
        }
    }
}
