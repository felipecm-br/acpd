pub mod adapters;
pub mod api;
pub mod auth;
pub mod cockpit;
pub mod config;
pub mod daemon;
pub mod health;
pub mod hook;
pub mod init;
pub mod inject;
pub mod pid;
pub mod presets;
pub mod rpc_client;
pub mod signals;
pub mod theme;
pub mod tmux_cmd;
pub mod triage;
pub mod wt;

use std::path::Path;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn print_help(bin_name: &str) {
    println!(
        "{} {}\nWaymux — Autonomous Terminal AI Cockpit & Orchestrator Daemon\n\n\
USAGE:\n    {} [OPTIONS] [COMMAND]\n\n\
OPTIONS:\n    \
-c, --config <FILE>    Custom path to config.toml (daemon mode)\n    \
-h, --help             Print help information\n    \
-V, --version          Print version information\n\n\
COMMANDS:\n    \
daemon                 Start the async JSON-RPC & HTTP state daemon (default)\n    \
health                 Check health status of running daemon\n    \
status                 Print daemon health and active AI agent states\n    \
list                   List active AI agent pane states (TSV)\n    \
dismiss                Dismiss attention/question/permission badge for a pane\n    \
cockpit                Launch the interactive Fleet & Agent Cockpit HUD popup\n    \
cockpit-items          Emit prioritized 12-column TSV rows for Cockpit HUD\n    \
cockpit-preview        Render contextual agent card and terminal snapshot\n    \
cockpit-diff           Render side-by-side Git diff and review notes\n    \
cockpit-help           Render telemetry HUD and keybindings cheat-sheet\n    \
agent-action           Execute guarded agent confirmation/prompt action\n    \
review-notes           Count, list, or dispatch Lazygitrs AI review notes\n    \
window-create          Create Tmux window preserving row context\n    \
window-delete          Delete Tmux window or linked worktree session\n    \
triage                 Cycle AI agent alerts (--popup or --jump)\n    \
hook                   Zero-latency AI agent lifecycle hook (tmux | lazygit)\n    \
inject                 Bracketed-paste prompt injector for Lazygitrs\n    \
wt                     Git worktree & multi-agent session orchestrator (awt)\n    \
init                   Portable zero-config bootstrapper (--tmux | --systemd | --hooks)",
        bin_name,
        env!("CARGO_PKG_VERSION"),
        bin_name
    );
}

pub fn entry() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let bin_name = args
        .first()
        .and_then(|s| Path::new(s).file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "waymux".to_string());

    if bin_name == "awt" {
        let code = wt::run_cli(&args[1..]);
        if code != 0 {
            std::process::exit(code);
        }
        return Ok(());
    }

    let first_arg = args.get(1).map(|s| s.as_str()).unwrap_or("");

    // Fast-path synchronous subcommands (zero Tokio startup, <1.5ms execution)
    let exit_code = match first_arg {
        "--version" | "-V" => {
            println!("{} {}", bin_name, env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        "--help" | "-h" | "help" => {
            print_help(&bin_name);
            return Ok(());
        }
        "health" => Some(rpc_client::check_health_cli()),
        "status" => Some(rpc_client::status_cli()),
        "list" => Some(rpc_client::list_agents_cli()),
        "dismiss" => Some(rpc_client::dismiss_pane_cli(&args[2..])),
        "cockpit" => Some(cockpit::run_cockpit(&args[2..])),
        "cockpit-items" => Some(cockpit::items::run_cli(&args[2..])),
        "cockpit-preview" => Some(cockpit::preview::run_cli(&args[2..])),
        "cockpit-diff" => Some(cockpit::diff::run_cli(&args[2..])),
        "cockpit-help" => Some(cockpit::help::run_cli()),
        "agent-action" => Some(cockpit::actions::run_agent_action(&args[2..])),
        "review-notes" => Some(cockpit::notes::run_cli(&args[2..])),
        "window-create" => Some(cockpit::actions::run_window_create()),
        "window-delete" => Some(cockpit::actions::run_window_delete()),
        "triage" => Some(triage::run_cli(&args[2..])),
        "hook" => Some(hook::run_cli(&args[2..])),
        "inject" => Some(inject::run_cli(&args[2..])),
        "wt" | "awt" => Some(wt::run_cli(&args[2..])),
        "init" => Some(init::run_cli(&args[2..])),
        _ => None,
    };

    if let Some(code) = exit_code {
        if code != 0 {
            std::process::exit(code);
        }
        return Ok(());
    }

    // Daemon Mode: initialize Tokio multi-threaded runtime and tracing
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "acpd=info,axum=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();

        tracing::info!("Starting Waymux Daemon...");

        let mut config_path = None;
        let mut iter = args.iter().skip(1);
        while let Some(arg) = iter.next() {
            if arg == "daemon" {
                continue;
            } else if arg == "--config" || arg == "-c" {
                config_path = iter.next().cloned();
            } else if let Some(stripped) = arg.strip_prefix("--config=") {
                config_path = Some(stripped.to_string());
            }
        }

        let config = match config_path {
            Some(path) => {
                tracing::info!("Loading config from {}", path);
                crate::config::Config::load(&path)?
            }
            None => {
                let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                    format!("{}/.config", home)
                });
                let candidates = [
                    format!("{}/waymux/config.toml", xdg_config),
                    format!("{}/acpd/config.toml", xdg_config),
                    "/etc/waymux/config.toml".to_string(),
                    "/etc/acpd/config.toml".to_string(),
                    "config/default.toml".to_string(),
                ];

                if let Some(found) = candidates.iter().find(|p| Path::new(p).is_file()) {
                    tracing::info!("Loading config from {}", found);
                    crate::config::Config::load(found)?
                } else {
                    tracing::info!("No external config file found, using built-in defaults");
                    crate::config::Config::default()
                }
            }
        };

        let _pid_file = if let Some(ref path) = config.pid_file {
            tracing::info!("Creating PID file at {}", path);
            Some(crate::pid::PidFile::create(path)?)
        } else {
            None
        };

        crate::daemon::run(config).await?;
        Ok(())
    })
}
