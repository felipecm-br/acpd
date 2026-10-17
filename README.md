# acpd — Agent Client Protocol Daemon

A headless Rust daemon that bridges Agent Client Protocol (ACP) events to desktop UI surfaces: Tmux pane state, Waybar status files, and desktop notifications.

## What it does

AI coding agents (Claude Code, Goose, custom hooks) currently manage their own Tmux spinner logic, window title cleanup, and UI state. `acpd` centralizes that into a single long-running daemon so clients become thin HTTP callers:

```text
AI agent  ──HTTP──▶  acpd  ──▶  Tmux / Waybar / Notifications
```

## Features

- **ACP JSON-RPC endpoint** at `POST /rpc` (`agentState/update`, `agentState/list`)
- **Tmux orchestration & inspection RPC methods** (`tmux.capture_pane`, `tmux.list_panes`, `tmux.send_keys`, `tmux.new_window`, `tmux.split_pane`, `tmux.list_windows`, `tmux.list_sessions`)
- **Local Session Token Auth** with Bearer token header / path-based authentication (`0600` permissions on session token)
- **Tmux state color synchronization** with dynamic pane border styling based on state (`working`, `idle`, `waiting`, `error`, `closed`)
- **Debounced idle updates & race condition protection** via sequence tracking per pane
- **Periodic stale pane cleanup task** (prunes dead panes every 30s)
- **Simple REST endpoint** at `POST /api/status` for custom bash/JS hooks
- **Health and readiness probes** at `GET /health` and `GET /ready`
- **systemd-notify integration** for `Type=notify` services
- **Graceful shutdown** on `SIGTERM`/`SIGINT`, config reload hook on `SIGHUP`
- **Unified `waymux` Orchestrator CLI (`v0.2.0+`)**: Built alongside `acpd` from the same crate, providing sub-2ms synchronous CLI commands for the Cockpit HUD (`waymux cockpit`), 12-column TSV item generation (`cockpit-items`), live previews (`cockpit-preview`, `cockpit-diff`, `cockpit-help`), review note dispatch (`review-notes`), alert triage (`triage --popup|--jump`), zero-Node agent hooks (`hook tmux|lazygit`), bracketed-paste injection (`inject`), Git worktree orchestration (`wt`), and portable setup (`init`).

## Quick start

Requires Rust 1.95+.

```bash
# Build
cargo build --release

# Run with the reference config
cargo run --release -- --config config/default.toml
```

The daemon listens on `127.0.0.1:4040` by default.

## Configuration

Create a TOML config file:

```toml
listen_addr = "127.0.0.1"
port = 4040
pid_file = "/run/acpd/acpd.pid"
shutdown_timeout_secs = 30
log_level = "info"
```

`acpd` reads the config path from the second CLI argument, defaulting to `/etc/acpd/config.toml`.

## API & Authentication

Protected endpoints support local session token authentication via HTTP Bearer token: `-H "Authorization: Bearer <token>"`.

### Health

```bash
curl http://127.0.0.1:4040/health
curl http://127.0.0.1:4040/ready
```

### REST status update

```bash
curl -X POST http://127.0.0.1:4040/api/status \
  -H "Authorization: Bearer $(cat ~/.cache/acpd/token)" \
  -H "Content-Type: application/json" \
  -d '{"agent":"antigravity","pane_id":"%2","state":"working","message":"Needs file system access"}'
```

Valid states: `working`, `idle`, `waiting`, `error`, `closed`.

### ACP JSON-RPC endpoints

**1. Update Agent State (`agentState/update`):**
```bash
curl -X POST http://127.0.0.1:4040/rpc \
  -H "Authorization: Bearer $(cat ~/.cache/acpd/token)" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"agentState/update","params":{"state":"working","pane_id":"%2"},"id":1}'
```

**2. List Active States (`agentState/list`):**
```bash
curl -X POST http://127.0.0.1:4040/rpc \
  -H "Authorization: Bearer $(cat ~/.cache/acpd/token)" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"agentState/list","params":{},"id":1}'
```

### Tmux Orchestration & Inspection (RPC)

`acpd` can dynamically orchestrate Tmux windows and panes via JSON-RPC 2.0 calls:

- `tmux.new_window` — Create a new Tmux window (`name`, `command`)
- `tmux.split_pane` — Split current pane (`vertical`, `command`)
- `tmux.list_panes` — List active panes
- `tmux.capture_pane` — Capture pane scrollback text
- `tmux.send_keys` — Send keystrokes to a target pane
- `tmux.list_windows` — List active windows (includes `window_name`, `session_name`)
- `tmux.list_sessions` — List active sessions

## Deploy with systemd (User Level)

Instead of running globally as root, `acpd` is designed to run as a user-level background service, allowing easy development and updates without `sudo`.

```bash
# 1. Setup default configuration
mkdir -p ~/.config/acpd
cp config/default.toml ~/.config/acpd/config.toml

# 2. Link the systemd service to your user configuration
mkdir -p ~/.config/systemd/user
ln -sf $(pwd)/systemd/acpd.service ~/.config/systemd/user/acpd.service

# 3. Reload and enable the daemon on boot
systemctl --user daemon-reload
systemctl --user enable --now acpd
```

View logs:
```bash
journalctl --user -u acpd -f
```

**Developer Workflow:** 
If you modify the source code, run `./scripts/update-daemon.sh`. It will automatically rebuild the release binary and restart the service in the background.

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Format and lint
cargo fmt
cargo clippy -- -D warnings
```

See `AGENTS.md` for detailed agent-focused instructions, `CONTRIBUTING.md` for contribution guidelines and commit conventions, and `ARCHITECTURE.md` for the original design and migration plan.
