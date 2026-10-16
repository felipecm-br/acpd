use crate::config::{AgentStateTheme, SoundConfig, Spinner, ThemeConfig};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    AwaitingInput,
    Permission,
    Stalled,
    Error,
    Closed,
}

impl AgentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::AwaitingInput => "awaiting_input",
            AgentState::Permission => "permission",
            AgentState::Stalled => "stalled",
            AgentState::Error => "error",
            AgentState::Closed => "closed",
        }
    }

    pub fn from_tmux_raw(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "busy" | "working" => Some(AgentState::Working),
            "question" | "awaiting_input" => Some(AgentState::AwaitingInput),
            "permission" => Some(AgentState::Permission),
            "stalled" | "hanging" => Some(AgentState::Stalled),
            "error" => Some(AgentState::Error),
            "idle" => Some(AgentState::Idle),
            _ => None,
        }
    }
}

impl std::str::FromStr for AgentState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "idle" => Ok(AgentState::Idle),
            "working" | "busy" => Ok(AgentState::Working),
            "awaiting_input" | "question" => Ok(AgentState::AwaitingInput),
            "permission" => Ok(AgentState::Permission),
            "stalled" | "hanging" => Ok(AgentState::Stalled),
            "error" => Ok(AgentState::Error),
            "closed" => Ok(AgentState::Closed),
            invalid => Err(format!(
                "Invalid params: Unknown agent state '{}'. Expected one of: working, idle, awaiting_input, permission, stalled, error, closed.",
                invalid
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUpdate {
    pub pane_id: String,
    pub state: AgentState,
    pub message: Option<String>,
}

#[async_trait]
pub trait OutputAdapter: Send + Sync {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()>;
    async fn rehydrate(&self, _update: &AgentUpdate) -> anyhow::Result<()> {
        Ok(())
    }
}

// ==========================================
// WAYBAR ADAPTER
// ==========================================
fn get_waybar_state_path() -> String {
    let xdg_runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    format!("{}/acpd-waybar-state.json", xdg_runtime)
}

fn urgency_score(state: &AgentState) -> u8 {
    match state {
        AgentState::Permission => 6,
        AgentState::AwaitingInput => 5,
        AgentState::Stalled => 4,
        AgentState::Error => 3,
        AgentState::Working => 2,
        AgentState::Idle => 1,
        AgentState::Closed => 0,
    }
}

pub struct WaybarAdapter {
    theme: Option<ThemeConfig>,
    pane_updates: Arc<Mutex<HashMap<String, AgentUpdate>>>,
}

impl WaybarAdapter {
    pub fn new(theme: Option<ThemeConfig>) -> Self {
        Self {
            theme,
            pane_updates: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl OutputAdapter for WaybarAdapter {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        let active_update = {
            let mut map = self.pane_updates.lock().await;
            if update.state == AgentState::Closed {
                map.remove(&update.pane_id);
            } else {
                map.insert(update.pane_id.clone(), update.clone());
            }

            map.values()
                .filter(|u| u.state != AgentState::Closed && u.state != AgentState::Idle)
                .max_by_key(|u| urgency_score(&u.state))
                .cloned()
        };

        if let Some(active) = active_update {
            let (raw_state, tooltip) = match active.state {
                AgentState::Idle => ("idle", "Agent Idle"),
                AgentState::Working => ("busy", "Agent Working"),
                AgentState::AwaitingInput => ("question", "Awaiting Input"),
                AgentState::Permission => ("permission", "Permission Required"),
                AgentState::Stalled => ("stalled", "Agent Stalled / Hanging"),
                AgentState::Error => ("error", "Agent Error"),
                AgentState::Closed => ("idle", "Agent Closed"),
            };

            let (icon, color) = if let Some(theme) = &self.theme {
                if let Some(state_theme) = theme.states.get(raw_state) {
                    (state_theme.icon.clone(), state_theme.color.clone())
                } else if raw_state == "stalled" {
                    ("󱥁".to_string(), "#fab387".to_string())
                } else {
                    ("".to_string(), "#ffffff".to_string())
                }
            } else if raw_state == "stalled" {
                ("󱥁".to_string(), "#fab387".to_string())
            } else {
                ("".to_string(), "#ffffff".to_string())
            };

            let json_payload = serde_json::json!({
                "text": icon,
                "tooltip": tooltip,
                "class": raw_state,
                "color": color
            });

            let content = json_payload.to_string();

            let path = get_waybar_state_path();
            if let Err(e) = tokio::fs::write(&path, content).await {
                tracing::error!("Failed to write waybar state file: {}", e);
            }
            tracing::info!("WaybarAdapter updated to: {}", raw_state);
        } else {
            let path = get_waybar_state_path();
            let _ = tokio::fs::remove_file(&path).await;
            tracing::info!("WaybarAdapter: All agents idle/closed, cleared waybar state");
        }

        // Pkill waybar to trigger custom module update
        if let Err(e) = Command::new("pkill")
            .args(["-RTMIN+13", "waybar"])
            .output()
            .await
        {
            tracing::error!("Failed to send signal to waybar: {}", e);
        }

        Ok(())
    }

    async fn rehydrate(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        self.update(update).await
    }
}

// ==========================================
// TMUX ADAPTER
// ==========================================
async fn set_tmux_window_state(pane_id: &str, icon: &str, raw: &str, color: &str) {
    let out = Command::new("tmux")
        .args([
            "set-option",
            "-w",
            "-t",
            pane_id,
            "@ai_agent_state",
            icon,
            ";",
            "set-option",
            "-w",
            "-t",
            pane_id,
            "@ai_agent_state_raw",
            raw,
            ";",
            "set-option",
            "-w",
            "-t",
            pane_id,
            "@ai_agent_state_color",
            color,
            ";",
            "refresh-client",
            "-S",
        ])
        .output()
        .await;
    if let Ok(o) = out
        && !o.status.success()
    {
        tracing::warn!(
            "Tmux set_tmux_window_state failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

async fn unset_tmux_window_state(pane_id: &str) {
    let out = Command::new("tmux")
        .args([
            "set-option",
            "-w",
            "-u",
            "-t",
            pane_id,
            "@ai_agent_state",
            ";",
            "set-option",
            "-w",
            "-u",
            "-t",
            pane_id,
            "@ai_agent_state_raw",
            ";",
            "set-option",
            "-w",
            "-u",
            "-t",
            pane_id,
            "@ai_agent_state_color",
            ";",
            "refresh-client",
            "-S",
        ])
        .output()
        .await;
    if let Ok(o) = out
        && !o.status.success()
    {
        tracing::warn!(
            "Tmux unset_tmux_window_state failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[allow(dead_code)]
async fn set_tmux_option(pane_id: &str, option: &str, value: &str) {
    let out = Command::new("tmux")
        .args(["set-option", "-w", "-t", pane_id, option, value])
        .output()
        .await;
    if let Ok(o) = out
        && !o.status.success()
    {
        tracing::warn!(
            "Tmux set-option {} failed: {}",
            option,
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[allow(dead_code)]
async fn unset_tmux_option(pane_id: &str, option: &str) {
    let out = Command::new("tmux")
        .args(["set-option", "-w", "-u", "-t", pane_id, option])
        .output()
        .await;
    if let Ok(o) = out
        && !o.status.success()
    {
        tracing::warn!(
            "Tmux unset-option {} failed: {}",
            option,
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[allow(dead_code)]
async fn refresh_tmux_client() {
    let out = Command::new("tmux")
        .args(["refresh-client", "-S"])
        .output()
        .await;
    if let Ok(o) = out
        && !o.status.success()
    {
        tracing::warn!(
            "Tmux refresh-client failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

pub struct TmuxAdapter {
    spinners: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    pending_idles: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    pane_states: Arc<Mutex<HashMap<String, AgentState>>>,
    spinner_frames: Vec<String>,
    spinner_interval: u64,
    theme: Option<ThemeConfig>,
}

impl TmuxAdapter {
    pub fn new(theme: Option<ThemeConfig>, active_spinner: Option<Spinner>) -> Self {
        let (frames, interval) = match active_spinner {
            Some(s) if !s.frames.is_empty() => (s.frames, s.interval),
            _ => (vec![], 0),
        };
        Self {
            spinners: Arc::new(Mutex::new(HashMap::new())),
            pending_idles: Arc::new(Mutex::new(HashMap::new())),
            pane_states: Arc::new(Mutex::new(HashMap::new())),
            spinner_frames: frames,
            spinner_interval: interval,
            theme,
        }
    }

    async fn stop_spinner(&self, pane_id: &str) {
        let mut spinners = self.spinners.lock().await;
        if let Some(task) = spinners.remove(pane_id) {
            task.abort();
        }
    }

    async fn start_spinner(&self, pane_id: String) {
        self.stop_spinner(&pane_id).await;

        let pane_clone = pane_id.clone();
        let frames = self.spinner_frames.clone();
        let interval = self.spinner_interval;
        let (icon, color) = self
            .theme
            .as_ref()
            .and_then(|t| t.states.get("busy"))
            .map(|s| (s.icon.clone(), s.color.clone()))
            .unwrap_or_else(|| ("󰑮".to_string(), "#f9e2af".to_string()));

        let task = tokio::spawn(async move {
            if frames.is_empty() {
                set_tmux_window_state(&pane_clone, &icon, "busy", &color).await;
                return;
            }

            if frames.len() == 1 {
                set_tmux_window_state(&pane_clone, &frames[0], "busy", &color).await;
                return;
            }

            let mut i = 0;

            loop {
                let frame = &frames[i % frames.len()];

                set_tmux_window_state(&pane_clone, frame, "busy", &color).await;

                i += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(interval)).await;
            }
        });

        self.spinners.lock().await.insert(pane_id, task);
    }

    async fn trigger_bell(&self, pane_id: &str, action: &str, force: bool) {
        // Fetch session and window properties for the pane
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-t",
                pane_id,
                "-p",
                "#S|#{window_id}|#I|#W",
            ])
            .output()
            .await;

        if let Ok(out) = output {
            let props = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<&str> = props.trim().split('|').collect();
            if parts.len() == 4 {
                let session = parts[0];
                let window_id = parts[1];
                let window_idx = parts[2];
                let window_name = parts[3];

                // Check if any other client is looking elsewhere
                let clients_out = Command::new("tmux")
                    .args(["list-clients", "-F", "#{client_session} #{window_id}"])
                    .output()
                    .await;

                let mut notify = true;
                if !force && let Ok(cout) = clients_out {
                    let clients_str = String::from_utf8_lossy(&cout.stdout);
                    let any_other_client = clients_str.lines().any(|line| {
                        let mut iter = line.split_whitespace();
                        if let (Some(c_sess), Some(c_wid)) = (iter.next(), iter.next()) {
                            !(c_sess == session && c_wid == window_id)
                        } else {
                            false
                        }
                    });

                    if !any_other_client {
                        notify = false;
                    }
                }

                if notify {
                    let _ = Command::new("tmux")
                        .args(["set", "-g", "@ai_agent_last_bell", pane_id])
                        .output()
                        .await;
                    let msg = format!(
                        "  #[fg=cyan]{}:{} › {} #[fg=yellow](i)#[fg=default]",
                        window_idx, window_name, action
                    );
                    let _ = Command::new("tmux")
                        .args(["set", "-g", "@ai_agent_bell", &msg])
                        .output()
                        .await;
                    let _ = Command::new("tmux")
                        .args(["refresh-client", "-S"])
                        .output()
                        .await;

                    // Clear after 7 seconds
                    tokio::spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(7)).await;
                        let _ = Command::new("tmux")
                            .args(["set", "-g", "@ai_agent_bell", ""])
                            .output()
                            .await;
                        let _ = Command::new("tmux")
                            .args(["refresh-client", "-S"])
                            .output()
                            .await;
                    });
                }
            }
        }
    }
}

#[async_trait]
impl OutputAdapter for TmuxAdapter {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        // Cancel any pending idle debounce task for this pane
        {
            let mut pending = self.pending_idles.lock().await;
            if let Some(task) = pending.remove(&update.pane_id) {
                task.abort();
            }
        }

        // Debounce Idle state to prevent rapid flickering during tool call chains
        if update.state == AgentState::Idle {
            let pane_states = Arc::clone(&self.pane_states);
            let spinners = Arc::clone(&self.spinners);
            let theme = self.theme.clone();
            let pane_id = update.pane_id.clone();

            let task = tokio::spawn(async move {
                // Check deduplication
                {
                    let mut states = pane_states.lock().await;
                    if let Some(prev) = states.get(&pane_id)
                        && prev == &AgentState::Idle
                    {
                        return;
                    }
                    states.insert(pane_id.clone(), AgentState::Idle);
                }

                // Stop any running spinner
                {
                    let mut sp = spinners.lock().await;
                    if let Some(t) = sp.remove(&pane_id) {
                        t.abort();
                    }
                }

                let default_theme = AgentStateTheme {
                    icon: "?".to_string(),
                    color: "white".to_string(),
                };
                let t = theme
                    .as_ref()
                    .and_then(|th| th.states.get("idle"))
                    .unwrap_or(&default_theme);

                set_tmux_window_state(&pane_id, &t.icon, "idle", &t.color).await;

                tracing::info!("TmuxAdapter: Debounced update pane {} to Idle", pane_id);
            });

            self.pending_idles
                .lock()
                .await
                .insert(update.pane_id.clone(), task);
            return Ok(());
        }

        // State deduplication: ignore redundant updates for the same state on the same pane
        {
            let mut states = self.pane_states.lock().await;
            if let Some(prev) = states.get(&update.pane_id)
                && prev == &update.state
            {
                return Ok(());
            }
            if update.state == AgentState::Closed {
                states.remove(&update.pane_id);
            } else {
                states.insert(update.pane_id.clone(), update.state.clone());
            }
        }

        match &update.state {
            AgentState::Working => {
                self.start_spinner(update.pane_id.clone()).await;
            }
            state => {
                self.stop_spinner(&update.pane_id).await;

                // Dynamic colors from theme
                let default_theme = AgentStateTheme {
                    icon: "?".to_string(),
                    color: "white".to_string(),
                };

                let (icon, color, raw) = match state {
                    AgentState::Closed => {
                        // Clear the variables instead of setting them
                        unset_tmux_window_state(&update.pane_id).await;
                        tracing::info!("TmuxAdapter: Cleared variables for Closed state");
                        return Ok(());
                    }
                    AgentState::Idle => unreachable!(),
                    AgentState::AwaitingInput => {
                        let t = self
                            .theme
                            .as_ref()
                            .and_then(|th| th.states.get("question"))
                            .unwrap_or(&default_theme);
                        (t.icon.clone(), t.color.clone(), "question")
                    }
                    AgentState::Permission => {
                        let t = self
                            .theme
                            .as_ref()
                            .and_then(|th| th.states.get("permission"))
                            .unwrap_or(&default_theme);
                        (t.icon.clone(), t.color.clone(), "permission")
                    }
                    AgentState::Stalled => {
                        let t = self
                            .theme
                            .as_ref()
                            .and_then(|th| th.states.get("stalled"))
                            .cloned()
                            .unwrap_or(AgentStateTheme {
                                icon: "󱥁".to_string(),
                                color: "#fab387".to_string(),
                            });
                        (t.icon, t.color, "stalled")
                    }
                    AgentState::Error => {
                        let t = self
                            .theme
                            .as_ref()
                            .and_then(|th| th.states.get("error"))
                            .unwrap_or(&default_theme);
                        (t.icon.clone(), t.color.clone(), "error")
                    }
                    _ => unreachable!(),
                };

                set_tmux_window_state(&update.pane_id, &icon, raw, &color).await;

                // Trigger a bell based on state
                let (action_msg, force_bell) = match state {
                    AgentState::AwaitingInput => (Some("󱜻 question"), true),
                    AgentState::Permission => (Some("󱅭 permission"), true),
                    AgentState::Stalled => (Some("󱥁 stalled"), true),
                    _ => (None, false),
                };

                if let Some(msg) = action_msg {
                    self.trigger_bell(&update.pane_id, msg, force_bell).await;
                }
            }
        }

        tracing::info!(
            "TmuxAdapter: Updated pane {} to {:?}",
            update.pane_id,
            update.state
        );
        Ok(())
    }

    async fn rehydrate(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        // Record pane state in internal cache to prevent redundant updates
        {
            let mut states = self.pane_states.lock().await;
            states.insert(update.pane_id.clone(), update.state.clone());
        }

        // If the agent was working, resume spinner animation without bells
        if update.state == AgentState::Working {
            self.start_spinner(update.pane_id.clone()).await;
        }
        Ok(())
    }
}

// ==========================================
// SOUND ADAPTER
// ==========================================
pub struct SoundAdapter {
    config: Option<SoundConfig>,
    pane_states: Arc<Mutex<HashMap<String, AgentState>>>,
}

impl SoundAdapter {
    pub fn new(config: Option<SoundConfig>) -> Self {
        Self {
            config,
            pane_states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn expand_home(path: &str) -> String {
        if path.starts_with("~/")
            && let Ok(home) = std::env::var("HOME")
        {
            return format!("{}{}", home, &path[1..]);
        }
        path.to_string()
    }

    fn resolve_sound_path(&self, event_type: &str) -> Option<String> {
        let home = std::env::var("HOME").ok()?;

        // Check explicit mute marker files
        let mute_markers = [
            format!("{}/.config/omarchy/sounds/muted", home),
            format!("{}/.config/acpd/sounds/muted", home),
        ];
        for marker in &mute_markers {
            if std::path::Path::new(marker).exists() {
                return None;
            }
        }

        // 1. Check runtime state pointer files first (allows instant hot-switching without restart)
        let mut state_pointers = vec![
            format!("{}/.config/omarchy/sounds/ai-{}.sound", home, event_type),
            format!("{}/.config/acpd/sounds/{}.sound", home, event_type),
        ];
        if event_type == "response" {
            state_pointers.push(format!("{}/.config/omarchy/sounds/ai-response.sound", home));
            state_pointers.push(format!("{}/.config/acpd/sounds/response.sound", home));
        }

        for pointer in &state_pointers {
            if let Ok(content) = std::fs::read_to_string(pointer) {
                let trimmed = content.trim();
                // Explicitly muted or disabled
                if trimmed.eq_ignore_ascii_case("none")
                    || trimmed.eq_ignore_ascii_case("off")
                    || trimmed.eq_ignore_ascii_case("disabled")
                    || trimmed.eq_ignore_ascii_case("muted")
                {
                    return None;
                }
                if !trimmed.is_empty() {
                    let expanded = Self::expand_home(trimmed);
                    if std::path::Path::new(&expanded).is_file() {
                        return Some(expanded);
                    }
                }
            }
        }

        // 2. Check direct symlinks/files in user config
        let mut direct_files = vec![
            format!("{}/.config/omarchy/sounds/ai-{}.wav", home, event_type),
            format!("{}/.config/omarchy/sounds/ai-{}.oga", home, event_type),
            format!("{}/.config/acpd/sounds/{}.wav", home, event_type),
        ];
        if event_type == "response" {
            direct_files.push(format!("{}/.config/omarchy/sounds/ai-response.wav", home));
        }

        for path in &direct_files {
            if std::path::Path::new(path).is_file() {
                return Some(path.clone());
            }
        }

        // 3. Check explicit config from toml
        if let Some(sound_cfg) = &self.config {
            let custom_path = match event_type {
                "response" => sound_cfg.response.as_deref(),
                "question" => sound_cfg
                    .question
                    .as_deref()
                    .or(sound_cfg.response.as_deref()),
                "permission" => sound_cfg
                    .permission
                    .as_deref()
                    .or(sound_cfg.response.as_deref()),
                "error" => sound_cfg.error.as_deref(),
                _ => None,
            };

            if let Some(p) = custom_path {
                let expanded = Self::expand_home(p);
                if std::path::Path::new(&expanded).is_file() {
                    return Some(expanded);
                }
            }
        }

        // 4. Fallback defaults
        let default_sound = match event_type {
            "question" => format!("{}/.local/share/sounds/ai/02-gentle-ping.wav", home),
            "permission" => format!("{}/.local/share/sounds/ai/06-cyber-pulse.wav", home),
            "error" => format!("{}/.local/share/sounds/ai/10-arcade-blip.wav", home),
            _ => format!("{}/.local/share/sounds/ai/01-crystal-chime.wav", home),
        };

        let fallbacks = [
            default_sound,
            format!("{}/.local/share/sounds/ai/01-crystal-chime.wav", home),
            "/usr/share/sounds/freedesktop/stereo/complete.oga".to_string(),
            "/usr/share/sounds/freedesktop/stereo/message.oga".to_string(),
            "/usr/share/sounds/freedesktop/stereo/bell.oga".to_string(),
        ];

        for fb in &fallbacks {
            if std::path::Path::new(fb).is_file() {
                return Some(fb.clone());
            }
        }

        None
    }

    fn play(&self, event_type: &'static str) {
        if let Some(cfg) = &self.config
            && !cfg.enabled
        {
            return;
        }

        let sound_path = match self.resolve_sound_path(event_type) {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "SoundAdapter: No sound file resolved for event '{}'",
                    event_type
                );
                return;
            }
        };

        let player = self
            .config
            .as_ref()
            .and_then(|c| c.player.clone())
            .unwrap_or_else(|| "pw-play".to_string());

        tokio::spawn(async move {
            tracing::info!(
                "SoundAdapter: Playing {} sound from '{}' via {}",
                event_type,
                sound_path,
                player
            );
            let result = Command::new(&player).arg(&sound_path).output().await;

            if let Err(e) = result {
                tracing::warn!("SoundAdapter: Failed to execute player '{}': {}", player, e);
                if player == "pw-play" {
                    let _ = Command::new("paplay").arg(&sound_path).output().await;
                }
            }
        });
    }
}

#[async_trait]
impl OutputAdapter for SoundAdapter {
    async fn update(&self, update: &AgentUpdate) -> anyhow::Result<()> {
        let prev_state = {
            let mut states = self.pane_states.lock().await;
            let prev = states.get(&update.pane_id).cloned();
            if update.state == AgentState::Closed {
                states.remove(&update.pane_id);
            } else {
                states.insert(update.pane_id.clone(), update.state.clone());
            }
            prev
        };

        match (&prev_state, &update.state) {
            // When agent transitions from Working (or active state) to Idle -> AI response finished!
            (Some(AgentState::Working), AgentState::Idle)
            | (Some(AgentState::AwaitingInput), AgentState::Idle)
            | (Some(AgentState::Permission), AgentState::Idle)
            | (Some(AgentState::Stalled), AgentState::Idle) => {
                self.play("response");
            }
            // When agent transitions to AwaitingInput -> AI asking question / input
            (_, AgentState::AwaitingInput) if prev_state != Some(AgentState::AwaitingInput) => {
                self.play("question");
            }
            // When agent transitions to Permission -> AI asking permission
            (_, AgentState::Permission) if prev_state != Some(AgentState::Permission) => {
                self.play("permission");
            }
            // When agent transitions to Stalled
            (_, AgentState::Stalled) if prev_state != Some(AgentState::Stalled) => {
                self.play("error");
            }
            // When agent transitions to Error
            (_, AgentState::Error) if prev_state != Some(AgentState::Error) => {
                self.play("error");
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_tmux_adapter_updates() {
        let mut states = HashMap::new();
        states.insert(
            "idle".to_string(),
            AgentStateTheme {
                icon: "󱥂".to_string(),
                color: "#94e2d5".to_string(),
            },
        );
        states.insert(
            "busy".to_string(),
            AgentStateTheme {
                icon: "󰝲".to_string(),
                color: "#f9e2af".to_string(),
            },
        );
        let theme = ThemeConfig {
            active_spinner: "dot".to_string(),
            states,
        };
        let adapter = TmuxAdapter::new(Some(theme), None);

        // Test non-closed state update
        let update_idle = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::Idle,
            message: None,
        };
        assert!(adapter.update(&update_idle).await.is_ok());

        // Test closed state update
        let update_closed = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::Closed,
            message: None,
        };
        assert!(adapter.update(&update_closed).await.is_ok());

        // Test working (spinner) state update
        let update_working = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::Working,
            message: None,
        };
        assert!(adapter.update(&update_working).await.is_ok());
        adapter.stop_spinner("%1").await;
    }

    #[tokio::test]
    async fn test_sound_adapter_updates() {
        let sound_cfg = SoundConfig {
            enabled: false, // Disabled in test so no actual audio process runs
            player: Some("true".to_string()),
            response: Some("/dev/null".to_string()),
            question: Some("/dev/null".to_string()),
            permission: Some("/dev/null".to_string()),
            error: Some("/dev/null".to_string()),
        };
        let adapter = SoundAdapter::new(Some(sound_cfg));

        let update_working = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::Working,
            message: None,
        };
        assert!(adapter.update(&update_working).await.is_ok());

        // Working -> Idle (should trigger response sound logic)
        let update_idle = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::Idle,
            message: None,
        };
        assert!(adapter.update(&update_idle).await.is_ok());

        // Question
        let update_question = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::AwaitingInput,
            message: None,
        };
        assert!(adapter.update(&update_question).await.is_ok());

        // Closed
        let update_closed = AgentUpdate {
            pane_id: "%1".to_string(),
            state: AgentState::Closed,
            message: None,
        };
        assert!(adapter.update(&update_closed).await.is_ok());
    }

    #[test]
    fn test_stalled_agent_state_parsing() {
        assert_eq!("stalled".parse::<AgentState>(), Ok(AgentState::Stalled));
        assert_eq!("hanging".parse::<AgentState>(), Ok(AgentState::Stalled));
        assert_eq!(
            AgentState::from_tmux_raw("stalled"),
            Some(AgentState::Stalled)
        );
        assert_eq!(
            AgentState::from_tmux_raw("hanging"),
            Some(AgentState::Stalled)
        );
        assert_eq!(AgentState::Stalled.as_str(), "stalled");
    }

    #[test]
    fn test_resolve_sound_path_isolation() {
        let temp_dir = std::env::temp_dir().join(format!("acpd-sound-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let resp_file = temp_dir.join("resp.wav");
        let quest_file = temp_dir.join("quest.wav");
        std::fs::write(&resp_file, b"resp").unwrap();
        std::fs::write(&quest_file, b"quest").unwrap();

        let old_home = std::env::var("HOME").ok();
        unsafe {
            std::env::set_var("HOME", &temp_dir);
        }

        let sound_cfg = SoundConfig {
            enabled: true,
            player: Some("pw-play".to_string()),
            response: Some(resp_file.to_string_lossy().to_string()),
            question: Some(quest_file.to_string_lossy().to_string()),
            permission: None,
            error: None,
        };
        let adapter = SoundAdapter::new(Some(sound_cfg));

        // When question is configured, it should resolve to quest_file, NOT fallback to response (resp_file)
        let resolved_quest = adapter.resolve_sound_path("question");
        let resolved_resp = adapter.resolve_sound_path("response");

        if let Some(h) = old_home {
            unsafe {
                std::env::set_var("HOME", h);
            }
        }

        assert_eq!(
            resolved_quest,
            Some(quest_file.to_string_lossy().to_string())
        );
        assert_eq!(resolved_resp, Some(resp_file.to_string_lossy().to_string()));

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
