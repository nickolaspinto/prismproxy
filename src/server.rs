use arc_swap::ArcSwap;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::error::ProxyError;
use crate::handler;
use crate::pool::ConnectionPool;
use crate::state::{build_state, AppState};

/// Start the proxy without hot reload. Used in integration tests.
pub async fn run_with_listener(
    listener: TcpListener,
    config: Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProxyError> {
    let pool = Arc::new(ConnectionPool::new(config.server.max_idle_connections));
    let app_state = Arc::new(ArcSwap::from_pointee(build_state(config)?));
    let start_time = Arc::new(Instant::now());
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result?;
                let app_state = app_state.clone();
                let pool = pool.clone();
                let start_time = start_time.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req| {
                        let app_state = app_state.clone();
                        let pool = pool.clone();
                        let start_time = start_time.clone();
                        async move { handler::handle(app_state, pool, addr, start_time, req).await }
                    });
                    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                        error!(%addr, "connection error: {e}");
                    }
                });
            }
            _ = &mut shutdown => {
                info!("shutting down");
                return Ok(());
            }
        }
    }
}

/// Start the proxy with hot reload. Used in production and reload tests.
/// Reads config from disk, polls mtime every 1s, atomically swaps state on change.
/// On any reload error: logs warn! and keeps previous state.
pub async fn run_with_listener_hot(
    listener: TcpListener,
    config_path: impl AsRef<Path>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProxyError> {
    let config_path = config_path.as_ref().to_path_buf();
    let config = Config::from_file(&config_path)?;
    config.validate()?;

    let pool = Arc::new(ConnectionPool::new(config.server.max_idle_connections));
    let app_state = Arc::new(ArcSwap::from_pointee(build_state(config)?));
    let start_time = Arc::new(Instant::now());

    let initial_mtime = tokio::fs::metadata(&config_path)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .unwrap_or(std::time::UNIX_EPOCH);

    {
        let app_state = app_state.clone();
        let config_path = config_path.clone();
        tokio::spawn(async move {
            reload_loop(config_path, app_state, initial_mtime).await;
        });
    }

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result?;
                let app_state = app_state.clone();
                let pool = pool.clone();
                let start_time = start_time.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req| {
                        let app_state = app_state.clone();
                        let pool = pool.clone();
                        let start_time = start_time.clone();
                        async move { handler::handle(app_state, pool, addr, start_time, req).await }
                    });
                    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                        error!(%addr, "connection error: {e}");
                    }
                });
            }
            _ = &mut shutdown => {
                info!("shutting down");
                return Ok(());
            }
        }
    }
}

async fn reload_loop(
    config_path: PathBuf,
    app_state: Arc<ArcSwap<AppState>>,
    mut last_mtime: std::time::SystemTime,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let mtime = match tokio::fs::metadata(&config_path).await {
            Ok(m) => match m.modified() {
                Ok(t) => t,
                Err(e) => {
                    warn!("config mtime unavailable: {e}");
                    continue;
                }
            },
            Err(e) => {
                warn!("config stat failed: {e}");
                continue;
            }
        };
        if mtime <= last_mtime {
            continue;
        }
        match try_reload(&config_path) {
            Ok(new_state) => {
                let route_count = new_state.routes.len();
                app_state.store(Arc::new(new_state));
                last_mtime = mtime;
                info!(routes = route_count, "config reloaded");
            }
            Err(e) => warn!("reload failed, keeping previous config: {e}"),
        }
    }
}

fn try_reload(config_path: &Path) -> Result<AppState, ProxyError> {
    let config = Config::from_file(config_path)?;
    config.validate()?;
    build_state(config)
}
