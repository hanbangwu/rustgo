use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use igo_server::{app_with_shutdown, ServerConfig, Shutdown};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "igo_server=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let defaults = ServerConfig::default();
    let expiry_days: u64 = env_or("EXPIRY_DAYS", 1);
    let config = ServerConfig {
        max_idle: std::time::Duration::from_secs(expiry_days * 24 * 3600),
        sweep_interval: defaults.sweep_interval,
        max_rooms: env_or("MAX_ROOMS", defaults.max_rooms),
        database: std::env::var_os("DATABASE").map(PathBuf::from),
        static_dir: std::env::var("STATIC_DIR")
            .map(PathBuf::from)
            .unwrap_or(defaults.static_dir),
        allowed_origins: std::env::var("ALLOWED_ORIGINS")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|o| !o.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
    };
    let port: u16 = env_or("PORT", 3030);

    tracing::info!(?config, port, "starting igo");

    let shutdown = Shutdown::new();
    let router = app_with_shutdown(config, shutdown.clone())?;
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
    axum::serve(listener, router)
        .with_graceful_shutdown({
            let shutdown = shutdown.clone();
            async move {
                terminated().await;
                tracing::info!("shutting down");
                shutdown.trigger();
            }
        })
        .await?;
    // Hosts such as Render allow 30 seconds between SIGTERM and SIGKILL.
    shutdown.drained(Duration::from_secs(10)).await;
    Ok(())
}

async fn terminated() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
}
