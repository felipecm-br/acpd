use crate::adapters::{AgentState, AgentUpdate, OutputAdapter};
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PaneStateInfo {
    pub last_timestamp: u64,
    pub seq_id: u64,
    pub state: AgentState,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StallRecord {
    pub last_output_snapshot: String,
    pub last_cpu_ticks: u64,
    pub last_activity_instant: tokio::time::Instant,
}

#[derive(Clone)]
pub struct ApiState {
    pub adapters: Arc<Vec<Box<dyn OutputAdapter>>>,
    pub pending_idles:
        Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::JoinHandle<()>>>>,
    pub idle_debounce_ms: u64,
    pub pane_states: Arc<tokio::sync::Mutex<std::collections::HashMap<String, PaneStateInfo>>>,
    pub next_seq: Arc<std::sync::atomic::AtomicU64>,
    pub state_broadcast: tokio::sync::broadcast::Sender<AgentUpdate>,
    pub stall_records: Arc<tokio::sync::Mutex<std::collections::HashMap<String, StallRecord>>>,
}

impl ApiState {
    pub fn new(adapters: Vec<Box<dyn OutputAdapter>>, idle_debounce_ms: u64) -> Self {
        let (state_broadcast, _) = tokio::sync::broadcast::channel(128);
        Self {
            adapters: Arc::new(adapters),
            pending_idles: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            idle_debounce_ms,
            pane_states: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            next_seq: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            state_broadcast,
            stall_records: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub async fn clean_stale_panes(&self) {
        let registered_panes: Vec<String> = {
            let states = self.pane_states.lock().await;
            states.keys().cloned().collect()
        };

        if registered_panes.is_empty() {
            return;
        }

        let output = match tokio::process::Command::new("tmux")
            .args(["list-panes", "-a", "-F", "#{pane_id}"])
            .output()
            .await
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => String::new(),
        };

        let active_panes: std::collections::HashSet<&str> = output.lines().collect();

        for pane_id in registered_panes {
            if pane_id.starts_with('%') && !active_panes.contains(pane_id.as_str()) {
                tracing::info!("Cleaning up dead pane {}", pane_id);
                let update = AgentUpdate {
                    pane_id: pane_id.clone(),
                    state: AgentState::Closed,
                    message: Some("pane closed".into()),
                };

                {
                    let mut pending = self.pending_idles.lock().await;
                    if let Some(task) = pending.remove(&pane_id) {
                        task.abort();
                    }
                }

                for adapter in self.adapters.iter() {
                    if let Err(e) = adapter.update(&update).await {
                        tracing::error!("Adapter error on dead pane cleanup: {}", e);
                    }
                }

                let _ = self.state_broadcast.send(update);

                {
                    let mut states = self.pane_states.lock().await;
                    states.remove(&pane_id);
                }
                {
                    let mut stalls = self.stall_records.lock().await;
                    stalls.remove(&pane_id);
                }
            }
        }
    }

    pub async fn rehydrate_from_raw_lines(&self, lines: &str, now_secs: u64) {
        for line in lines.lines() {
            let mut parts = line.split('\t');
            let pane_id = match parts.next() {
                Some(id) if !id.trim().is_empty() => id.trim(),
                _ => continue,
            };
            let raw_state = match parts.next() {
                Some(s) if !s.trim().is_empty() => s.trim(),
                _ => continue,
            };

            if let Some(state) = AgentState::from_tmux_raw(raw_state) {
                let seq = self
                    .next_seq
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                {
                    let mut states = self.pane_states.lock().await;
                    states.insert(
                        pane_id.to_string(),
                        PaneStateInfo {
                            last_timestamp: now_secs,
                            seq_id: seq,
                            state: state.clone(),
                            message: None,
                        },
                    );
                }

                let update = AgentUpdate {
                    pane_id: pane_id.to_string(),
                    state,
                    message: None,
                };

                for adapter in self.adapters.iter() {
                    if let Err(e) = adapter.rehydrate(&update).await {
                        tracing::warn!("Adapter rehydrate error for pane {}: {}", pane_id, e);
                    }
                }
                let _ = self.state_broadcast.send(update.clone());
                tracing::info!("Rehydrated pane {} with state {:?}", pane_id, update.state);
            }
        }
    }

    pub async fn rehydrate_from_tmux(&self) {
        let output = match tokio::process::Command::new("tmux")
            .args([
                "list-panes",
                "-a",
                "-F",
                "#{pane_id}\t#{@ai_agent_state_raw}",
            ])
            .output()
            .await
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return,
        };

        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        self.rehydrate_from_raw_lines(&output, now_secs).await;
    }

    pub async fn check_pane_stalls(&self) {
        let target_panes: Vec<(String, AgentState)> = {
            let states = self.pane_states.lock().await;
            states
                .iter()
                .filter_map(|(pane_id, info)| {
                    if info.state == AgentState::Working || info.state == AgentState::Stalled {
                        Some((pane_id.clone(), info.state.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        };

        if target_panes.is_empty() {
            return;
        }

        for (pane_id, current_state) in target_panes {
            let display_out = tokio::process::Command::new("tmux")
                .args([
                    "display-message",
                    "-p",
                    "-t",
                    &pane_id,
                    "#{pane_pid} #{cursor_x} #{cursor_y} #{history_size}",
                ])
                .output()
                .await;

            let (pane_pid, cursor_info) = match display_out {
                Ok(o) if o.status.success() => {
                    let out_str = String::from_utf8_lossy(&o.stdout);
                    let mut parts = out_str.split_whitespace();
                    let pid = parts
                        .next()
                        .and_then(|p| p.parse::<u32>().ok())
                        .unwrap_or(0);
                    let rest = parts.collect::<Vec<&str>>().join(":");
                    (pid, rest)
                }
                _ => continue,
            };

            let capture_out = tokio::process::Command::new("tmux")
                .args(["capture-pane", "-p", "-t", &pane_id, "-S", "-5"])
                .output()
                .await;

            let text_snapshot = match capture_out {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            };

            let current_snapshot = format!("{}:{}", cursor_info, text_snapshot);
            let current_cpu = if pane_pid > 0 {
                get_process_tree_cpu_ticks(pane_pid)
            } else {
                0
            };

            let mut to_dispatch: Option<IncomingUpdate> = None;

            {
                let mut stalls = self.stall_records.lock().await;
                let record = stalls
                    .entry(pane_id.clone())
                    .or_insert_with(|| StallRecord {
                        last_output_snapshot: current_snapshot.clone(),
                        last_cpu_ticks: current_cpu,
                        last_activity_instant: tokio::time::Instant::now(),
                    });

                let output_changed = record.last_output_snapshot != current_snapshot;
                let cpu_changed = current_cpu > record.last_cpu_ticks;

                if output_changed || cpu_changed {
                    record.last_output_snapshot = current_snapshot;
                    record.last_cpu_ticks = current_cpu;
                    record.last_activity_instant = tokio::time::Instant::now();

                    if current_state == AgentState::Stalled {
                        tracing::info!(
                            "Pane {} resumed activity; recovering from Stalled to Working",
                            pane_id
                        );
                        to_dispatch = Some(IncomingUpdate {
                            pane_id: pane_id.clone(),
                            state: AgentState::Working,
                            message: Some("recovered from stall".into()),
                            timestamp: None,
                        });
                    }
                } else if current_state == AgentState::Working
                    && record.last_activity_instant.elapsed() >= std::time::Duration::from_secs(20)
                {
                    tracing::warn!(
                        "Pane {} stalled (>20s silence and 0 CPU delta). Transitioning to Stalled.",
                        pane_id
                    );
                    to_dispatch = Some(IncomingUpdate {
                        pane_id: pane_id.clone(),
                        state: AgentState::Stalled,
                        message: Some("agent stalled (no output / CPU for 20s)".into()),
                        timestamp: None,
                    });
                }
            }

            if let Some(update) = to_dispatch {
                dispatch_update(self, update).await;
            }
        }
    }
}

fn get_process_tree_cpu_ticks(root_pid: u32) -> u64 {
    let mut ppid_map: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut ticks_map: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();

    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let pid_str = match file_name.to_str() {
                Some(s) if s.chars().all(|c| c.is_ascii_digit()) => s,
                _ => continue,
            };
            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let stat_path = format!("/proc/{}/stat", pid);
            if let Ok(content) = std::fs::read_to_string(stat_path)
                && let Some(rparen) = content.rfind(')')
            {
                let rest = &content[rparen + 1..];
                let fields: Vec<&str> = rest.split_whitespace().collect();
                if fields.len() > 13
                    && let (Ok(ppid), Ok(utime), Ok(stime)) = (
                        fields[1].parse::<u32>(),
                        fields[11].parse::<u64>(),
                        fields[12].parse::<u64>(),
                    )
                {
                    ppid_map.entry(ppid).or_default().push(pid);
                    ticks_map.insert(pid, utime.saturating_add(stime));
                }
            }
        }
    }

    let mut total_ticks: u64 = 0;
    let mut queue = vec![root_pid];
    let mut visited = std::collections::HashSet::new();

    while let Some(curr) = queue.pop() {
        if !visited.insert(curr) {
            continue;
        }
        if let Some(&ticks) = ticks_map.get(&curr) {
            total_ticks = total_ticks.saturating_add(ticks);
        }
        if let Some(children) = ppid_map.get(&curr) {
            for &child in children {
                queue.push(child);
            }
        }
    }

    total_ticks
}

use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct StateUpdateParams {
    pub state: String,
    pub pane_id: String,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct DismissParams {
    pub pane_id: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct WaitParams {
    pub pane_id: String,
    pub target_state: Option<String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct SplitPaneParams {
    pub command: Option<String>,
    pub target_pane: Option<String>,
    pub vertical: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NewWindowParams {
    pub name: Option<String>,
    pub target: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewSessionParams {
    pub name: String,
    pub directory: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct TargetParams {
    pub target: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct CapturePaneParams {
    pub target: Option<String>,
    pub target_pane: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub escape_sequences: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListPanesParams {
    pub target: Option<String>,
    pub all: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListWindowsParams {
    pub target: Option<String>,
    pub target_session: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListSessionsParams {}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SendKeysParams {
    pub target: Option<String>,
    pub target_pane: Option<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    pub literal: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SelectWindowParams {
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SelectPaneParams {
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct DisplayMessageParams {
    pub message: String,
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct RingBellParams {
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct SelectLayoutParams {
    pub layout: String,
    pub target: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RpcResponse {
    Success {
        jsonrpc: String,
        result: Value,
        id: u64,
    },
    Error {
        jsonrpc: String,
        error: RpcError,
        id: u64,
    },
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct StatusRequest {
    pub agent: Option<String>,
    pub pane_id: String,
    pub state: String,
    pub message: Option<String>,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub success: bool,
}

fn log_flow_event(event: &str, target: &str, extra: &str) {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/fecavmi".to_string());
    let script = format!("{}/.config/tmux/flow-log.sh", home);
    let event = event.to_string();
    let target = target.to_string();
    let extra = extra.to_string();

    tokio::spawn(async move {
        if std::path::Path::new(&script).exists() {
            let _ = tokio::process::Command::new(&script)
                .args([&event, &target, &extra])
                .output()
                .await;
        } else {
            let log_dir = format!("{}/.local/state/flow", home);
            let _ = tokio::fs::create_dir_all(&log_dir).await;
            let log_path = format!("{}/events.tsv", log_dir);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let line = format!("{}\t{}\t{}\t{}\n", now, event, target, extra);
            use tokio::io::AsyncWriteExt;
            if let Ok(mut f) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .await
            {
                let _ = f.write_all(line.as_bytes()).await;
            }
        }
    });
}

#[derive(Debug, Clone)]
pub struct IncomingUpdate {
    pub pane_id: String,
    pub state: AgentState,
    pub message: Option<String>,
    pub timestamp: Option<u64>,
}

async fn dispatch_update(state: &ApiState, update: IncomingUpdate) {
    let seq = state
        .next_seq
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let (prev_state, should_proceed) = {
        let mut states = state.pane_states.lock().await;
        if let Some(existing) = states.get(&update.pane_id)
            && matches!(update.timestamp, Some(ts) if ts < existing.last_timestamp)
        {
            tracing::warn!(
                "Discarding stale state update for pane {}: incoming_ts {:?} < last_ts {}",
                update.pane_id,
                update.timestamp,
                existing.last_timestamp
            );
            (None, false)
        } else {
            let prev = states.get(&update.pane_id).map(|s| s.state.clone());
            states.insert(
                update.pane_id.clone(),
                PaneStateInfo {
                    last_timestamp: update.timestamp.unwrap_or(0),
                    seq_id: seq,
                    state: update.state.clone(),
                    message: update.message.clone(),
                },
            );
            (prev, true)
        }
    };

    if !should_proceed {
        return;
    }

    // Telemetry instrumentation: measure human response latency
    match (&prev_state, &update.state) {
        // Agent asks question / requests permission
        (prev, AgentState::AwaitingInput) if prev != &Some(AgentState::AwaitingInput) => {
            log_flow_event("agent_question", &update.pane_id, "");
        }
        (prev, AgentState::Permission) if prev != &Some(AgentState::Permission) => {
            log_flow_event("agent_question", &update.pane_id, "");
        }
        // Human answered question (transitioned from question/permission to working or idle)
        (Some(AgentState::AwaitingInput), AgentState::Working)
        | (Some(AgentState::AwaitingInput), AgentState::Idle)
        | (Some(AgentState::Permission), AgentState::Working)
        | (Some(AgentState::Permission), AgentState::Idle) => {
            log_flow_event("human_reply", &update.pane_id, "");
        }
        _ => {}
    }

    if update.state == AgentState::Working {
        let mut stalls = state.stall_records.lock().await;
        stalls.insert(
            update.pane_id.clone(),
            StallRecord {
                last_output_snapshot: String::new(),
                last_cpu_ticks: 0,
                last_activity_instant: tokio::time::Instant::now(),
            },
        );
    } else if update.state != AgentState::Stalled {
        let mut stalls = state.stall_records.lock().await;
        stalls.remove(&update.pane_id);
    }

    if update.state == AgentState::Idle {
        let mut pending = state.pending_idles.lock().await;
        if let Some(task) = pending.remove(&update.pane_id) {
            task.abort();
        }
        let adapters = state.adapters.clone();
        let update_clone = AgentUpdate {
            pane_id: update.pane_id.clone(),
            state: update.state.clone(),
            message: update.message.clone(),
        };
        let pane_id = update.pane_id.clone();
        let pending_map = state.pending_idles.clone();
        let pane_states = state.pane_states.clone();
        let broadcast_tx = state.state_broadcast.clone();
        let delay_ms = state.idle_debounce_ms;
        let prev_state_for_idle = prev_state.clone();

        let task = tokio::spawn(async move {
            if delay_ms > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
            }
            {
                let states = pane_states.lock().await;
                if states.get(&pane_id).is_some_and(|c| c.seq_id != seq) {
                    return;
                }
            }
            if prev_state_for_idle == Some(AgentState::Working) {
                log_flow_event("agent_finished", &pane_id, "");
            }
            for adapter in adapters.iter() {
                if let Err(e) = adapter.update(&update_clone).await {
                    tracing::error!("Adapter error: {}", e);
                }
            }
            let _ = broadcast_tx.send(update_clone);
            let mut pending = pending_map.lock().await;
            pending.remove(&pane_id);
        });
        pending.insert(update.pane_id.clone(), task);
    } else {
        {
            let mut pending = state.pending_idles.lock().await;
            if let Some(task) = pending.remove(&update.pane_id) {
                task.abort();
            }
        }
        let agent_update = AgentUpdate {
            pane_id: update.pane_id,
            state: update.state,
            message: update.message,
        };
        for adapter in state.adapters.iter() {
            if let Err(e) = adapter.update(&agent_update).await {
                tracing::error!("Adapter error: {}", e);
            }
        }
        let _ = state.state_broadcast.send(agent_update);
    }
}

async fn handle_rpc(
    State(adapters): State<ApiState>,
    Json(payload): Json<RpcRequest>,
) -> Json<RpcResponse> {
    tracing::info!("Received RPC: method={} id={}", payload.method, payload.id);

    match payload.method.as_str() {
        "agentState/update" => match serde_json::from_value::<StateUpdateParams>(payload.params) {
            Ok(params) => match params.state.parse::<AgentState>() {
                Ok(state) => {
                    let update = IncomingUpdate {
                        pane_id: params.pane_id,
                        state,
                        message: None,
                        timestamp: params.timestamp,
                    };
                    dispatch_update(&adapters, update).await;
                    Json(RpcResponse::Success {
                        jsonrpc: "2.0".into(),
                        result: serde_json::json!("ok"),
                        id: payload.id,
                    })
                }
                Err(err_msg) => Json(RpcResponse::Error {
                    jsonrpc: "2.0".into(),
                    error: RpcError {
                        code: -32602,
                        message: err_msg,
                    },
                    id: payload.id,
                }),
            },
            Err(e) => json_invalid_params(e, payload.id),
        },
        "agentState/list" => {
            let states = adapters.pane_states.lock().await;
            let mut map = serde_json::Map::new();
            for (pane_id, info) in states.iter() {
                let mut pane_obj = serde_json::json!({
                    "state": info.state.as_str(),
                    "last_timestamp": info.last_timestamp,
                });
                if let Some(ref msg) = info.message {
                    pane_obj["message"] = serde_json::json!(msg);
                }
                map.insert(pane_id.clone(), pane_obj);
            }
            Json(RpcResponse::Success {
                jsonrpc: "2.0".into(),
                result: serde_json::Value::Object(map),
                id: payload.id,
            })
        }
        "agentState/dismiss" | "agent_state/dismiss" => {
            match parse_rpc_params::<DismissParams>(payload.params) {
                Ok(params) => {
                    let pane_id = params.pane_id.or(params.target).unwrap_or_default();
                    if pane_id.is_empty() {
                        return Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32602,
                                message: "Missing pane_id parameter".into(),
                            },
                            id: payload.id,
                        });
                    }

                    let current_state = {
                        let states = adapters.pane_states.lock().await;
                        states.get(&pane_id).map(|s| s.state.clone())
                    };

                    match current_state {
                        Some(AgentState::AwaitingInput)
                        | Some(AgentState::Permission)
                        | Some(AgentState::Stalled)
                        | Some(AgentState::Error) => {
                            tracing::info!("Dismissing attention state for pane {}", pane_id);
                            dispatch_update(
                                &adapters,
                                IncomingUpdate {
                                    pane_id: pane_id.clone(),
                                    state: AgentState::Idle,
                                    message: Some("dismissed by user focus".into()),
                                    timestamp: None,
                                },
                            )
                            .await;

                            let _ = tokio::process::Command::new("tmux")
                                .args(["set", "-g", "@ai_agent_bell", ""])
                                .output()
                                .await;
                            let _ = tokio::process::Command::new("tmux")
                                .args(["refresh-client", "-S"])
                                .output()
                                .await;

                            Json(RpcResponse::Success {
                                jsonrpc: "2.0".into(),
                                result: serde_json::json!({
                                    "status": "dismissed",
                                    "pane_id": pane_id,
                                }),
                                id: payload.id,
                            })
                        }
                        Some(AgentState::Working) => Json(RpcResponse::Success {
                            jsonrpc: "2.0".into(),
                            result: serde_json::json!({
                                "status": "ignored",
                                "pane_id": pane_id,
                                "reason": "agent_working",
                            }),
                            id: payload.id,
                        }),
                        _ => {
                            let _ = tokio::process::Command::new("tmux")
                                .args(["set", "-g", "@ai_agent_bell", ""])
                                .output()
                                .await;
                            Json(RpcResponse::Success {
                                jsonrpc: "2.0".into(),
                                result: serde_json::json!({
                                    "status": "noop",
                                    "pane_id": pane_id,
                                }),
                                id: payload.id,
                            })
                        }
                    }
                }
                Err(e) => json_invalid_params(e, payload.id),
            }
        }
        "agentState/wait" | "agent_state/wait" => {
            match parse_rpc_params::<WaitParams>(payload.params) {
                Ok(params) => {
                    if params.pane_id.is_empty() {
                        return Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32602,
                                message: "Missing pane_id parameter".into(),
                            },
                            id: payload.id,
                        });
                    }

                    let target_state = match params
                        .target_state
                        .as_deref()
                        .unwrap_or("idle")
                        .parse::<AgentState>()
                    {
                        Ok(s) => s,
                        Err(e) => {
                            return Json(RpcResponse::Error {
                                jsonrpc: "2.0".into(),
                                error: RpcError {
                                    code: -32602,
                                    message: format!("Invalid target_state: {}", e),
                                },
                                id: payload.id,
                            });
                        }
                    };

                    // Check if current state already matches target_state
                    {
                        let states = adapters.pane_states.lock().await;
                        if let Some(info) = states.get(&params.pane_id) {
                            if info.state == target_state {
                                return Json(RpcResponse::Success {
                                    jsonrpc: "2.0".into(),
                                    result: serde_json::json!({
                                        "status": "reached",
                                        "pane_id": params.pane_id,
                                        "state": target_state.as_str(),
                                    }),
                                    id: payload.id,
                                });
                            }
                        } else if target_state == AgentState::Idle {
                            return Json(RpcResponse::Success {
                                jsonrpc: "2.0".into(),
                                result: serde_json::json!({
                                    "status": "reached",
                                    "pane_id": params.pane_id,
                                    "state": "idle",
                                }),
                                id: payload.id,
                            });
                        }
                    }

                    let mut rx = adapters.state_broadcast.subscribe();
                    let pane_id = params.pane_id.clone();
                    let timeout_secs = params.timeout_secs.unwrap_or(300);

                    let wait_fut = async {
                        loop {
                            match rx.recv().await {
                                Ok(update) => {
                                    if update.pane_id == pane_id {
                                        if update.state == target_state {
                                            return Ok((
                                                "reached",
                                                update.state.as_str().to_string(),
                                            ));
                                        } else if update.state == AgentState::Closed {
                                            return Ok(("closed", "closed".to_string()));
                                        }
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    let states = adapters.pane_states.lock().await;
                                    if let Some(info) = states.get(&pane_id) {
                                        if info.state == target_state {
                                            return Ok((
                                                "reached",
                                                target_state.as_str().to_string(),
                                            ));
                                        } else if info.state == AgentState::Closed {
                                            return Ok(("closed", "closed".to_string()));
                                        }
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    return Err("Broadcast channel closed".to_string());
                                }
                            }
                        }
                    };

                    match tokio::time::timeout(
                        std::time::Duration::from_secs(timeout_secs),
                        wait_fut,
                    )
                    .await
                    {
                        Ok(Ok((status, state_str))) => Json(RpcResponse::Success {
                            jsonrpc: "2.0".into(),
                            result: serde_json::json!({
                                "status": status,
                                "pane_id": pane_id,
                                "state": state_str,
                            }),
                            id: payload.id,
                        }),
                        Ok(Err(err)) => Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32000,
                                message: err,
                            },
                            id: payload.id,
                        }),
                        Err(_) => {
                            tracing::debug!(
                                "RPC agentState/wait connection timed out after {}s for pane {}",
                                timeout_secs,
                                pane_id
                            );
                            let current_state = {
                                let states = adapters.pane_states.lock().await;
                                states
                                    .get(&pane_id)
                                    .map(|s| s.state.as_str().to_string())
                                    .unwrap_or_else(|| "unknown".into())
                            };
                            Json(RpcResponse::Success {
                                jsonrpc: "2.0".into(),
                                result: serde_json::json!({
                                    "status": "timeout",
                                    "pane_id": pane_id,
                                    "current_state": current_state,
                                }),
                                id: payload.id,
                            })
                        }
                    }
                }
                Err(e) => json_invalid_params(e, payload.id),
            }
        }
        "tmux.split_pane" => {
            match serde_json::from_value::<SplitPaneParams>(payload.params) {
                Ok(params) => {
                    let mut cmd = tokio::process::Command::new("tmux");
                    cmd.arg("split-window");

                    if params.vertical.unwrap_or(false) {
                        cmd.arg("-h"); // tmux -h is vertical split
                    } else {
                        cmd.arg("-v"); // tmux -v is horizontal split
                    }

                    if let Some(target) = params.target_pane {
                        cmd.arg("-t").arg(target);
                    }

                    if let Some(command) = params.command {
                        cmd.arg(&command);
                    }

                    execute_tmux_cmd(cmd, payload.id, "pane created").await
                }
                Err(e) => Json(RpcResponse::Error {
                    jsonrpc: "2.0".into(),
                    error: RpcError {
                        code: -32602,
                        message: format!("Invalid params: {}", e),
                    },
                    id: payload.id,
                }),
            }
        }
        "tmux.new_window" => match serde_json::from_value::<NewWindowParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("new-window");
                if let Some(name) = params.name {
                    cmd.arg("-n").arg(name);
                }
                if let Some(target) = params.target {
                    cmd.arg("-t").arg(target);
                }
                if let Some(command) = params.command {
                    cmd.arg(&command);
                }
                execute_tmux_cmd(cmd, payload.id, "window created").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.new_session" => match serde_json::from_value::<NewSessionParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("new-session").arg("-d").arg("-s").arg(params.name);
                if let Some(dir) = params.directory {
                    cmd.arg("-c").arg(dir);
                }
                if let Some(command) = params.command {
                    cmd.arg(&command);
                }
                execute_tmux_cmd(cmd, payload.id, "session created").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.kill_pane" => match serde_json::from_value::<TargetParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("kill-pane").arg("-t").arg(params.target);
                execute_tmux_cmd(cmd, payload.id, "pane killed").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.kill_window" => match serde_json::from_value::<TargetParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("kill-window").arg("-t").arg(params.target);
                execute_tmux_cmd(cmd, payload.id, "window killed").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.kill_session" => match serde_json::from_value::<TargetParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("kill-session").arg("-t").arg(params.target);
                execute_tmux_cmd(cmd, payload.id, "session killed").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.capture_pane" => match parse_rpc_params::<CapturePaneParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("capture-pane").arg("-p");
                if let Some(target) = params.target.or(params.target_pane) {
                    cmd.arg("-t").arg(target);
                }
                if let Some(start) = params.start_line {
                    cmd.arg("-S").arg(start.to_string());
                }
                if let Some(end) = params.end_line {
                    cmd.arg("-E").arg(end.to_string());
                }
                if params.escape_sequences.unwrap_or(false) {
                    cmd.arg("-e");
                }
                match cmd.output().await {
                    Ok(output) if output.status.success() => {
                        let content = String::from_utf8_lossy(&output.stdout).to_string();
                        Json(RpcResponse::Success {
                            jsonrpc: "2.0".into(),
                            result: serde_json::json!({ "content": content }),
                            id: payload.id,
                        })
                    }
                    Ok(output) => {
                        let err_msg = String::from_utf8_lossy(&output.stderr);
                        Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32000,
                                message: format!("Tmux error: {}", err_msg),
                            },
                            id: payload.id,
                        })
                    }
                    Err(e) => Json(RpcResponse::Error {
                        jsonrpc: "2.0".into(),
                        error: RpcError {
                            code: -32000,
                            message: format!("Failed to execute tmux: {}", e),
                        },
                        id: payload.id,
                    }),
                }
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.list_panes" => match parse_rpc_params::<ListPanesParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("list-panes");
                cmd.arg("-F").arg(
                    "#{pane_id}\t#{pane_active}\t#{pane_width}\t#{pane_height}\t#{pane_current_path}\t#{pane_current_command}",
                );
                if params.all.unwrap_or(false) {
                    cmd.arg("-a");
                } else if let Some(target) = params.target {
                    cmd.arg("-t").arg(target);
                }
                match cmd.output().await {
                    Ok(output) if output.status.success() => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let panes: Vec<serde_json::Value> = stdout
                            .lines()
                            .filter_map(|line| {
                                let parts: Vec<&str> = line.split('\t').collect();
                                if parts.len() >= 6 {
                                    Some(serde_json::json!({
                                        "pane_id": parts[0],
                                        "active": parts[1] == "1",
                                        "width": parts[2].parse::<u32>().unwrap_or(0),
                                        "height": parts[3].parse::<u32>().unwrap_or(0),
                                        "current_path": parts[4],
                                        "current_command": parts[5],
                                    }))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        Json(RpcResponse::Success {
                            jsonrpc: "2.0".into(),
                            result: serde_json::json!(panes),
                            id: payload.id,
                        })
                    }
                    Ok(output) => {
                        let err_msg = String::from_utf8_lossy(&output.stderr);
                        Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32000,
                                message: format!("Tmux error: {}", err_msg),
                            },
                            id: payload.id,
                        })
                    }
                    Err(e) => Json(RpcResponse::Error {
                        jsonrpc: "2.0".into(),
                        error: RpcError {
                            code: -32000,
                            message: format!("Failed to execute tmux: {}", e),
                        },
                        id: payload.id,
                    }),
                }
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.list_windows" => match parse_rpc_params::<ListWindowsParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("list-windows");
                cmd.arg("-F").arg(
                    "#{window_id}\t#{window_name}\t#{window_index}\t#{window_active}\t#{window_panes}",
                );
                if let Some(target) = params.target.or(params.target_session) {
                    cmd.arg("-t").arg(target);
                }
                match cmd.output().await {
                    Ok(output) if output.status.success() => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let windows: Vec<serde_json::Value> = stdout
                            .lines()
                            .filter_map(|line| {
                                let parts: Vec<&str> = line.split('\t').collect();
                                if parts.len() >= 5 {
                                    Some(serde_json::json!({
                                        "window_id": parts[0],
                                        "window_name": parts[1],
                                        "active": parts[3] == "1",
                                        "panes": parts[4].parse::<u32>().unwrap_or(0),
                                    }))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        Json(RpcResponse::Success {
                            jsonrpc: "2.0".into(),
                            result: serde_json::json!(windows),
                            id: payload.id,
                        })
                    }
                    Ok(output) => {
                        let err_msg = String::from_utf8_lossy(&output.stderr);
                        Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32000,
                                message: format!("Tmux error: {}", err_msg),
                            },
                            id: payload.id,
                        })
                    }
                    Err(e) => Json(RpcResponse::Error {
                        jsonrpc: "2.0".into(),
                        error: RpcError {
                            code: -32000,
                            message: format!("Failed to execute tmux: {}", e),
                        },
                        id: payload.id,
                    }),
                }
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.list_sessions" => match parse_rpc_params::<ListSessionsParams>(payload.params) {
            Ok(_) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("list-sessions");
                cmd.arg("-F")
                    .arg("#{session_id}\t#{session_name}\t#{session_windows}\t#{session_attached}");
                match cmd.output().await {
                    Ok(output) if output.status.success() => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let sessions: Vec<serde_json::Value> = stdout
                            .lines()
                            .filter_map(|line| {
                                let parts: Vec<&str> = line.split('\t').collect();
                                if parts.len() >= 4 {
                                    Some(serde_json::json!({
                                        "session_name": parts[1],
                                        "windows": parts[2].parse::<u32>().unwrap_or(0),
                                        "attached": parts[3] == "1",
                                    }))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        Json(RpcResponse::Success {
                            jsonrpc: "2.0".into(),
                            result: serde_json::json!(sessions),
                            id: payload.id,
                        })
                    }
                    Ok(output) => {
                        let err_msg = String::from_utf8_lossy(&output.stderr);
                        Json(RpcResponse::Error {
                            jsonrpc: "2.0".into(),
                            error: RpcError {
                                code: -32000,
                                message: format!("Tmux error: {}", err_msg),
                            },
                            id: payload.id,
                        })
                    }
                    Err(e) => Json(RpcResponse::Error {
                        jsonrpc: "2.0".into(),
                        error: RpcError {
                            code: -32000,
                            message: format!("Failed to execute tmux: {}", e),
                        },
                        id: payload.id,
                    }),
                }
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.send_keys" => match serde_json::from_value::<SendKeysParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("send-keys");
                if let Some(target) = params.target.or(params.target_pane) {
                    cmd.arg("-t").arg(target);
                }
                if params.literal.unwrap_or(false) {
                    cmd.arg("-l");
                }
                for key in params.keys {
                    cmd.arg(&key);
                }
                execute_tmux_cmd(cmd, payload.id, "keys sent").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.select_window" => match parse_rpc_params::<SelectWindowParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("select-window");
                if let Some(target) = params.target {
                    cmd.arg("-t").arg(target);
                }
                execute_tmux_cmd(cmd, payload.id, "window selected").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.select_pane" => match parse_rpc_params::<SelectPaneParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("select-pane");
                if let Some(target) = params.target {
                    cmd.arg("-t").arg(target);
                }
                execute_tmux_cmd(cmd, payload.id, "pane selected").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.display_message" => {
            match serde_json::from_value::<DisplayMessageParams>(payload.params) {
                Ok(params) => {
                    let mut cmd = tokio::process::Command::new("tmux");
                    cmd.arg("display-message");
                    if let Some(target) = params.target {
                        cmd.arg("-t").arg(target);
                    }
                    cmd.arg(params.message);
                    execute_tmux_cmd(cmd, payload.id, "message displayed").await
                }
                Err(e) => json_invalid_params(e, payload.id),
            }
        }
        "tmux.ring_bell" => match parse_rpc_params::<RingBellParams>(payload.params) {
            Ok(params) => {
                let mut cmd = tokio::process::Command::new("tmux");
                cmd.arg("send-keys");
                if let Some(target) = params.target {
                    cmd.arg("-t").arg(target);
                }
                cmd.arg("C-g");
                execute_tmux_cmd(cmd, payload.id, "bell triggered").await
            }
            Err(e) => json_invalid_params(e, payload.id),
        },
        "tmux.select_layout" => {
            match serde_json::from_value::<SelectLayoutParams>(payload.params) {
                Ok(params) => {
                    let mut cmd = tokio::process::Command::new("tmux");
                    cmd.arg("select-layout");
                    if let Some(target) = params.target {
                        cmd.arg("-t").arg(target);
                    }
                    cmd.arg(params.layout);
                    execute_tmux_cmd(cmd, payload.id, "layout applied").await
                }
                Err(e) => json_invalid_params(e, payload.id),
            }
        }
        _ => Json(RpcResponse::Error {
            jsonrpc: "2.0".into(),
            error: RpcError {
                code: -32601,
                message: "Method not found".into(),
            },
            id: payload.id,
        }),
    }
}

async fn handle_status(
    State(adapters): State<ApiState>,
    Json(payload): Json<StatusRequest>,
) -> Result<Json<StatusResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    tracing::info!("Received REST status: {:?}", payload);
    match payload.state.parse::<AgentState>() {
        Ok(state) => {
            let update = IncomingUpdate {
                pane_id: payload.pane_id.clone(),
                state,
                message: payload.message.clone(),
                timestamp: payload.timestamp,
            };
            dispatch_update(&adapters, update).await;
            Ok(Json(StatusResponse { success: true }))
        }
        Err(err_msg) => Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "success": false,
                "error": err_msg
            })),
        )),
    }
}

pub async fn auth_middleware(
    State(token): State<Arc<String>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let authenticated = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .is_some_and(|b| b.trim() == token.as_str());

    if authenticated {
        Ok(next.run(req).await)
    } else {
        tracing::warn!("Unauthorized HTTP request to {}", req.uri().path());
        Err(axum::http::StatusCode::UNAUTHORIZED)
    }
}

pub fn api_router(adapters: ApiState, auth_token: Arc<String>) -> Router {
    let protected = Router::new()
        .route("/rpc", post(handle_rpc))
        .route("/api/status", post(handle_status))
        .route_layer(axum::middleware::from_fn_with_state(
            auth_token,
            auth_middleware,
        ))
        .with_state(adapters);

    Router::new().merge(protected)
}

async fn execute_tmux_cmd(
    mut cmd: tokio::process::Command,
    id: u64,
    success_msg: &str,
) -> Json<RpcResponse> {
    match cmd.output().await {
        Ok(output) if output.status.success() => Json(RpcResponse::Success {
            jsonrpc: "2.0".into(),
            result: serde_json::json!(success_msg),
            id,
        }),
        Ok(output) => {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            Json(RpcResponse::Error {
                jsonrpc: "2.0".into(),
                error: RpcError {
                    code: -32000,
                    message: format!("Tmux error: {}", err_msg),
                },
                id,
            })
        }
        Err(e) => Json(RpcResponse::Error {
            jsonrpc: "2.0".into(),
            error: RpcError {
                code: -32000,
                message: format!("Failed to execute tmux: {}", e),
            },
            id,
        }),
    }
}

fn json_invalid_params(e: serde_json::Error, id: u64) -> Json<RpcResponse> {
    Json(RpcResponse::Error {
        jsonrpc: "2.0".into(),
        error: RpcError {
            code: -32602,
            message: format!("Invalid params: {}", e),
        },
        id,
    })
}

fn parse_rpc_params<T: serde::de::DeserializeOwned + Default>(
    params: Value,
) -> Result<T, serde_json::Error> {
    if params.is_null() {
        Ok(T::default())
    } else {
        serde_json::from_value(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AgentState;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    struct TestAdapter {
        updates: Arc<StdMutex<Vec<AgentUpdate>>>,
    }

    #[async_trait]
    impl OutputAdapter for TestAdapter {
        async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
            self.updates.lock().unwrap().push(update.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_stale_update_discarded() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 0);

        // Newer update at t=2000
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Working,
                message: None,
                timestamp: Some(2000),
            },
        )
        .await;

        // Stale update at t=1000
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Idle,
                message: None,
                timestamp: Some(1000),
            },
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recorded = updates.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].state, AgentState::Working);
    }

    #[tokio::test]
    async fn test_idle_superseded_by_working() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 100);

        // Idle update at t=1000 with 100ms debounce
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Idle,
                message: None,
                timestamp: Some(1000),
            },
        )
        .await;

        // Working update at t=1050 arrives before 100ms debounce finishes
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".into(),
                state: AgentState::Working,
                message: None,
                timestamp: Some(1050),
            },
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let recorded = updates.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].state, AgentState::Working);
    }

    #[test]
    fn test_rpc_param_deserialization() {
        let capture_json = serde_json::json!({ "target_pane": "%1", "start_line": -100, "escape_sequences": true });
        let capture_params: CapturePaneParams = serde_json::from_value(capture_json).unwrap();
        assert_eq!(capture_params.target_pane, Some("%1".into()));
        assert_eq!(capture_params.start_line, Some(-100));
        assert_eq!(capture_params.escape_sequences, Some(true));

        let send_keys_json = serde_json::json!({ "target": "%2", "keys": ["ls -la", "Enter"] });
        let send_params: SendKeysParams = serde_json::from_value(send_keys_json).unwrap();
        assert_eq!(send_params.target, Some("%2".into()));
        assert_eq!(send_params.keys, vec!["ls -la", "Enter"]);

        let select_win_json = serde_json::json!({ "target": "@1" });
        let select_win_params: SelectWindowParams =
            serde_json::from_value(select_win_json).unwrap();
        assert_eq!(select_win_params.target, Some("@1".into()));

        let select_pane_json = serde_json::json!({ "target": "%1" });
        let select_pane_params: SelectPaneParams =
            serde_json::from_value(select_pane_json).unwrap();
        assert_eq!(select_pane_params.target, Some("%1".into()));

        let display_msg_json = serde_json::json!({ "message": "Hello", "target": "@0" });
        let display_msg_params: DisplayMessageParams =
            serde_json::from_value(display_msg_json).unwrap();
        assert_eq!(display_msg_params.message, "Hello");
        assert_eq!(display_msg_params.target, Some("@0".into()));

        let ring_bell_json = serde_json::json!({ "target": "%0" });
        let ring_bell_params: RingBellParams = serde_json::from_value(ring_bell_json).unwrap();
        assert_eq!(ring_bell_params.target, Some("%0".into()));

        let layout_json = serde_json::json!({ "layout": "even-horizontal", "target": "@0" });
        let layout_params: SelectLayoutParams = serde_json::from_value(layout_json).unwrap();
        assert_eq!(layout_params.layout, "even-horizontal");
        assert_eq!(layout_params.target, Some("@0".into()));
    }

    #[tokio::test]
    async fn test_clean_stale_panes_removes_dead_pane() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 0);

        // Register a fake pane %999999 that definitely doesn't exist in tmux
        state.pane_states.lock().await.insert(
            "%999999".into(),
            PaneStateInfo {
                last_timestamp: 1000,
                seq_id: 1,
                state: AgentState::Working,
                message: None,
            },
        );

        state.clean_stale_panes().await;

        // Verify pane %999999 was removed from memory
        assert!(!state.pane_states.lock().await.contains_key("%999999"));

        // Verify closed update was sent to adapter
        let recorded = updates.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].pane_id, "%999999");
        assert_eq!(recorded[0].state, AgentState::Closed);
    }

    #[test]
    fn test_strict_agent_state_parsing() {
        assert_eq!("working".parse::<AgentState>(), Ok(AgentState::Working));
        assert_eq!("idle".parse::<AgentState>(), Ok(AgentState::Idle));
        assert_eq!(
            "permission".parse::<AgentState>(),
            Ok(AgentState::Permission)
        );
        let err = "workin".parse::<AgentState>().unwrap_err();
        assert!(err.contains("Unknown agent state 'workin'"));
        assert!(err.contains("Expected one of: working, idle"));
    }

    #[tokio::test]
    async fn test_auth_middleware_header_check() {
        let token = Arc::new("secret-token-123".to_string());

        let req = axum::http::Request::builder()
            .uri("/api/status")
            .header("Authorization", "Bearer secret-token-123")
            .body(axum::body::Body::empty())
            .unwrap();

        let auth_str = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(auth_str, "Bearer secret-token-123");
        let bearer = auth_str.strip_prefix("Bearer ").unwrap();
        assert_eq!(bearer.trim(), token.as_str());
    }

    #[tokio::test]
    async fn test_rehydrate_from_raw_lines() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 0);

        let sample_tmux_output =
            "%4\tbusy\n%21\tidle\n%23\t\n%27\tquestion\n%30\tpermission\n%31\terror\n";
        state
            .rehydrate_from_raw_lines(sample_tmux_output, 12345)
            .await;

        let pane_states = state.pane_states.lock().await;
        assert_eq!(pane_states.len(), 5);
        assert_eq!(pane_states.get("%4").unwrap().state, AgentState::Working);
        assert_eq!(pane_states.get("%21").unwrap().state, AgentState::Idle);
        assert!(!pane_states.contains_key("%23"));
        assert_eq!(
            pane_states.get("%27").unwrap().state,
            AgentState::AwaitingInput
        );
        assert_eq!(
            pane_states.get("%30").unwrap().state,
            AgentState::Permission
        );
        assert_eq!(pane_states.get("%31").unwrap().state, AgentState::Error);
    }

    #[test]
    fn test_from_tmux_raw() {
        assert_eq!(AgentState::from_tmux_raw("busy"), Some(AgentState::Working));
        assert_eq!(
            AgentState::from_tmux_raw("working"),
            Some(AgentState::Working)
        );
        assert_eq!(
            AgentState::from_tmux_raw("question"),
            Some(AgentState::AwaitingInput)
        );
        assert_eq!(
            AgentState::from_tmux_raw("awaiting_input"),
            Some(AgentState::AwaitingInput)
        );
        assert_eq!(
            AgentState::from_tmux_raw("permission"),
            Some(AgentState::Permission)
        );
        assert_eq!(
            AgentState::from_tmux_raw("stalled"),
            Some(AgentState::Stalled)
        );
        assert_eq!(
            AgentState::from_tmux_raw("hanging"),
            Some(AgentState::Stalled)
        );
        assert_eq!(AgentState::from_tmux_raw("error"), Some(AgentState::Error));
        assert_eq!(AgentState::from_tmux_raw("idle"), Some(AgentState::Idle));
        assert_eq!(AgentState::from_tmux_raw(""), None);
        assert_eq!(AgentState::from_tmux_raw("   "), None);
        assert_eq!(AgentState::from_tmux_raw("other_junk"), None);
    }

    #[tokio::test]
    async fn test_agent_state_dismiss_and_wait() {
        let updates = Arc::new(StdMutex::new(Vec::new()));
        let adapter = TestAdapter {
            updates: updates.clone(),
        };
        let state = ApiState::new(vec![Box::new(adapter)], 0);

        // 1. Set pane to AwaitingInput
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".to_string(),
                state: AgentState::AwaitingInput,
                message: None,
                timestamp: Some(100),
            },
        )
        .await;

        // Dismiss attention
        let dismiss_req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "agentState/dismiss".to_string(),
            params: serde_json::json!({ "pane_id": "%1" }),
            id: 1,
        };
        let res = handle_rpc(State(state.clone()), Json(dismiss_req)).await;
        match res.0 {
            RpcResponse::Success { result, .. } => {
                assert_eq!(result.get("status").unwrap(), "dismissed");
            }
            _ => panic!("Expected dismiss success"),
        }

        // Verify state is now Idle
        {
            let states = state.pane_states.lock().await;
            assert_eq!(states.get("%1").unwrap().state, AgentState::Idle);
        }

        // 2. Wait for idle when already idle should return immediately
        let wait_req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "agentState/wait".to_string(),
            params: serde_json::json!({ "pane_id": "%1", "target_state": "idle", "timeout_secs": 1 }),
            id: 2,
        };
        let wait_res = handle_rpc(State(state.clone()), Json(wait_req)).await;
        match wait_res.0 {
            RpcResponse::Success { result, .. } => {
                assert_eq!(result.get("status").unwrap(), "reached");
                assert_eq!(result.get("state").unwrap(), "idle");
            }
            _ => panic!("Expected wait success"),
        }

        // 3. When working, dismiss should be ignored
        dispatch_update(
            &state,
            IncomingUpdate {
                pane_id: "%1".to_string(),
                state: AgentState::Working,
                message: None,
                timestamp: Some(200),
            },
        )
        .await;

        let dismiss_working_req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "agentState/dismiss".to_string(),
            params: serde_json::json!({ "pane_id": "%1" }),
            id: 3,
        };
        let dis_work_res = handle_rpc(State(state.clone()), Json(dismiss_working_req)).await;
        match dis_work_res.0 {
            RpcResponse::Success { result, .. } => {
                assert_eq!(result.get("status").unwrap(), "ignored");
            }
            _ => panic!("Expected dismiss ignored"),
        }
    }
}
