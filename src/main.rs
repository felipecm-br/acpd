mod adapters;
mod api;
mod auth;
mod config;
mod daemon;
mod health;
mod pid;
mod signals;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Fast-path CLI queries (zero-tracing, sub-millisecond return)
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("acpd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "acpd {}\nAutonomous Cockpit Protocol Daemon\n\n\
USAGE:\n    acpd [OPTIONS] [COMMAND]\n\n\
OPTIONS:\n    -c, --config <FILE>    Custom path to config.toml\n    \
-h, --help             Print help information\n    \
-V, --version          Print version information\n\n\
COMMANDS:\n    \
health                 Check health status of running daemon",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    if args.get(1).map(|s| s.as_str()) == Some("health") {
        match tokio::net::TcpStream::connect("127.0.0.1:4040").await {
            Ok(mut stream) => {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let req = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1:4040\r\nConnection: close\r\n\r\n";
                if stream.write_all(req).await.is_ok() {
                    let mut resp = Vec::new();
                    let _ = stream.read_to_end(&mut resp).await;
                    let resp_str = String::from_utf8_lossy(&resp);
                    if resp_str.contains("200 OK") {
                        if let Some(body) = resp_str.split("\r\n\r\n").nth(1) {
                            println!("{}", body.trim());
                        } else {
                            println!("healthy");
                        }
                        return Ok(());
                    }
                }
                eprintln!("unhealthy: unexpected response from acpd");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("acpd is not running or unreachable on port 4040: {}", e);
                std::process::exit(1);
            }
        }
    }

    // 1. Initialize logging (tracing)
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "acpd=info,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting ACP Daemon (acpd)...");

    // 2. Parse arguments and resolve config path
    let mut config_path = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--config" || arg == "-c" {
            config_path = iter.next().cloned();
        } else if let Some(stripped) = arg.strip_prefix("--config=") {
            config_path = Some(stripped.to_string());
        }
    }

    let config_path = match config_path {
        Some(path) => path,
        None => {
            let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
                format!("{}/.config", home)
            });
            let user_path = format!("{}/acpd/config.toml", xdg_config);
            let system_path = "/etc/acpd/config.toml";
            let dev_path = "config/default.toml";

            if std::path::Path::new(&user_path).is_file() {
                user_path
            } else if std::path::Path::new(system_path).is_file() {
                system_path.to_string()
            } else if std::path::Path::new(dev_path).is_file() {
                tracing::info!("System/User config not found, falling back to {}", dev_path);
                dev_path.to_string()
            } else {
                anyhow::bail!(
                    "No config file found. Provide one with --config <path>, or create {}",
                    user_path
                );
            }
        }
    };

    tracing::info!("Loading config from {}", config_path);
    let config = crate::config::Config::load(&config_path)?;

    // 3. Handle PID file if configured
    let _pid_file = if let Some(ref path) = config.pid_file {
        tracing::info!("Creating PID file at {}", path);
        Some(crate::pid::PidFile::create(path)?)
    } else {
        None
    };

    // 4. Run the daemon lifecycle
    crate::daemon::run(config).await?;

    Ok(())
}
