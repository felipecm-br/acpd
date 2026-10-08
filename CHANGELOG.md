# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.1.1] - 2026-10-08

### 🚀 Features

- Rehydrate agent states from tmux pane options on daemon startup
- Add fast-path CLI flags `--version`, `--help`, and `health` check command
- Centralize idle debounce configuration via `config.toml`

## [0.1.0] - 2026-09-24

### 🚀 Features

- Initial release of ACPD: Agent Client Protocol Daemon
- Multiplex AI agent status to Tmux and Waybar
- Support JSON-RPC control plane over local TCP and Tailscale
