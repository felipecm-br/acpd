---
name: waymux
description: "Orchestrate Tmux, AI agent fleet states, Cockpit HUD popups, alert triage, and Git worktrees using Waymux (and the ACPD JSON-RPC 2.0 daemon at localhost:4040/rpc). Use this skill whenever you need to inspect or control Tmux panes/windows/sessions, wait on or dismiss agent states, manage isolated Git worktrees (`waymux wt`), or configure zero-latency AI hooks (`waymux hook`)."
---

# Waymux Orchestrator & Control Plane Skill

You are an AI agent operating inside a Tmux environment managed by **Waymux** (`waymux` / `acpd`). Waymux combines a sub-millisecond synchronous CLI with an asynchronous JSON-RPC 2.0 daemon listening on `http://127.0.0.1:4040/rpc`.

## 1. Synchronous CLI Commands (`waymux`)

All CLI subcommands execute synchronously in `<1.5ms` without initializing an async runtime:

- `waymux health`: Verify daemon health (`GET /health`).
- `waymux status`: Display daemon health and all active AI agent pane states.
- `waymux list`: List active AI agent pane states in TSV format (`pane_id`, `state`, `last_timestamp`).
- `waymux dismiss [pane_id]`: Clear attention/question/permission badge for a pane.
- `waymux cockpit`: Launch the interactive Golden-Ratio (`85% × 75%`) AI Fleet & Window Cockpit HUD via Waymaker (`wm`).
- `waymux triage --popup`: Open or cycle notifying AI agent panes in an isolated `_popups` overlay.
- `waymux triage --jump`: Jump focus directly to the next AI agent requiring attention (with origin-return memory).
- `waymux review-notes <count|list|send> <repo_path> [session] [idx] [pane_id]`: Inspect or atomically dispatch inline Lazygitrs code review notes (`.git/info/lines.json`) via bracketed paste.
- `waymux hook <tmux|lazygit> <Event>`: Native sub-10ms AI agent lifecycle hook handler (`PreInvocation`, `PreToolUse`, `PostToolUse`, `PostInvocation`, `Stop`, `SessionEnd`).
- `waymux inject <workspace_dir> <prompt>`: Bracketed-paste multi-line prompt injector into the active AI agent pane for a workspace.
- `waymux wt <subcommand>`: Native Git worktree orchestrator (`new`, `go`, `ls`, `rm`, `ship`, `reap`, `sweep`, `pick`).
- `waymux init <--tmux|--systemd|--hooks>`: Print portable configuration snippets.

## 2. JSON-RPC 2.0 Control Plane (`http://127.0.0.1:4040/rpc`)

Local requests authenticate using the Bearer token stored at `$XDG_RUNTIME_DIR/waymux/token` (or `/run/user/$UID/acpd/token`).

### Supported RPC Methods
- **Agent State**:
  - `agentState/update`: Update pane state (`working`, `idle`, `awaiting_input`, `question`, `permission`, `stalled`, `error`, `closed`).
  - `agentState/list`: List all tracked agent panes and timestamps.
  - `agentState/dismiss`: Clear attention badge on `pane_id`.
  - `agentState/wait`: Block until `pane_id` reaches `target_state` (with `timeout_secs`).
- **Tmux Orchestration**:
  - `tmux.list_sessions`, `tmux.new_session`, `tmux.kill_session`
  - `tmux.list_windows`, `tmux.new_window`, `tmux.select_window`, `tmux.kill_window`
  - `tmux.list_panes`, `tmux.split_pane`, `tmux.select_pane`, `tmux.kill_pane`
  - `tmux.capture_pane`, `tmux.send_keys`, `tmux.select_layout`
  - `tmux.display_message`, `tmux.ring_bell`

### Example RPC Call
```bash
TOKEN=$(cat "${XDG_RUNTIME_DIR:-/run/user/$UID}/waymux/token" 2>/dev/null || cat "/run/user/$UID/acpd/token")
curl -s -X POST http://127.0.0.1:4040/rpc \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"tmux.split_pane","params":{"vertical":true,"command":"cargo test"},"id":1}'
```
