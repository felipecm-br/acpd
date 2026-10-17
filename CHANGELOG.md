# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

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
