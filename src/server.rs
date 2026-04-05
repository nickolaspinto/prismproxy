use arc_swap::ArcSwap;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

use crate::acme;
use crate::config::Config;
use crate::error::ProxyError;
use crate::handler;
use crate::pool::ConnectionPool;
use crate::state::{build_state, AppState};

/// Start proxy without hot reload. Supports both plain HTTP and TLS (if config has [tls]
/// and cert files exist in cache_dir). Used in integration tests.
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
                    let state = app_state.load_full();
                    if let Some(ref tls_state) = state.tls {
                        let acceptor = tls_state.acceptor.clone();
                        drop(state);
                        serve_tls(stream, addr, acceptor, app_state, pool, start_time).await;
                    } else {
                        drop(state);
                        serve_plain(stream, addr, app_state, pool, start_time).await;
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

/// Start proxy with hot reload and optional TLS with ACME provisioning.
/// If [tls] is configured and no cert is cached, provisions via ACME before accepting.
/// Spawns a daily renewal_loop task when TLS is configured.
pub async fn run_with_listener_hot(
    listener: TcpListener,
    config_path: impl AsRef<Path>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProxyError> {
    let config_path = config_path.as_ref().to_path_buf();
    let config = Config::from_file(&config_path)?;
    config.validate()?;

    let pool = Arc::new(ConnectionPool::new(config.server.max_idle_connections));
    let has_tls_config = config.tls.is_some();
    let http_challenge_listen = config.server.http_challenge_listen.clone();

    let mut initial_state = build_state(config)?;

    // If TLS is configured but no cert cached yet, provision now (blocks startup)
    if has_tls_config && initial_state.tls.is_none() {
        let config = Config::from_file(&config_path)?;
        let tls_config = config.tls.as_ref().unwrap();
        let challenge_listen = http_challenge_listen.as_deref().ok_or_else(|| {
            ProxyError::Config(
                "http_challenge_listen required when [tls] is configured".to_string(),
            )
        })?;
        acme::provision(tls_config, challenge_listen).await?;
        // Reload state now that cert files are on disk
        let config2 = Config::from_file(&config_path)?;
        config2.validate()?;
        initial_state = build_state(config2)?;
    }

    let app_state = Arc::new(ArcSwap::from_pointee(initial_state));
    let start_time = Arc::new(Instant::now());

    let initial_mtime = tokio::fs::metadata(&config_path)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .unwrap_or(std::time::UNIX_EPOCH);

    // Config hot-reload task (mtime polling)
    {
        let app_state = app_state.clone();
        let config_path = config_path.clone();
        tokio::spawn(async move {
            reload_loop(config_path, app_state, initial_mtime).await;
        });
    }

    // Daily cert renewal task
    if has_tls_config {
        let app_state = app_state.clone();
        let config_path = config_path.clone();
        tokio::spawn(async move {
            acme::renewal_loop(config_path, app_state).await;
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
                    let state = app_state.load_full();
                    if let Some(ref tls_state) = state.tls {
                        let acceptor = tls_state.acceptor.clone();
                        drop(state);
                        serve_tls(stream, addr, acceptor, app_state, pool, start_time).await;
                    } else {
                        drop(state);
                        serve_plain(stream, addr, app_state, pool, start_time).await;
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

async fn serve_plain(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    start_time: Arc<Instant>,
) {
    let io = TokioIo::new(stream);
    let svc = service_fn(move |req| {
        let app_state = app_state.clone();
        let pool = pool.clone();
        let start_time = start_time.clone();
        async move { handler::handle(app_state, pool, addr, start_time, false, req).await }
    });
    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
        error!(%addr, "connection error: {e}");
    }
}

async fn serve_tls(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    acceptor: TlsAcceptor,
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    start_time: Arc<Instant>,
) {
    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            let io = TokioIo::new(tls_stream);
            let svc = service_fn(move |req| {
                let app_state = app_state.clone();
                let pool = pool.clone();
                let start_time = start_time.clone();
                async move { handler::handle(app_state, pool, addr, start_time, true, req).await }
            });
            if let Err(e) = auto::Builder::new(TokioExecutor::new())
                .serve_connection(io, svc)
                .await
            {
                error!(%addr, "tls connection error: {e}");
            }
        }
        Err(e) => error!(%addr, "tls handshake failed: {e}"),
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
