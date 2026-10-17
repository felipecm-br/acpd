# AGENTS.md

## Project Overview

**Waymux** (`waymux` / `acpd`) is a unified Rust orchestrator and headless JSON-RPC 2.0 control plane daemon that bridges AI coding agent lifecycle events (`agy`, `claude`, `codex`, `opencode`, `crush`), Tmux state/UI surfaces, Cockpit HUD popups (`wm`), inline code review notes (`lazygitrs`), and Git worktrees (`awt`).

- **Language:** Rust (edition 2024)
- **Binaries:** `waymux` (`src/bin/waymux.rs`) and `acpd` (`src/main.rs`), sharing 100% of `src/lib.rs`
- **Fast-Path CLI:** Synchronous execution (`<1.5ms`) before Tokio runtime initialization
- **Daemon Runtime:** Tokio multi-threaded async runtime + Axum 0.8 (`127.0.0.1:4040`)
- **Deployment:** User-level systemd service (`systemd/waymux.service`, `Type=notify`, `Alias=acpd.service`)

---

## Architecture & Module Map

- `src/lib.rs`: Synchronous CLI fast-path router and async Tokio daemon entrypoint.
- `src/bin/waymux.rs` & `src/main.rs`: Thin binary entrypoints invoking `acpd::entry()`.
- `src/api.rs`: Axum HTTP/JSON-RPC 2.0 handlers (`/rpc`, `/api/status`, `/health`, `/ready`), sequence-tracked 300ms idle debounce, and pane rehydration.
- `src/daemon.rs`: Background task supervisor, 30s stale pane reaper, and 5s proactive stall detector (20s zero-output + zero-CPU delta heuristic).
- `src/adapters.rs`: Output sinks for Tmux window/pane options, multi-pane urgency aggregation, Waybar status JSON, desktop notifications, and event-isolated sound playback.
- `src/auth.rs`: Session token generator (`0600` permissions at `$XDG_RUNTIME_DIR/waymux/token` mirrored to `$XDG_RUNTIME_DIR/acpd/token`).
- `src/rpc_client.rs`: Zero-dependency synchronous TCP HTTP client for CLI subcommands (`status`, `list`, `dismiss`, `health`, `agentState/list`, `/api/status`).
- `src/tmux_cmd.rs`: Synchronous Tmux command execution and atomic bracketed-paste buffer injection (`load-buffer` + `paste-buffer -p`).
- `src/theme.rs`: Dynamic Omarchy `colors.toml` parser, TrueColor ANSI escape generator, and ANSI stripper.
- `src/cockpit/`: Native Cockpit HUD engine:
  - `mod.rs`: `waymux cockpit` popup launcher, `@SEGMENT_BG`/`@FG`/`@ACCENT_COLOR` Waymaker color resolver, and `__ACTION__` dispatch loop.
  - `items.rs`: `waymux cockpit-items` (~14ms native `/proc/<pid>/stat` & `/proc/<pid>/comm` scanner, memoized Git/PR status, 12-column TSV emitter).
  - `preview.rs`: `waymux cockpit-preview` (contextual agent card, PR badge, review notes, terminal snapshot).
  - `diff.rs`: `waymux cockpit-diff` (diff header card, untracked files, flushed stdout into `delta --side-by-side` / `bat`).
  - `help.rs`: `waymux cockpit-help` (telemetry HUD and keybinding cheat-sheet).
  - `notes.rs`: `waymux review-notes` (`serde_json` parser/mutator for `<git-dir>/info/lines.json`).
  - `actions.rs`: `waymux agent-action`, `waymux window-create`, `waymux window-delete`.
- `src/presets/`: Embedded Waymaker preset (`cockpit-picker.toml`) materialized dynamically to `$XDG_RUNTIME_DIR/waymux/cockpit-picker.toml`.
- `src/triage.rs`: `waymux triage --popup` and `waymux triage --jump` ring-buffer alert navigation.
- `src/hook.rs`: `waymux hook {tmux|lazygit}` native sub-10ms AI agent lifecycle hooks.
- `src/inject.rs`: `waymux inject` bracketed-paste prompt injector with staleness guard.
- `src/wt/mod.rs`: `waymux wt` (and `argv[0] == "awt"`) native Git worktree orchestrator.
- `src/init.rs`: `waymux init {--tmux|--systemd|--hooks}` portable configuration generator.

---

## Development & Verification Commands

### Build & Install Locally

```bash
# Build release binaries and restart user systemd service
./scripts/update-daemon.sh
```

### Quality Gates (Required Before Every Commit)

All three checks must pass with **0 warnings and 0 failures**:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

> Current baseline: **30 unit tests passing** across `adapters`, `api`, `auth`, `cockpit`, `config`, `hook`, `inject`, `signals`, `theme`, and `wt`, with **0 Clippy warnings**.

---

## Engineering Conventions

1. **Preserve the Synchronous Fast-Path:** Never initialize Tokio or heavy global state before matching CLI subcommands in `src/lib.rs`. CLI subcommands (`hook`, `dismiss`, `cockpit-items`, `triage`) are invoked on interactive hot-paths and must start in `<1.5ms`.
2. **Use `anyhow::Result` & Safe Error Handling:** Avoid `.unwrap()` in production code paths; handle missing files, dead PIDs, or unreachable sockets gracefully.
3. **Flush Before Piping:** When printing Rust headers before spawning child processes that inherit stdout (e.g., `delta` or `bat` in `cockpit/diff.rs`), always call `std::io::stdout().flush()` first.
4. **Atomic Commits:** Keep commits focused, tested, and formatted.
