use crate::adapters::{SoundAdapter, TmuxAdapter, WaybarAdapter};
use crate::api::api_router;
use crate::config::Config;
use crate::health::health_router;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;

pub async fn run(config: Config) -> anyhow::Result<()> {
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    let signal_handle = tokio::spawn(crate::signals::signal_listener(shutdown_tx));

    tracing::info!("Daemon binding to {}:{}", config.listen_addr, config.port);
    let listener = TcpListener::bind(format!("{}:{}", config.listen_addr, config.port)).await?;

    let start_time = Arc::new(Instant::now());

    let active_spinner_name = config
        .theme
        .as_ref()
        .map(|t| t.active_spinner.clone())
        .unwrap_or_else(|| "arc".to_string());

    let active_spinner = config
        .spinners
        .as_ref()
        .and_then(|spinners| spinners.get(&active_spinner_name).cloned());
    tokio::spawn(async move {
        let _ = tokio::process::Command::new("tmux")
            .args(["set", "-g", "@ai_agent_spinner", &active_spinner_name])
            .output()
            .await;
    });

    let adapters = crate::api::ApiState::new(
        vec![
            Box::new(TmuxAdapter::new(config.theme.clone(), active_spinner)),
            Box::new(WaybarAdapter::new(config.theme.clone())),
            Box::new(SoundAdapter::new(config.sound.clone())),
        ],
        config.idle_debounce_ms.unwrap_or(650),
    );

    adapters.rehydrate_from_tmux().await;

    let cleanup_state = adapters.clone();
    let mut cleanup_shutdown_rx = shutdown_rx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cleanup_shutdown_rx.changed() => break,
                _ = interval.tick() => {
                    cleanup_state.clean_stale_panes().await;
                }
            }
        }
    });

    let token = Arc::new(crate::auth::generate_and_save_token()?);

    let app = axum::Router::new()
        .merge(health_router(start_time))
        .merge(api_router(adapters, token));

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await
    });

    sd_notify::notify(false, &[sd_notify::NotifyState::Ready])?;

    signal_handle.await?;

    match tokio::time::timeout(
        std::time::Duration::from_secs(config.shutdown_timeout_secs.unwrap_or(30)),
        server_handle,
    )
    .await
    {
        Ok(Ok(Ok(()))) => tracing::info!("axum server cleanly shutdown"),
        Ok(Ok(Err(e))) => tracing::error!("axum server error: {}", e),
        Ok(Err(e)) => tracing::error!("server task panicked: {}", e),
        Err(_) => tracing::error!("server shutdown timed out"),
    }

    sd_notify::notify(false, &[sd_notify::NotifyState::Stopping])?;

    Ok(())
}
