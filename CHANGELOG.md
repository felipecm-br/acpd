# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.2.0] - 2026-10-17

### 🚀 Features

- **Unified `waymux` Orchestrator Binary**: Consolidate `acpd` daemon, Cockpit HUD (`waymux cockpit`, `cockpit-items`, `cockpit-preview`, `cockpit-diff`, `cockpit-help`), guarded agent actions (`agent-action`), native review notes (`review-notes`), alert ring triage (`waymux triage --popup|--jump`), zero-Node lifecycle hooks (`waymux hook tmux|lazygit`), bracketed-paste prompt injection (`waymux inject`), Git worktree orchestration (`waymux wt`), and zero-config bootstrapper (`waymux init`) into a single ~5 MB binary.
- **Sub-Millisecond Synchronous CLI Fast-Path**: Execute all CLI subcommands synchronously before initializing the Tokio multi-threaded runtime or tracing subscriber, slashing hook latency from ~35ms to <2ms and Cockpit HUD item generation from ~50ms to ~14ms.
- **Embedded Waymaker Presets**: Embed `cockpit-picker.toml` via `include_str!` and materialize dynamically to `$XDG_RUNTIME_DIR/waymux/`.
- **Native `/proc` Process & Git Scanner**: Replace `ps`, `jq`, and `python3` subprocesses with native Rust `/proc/<pid>/stat` inspection and `serde_json` parsing.

### 🐛 Bug Fixes & Improvements

- Normalize epoch timestamp units (seconds vs milliseconds) when computing elapsed agent age in `cockpit-items`.
- Flush Rust stdout buffers before spawning child `delta` / `bat` streams in `cockpit-diff`.

## [0.1.2] - 2026-10-17

### 🚀 Features

- Implement proactive supervision with stall detection, auto-dismiss, and wait RPC
- Allow `question` and `busy` aliases in agent state parsing
- Instrument agent question and human reply latency tracking for evidence-based telemetry

### 🐛 Bug Fixes & Improvements

- Preserve bell notifications when window is detached and add connection timeout logging
- Aggregate multi-pane urgency states per window in `TmuxAdapter`
- Batch Tmux option updates and aggregate highest urgency for Waybar
- Isolate event sound resolution and protect auth tokens in test suites

### 🚀 Features

- Rehydrate agent states from tmux pane options on daemon startup
- Add fast-path CLI flags `--version`, `--help`, and `health` check command
- Centralize idle debounce configuration via `config.toml`

## [0.1.0] - 2026-09-24

### 🚀 Features

- Initial release of ACPD: Agent Client Protocol Daemon
- Multiplex AI agent status to Tmux and Waybar
- Support JSON-RPC control plane over local TCP and Tailscale
