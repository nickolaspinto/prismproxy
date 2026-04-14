use arc_swap::ArcSwap;
use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use std::convert::Infallible;
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
    let metrics = crate::metrics::Metrics::new();

    // Spawn upstream health-check loops
    {
        let state = app_state.load_full();
        for rs in &state.routes {
            tokio::spawn(crate::health_check::upstream_health_loop(
                rs.route.upstream.clone(),
                rs.healthy.clone(),
                Duration::from_secs(10),
                3,
                2,
            ));
        }
    }

    run_accept_loop(listener, app_state, pool, metrics, shutdown).await
}

/// Lower-level entry: accepts a pre-built app state. Used by tests that need to
/// pre-configure route health flags without waiting for health-check loops to fire.
pub async fn run_with_app_state(
    app_state: Arc<ArcSwap<AppState>>,
    listener: TcpListener,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProxyError> {
    let pool = Arc::new(ConnectionPool::new(8));
    let metrics = crate::metrics::Metrics::new();
    run_accept_loop(listener, app_state, pool, metrics, shutdown).await
}

async fn run_accept_loop(
    listener: TcpListener,
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    metrics: Arc<crate::metrics::Metrics>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProxyError> {
    let start_time = Arc::new(Instant::now());
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result?;
                let app_state = app_state.clone();
                let pool = pool.clone();
                let start_time = start_time.clone();
                let metrics = metrics.clone();
                tokio::spawn(async move {
                    let state = app_state.load_full();
                    if let Some(ref tls_state) = state.tls {
                        let acceptor = tls_state.acceptor.clone();
                        drop(state);
                        serve_tls(stream, addr, acceptor, app_state, pool, start_time, metrics).await;
                    } else {
                        drop(state);
                        serve_plain(stream, addr, app_state, pool, start_time, metrics).await;
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
    let metrics = crate::metrics::Metrics::new();

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

    // Spawn upstream health-check loops for initial routes
    {
        let state = app_state.load_full();
        for rs in &state.routes {
            tokio::spawn(crate::health_check::upstream_health_loop(
                rs.route.upstream.clone(),
                rs.healthy.clone(),
                Duration::from_secs(10),
                3,
                2,
            ));
        }
    }

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

    // HTTP → HTTPS redirect server + daily cert renewal
    let challenge_store = if has_tls_config {
        let bind = http_challenge_listen.clone().unwrap_or_default();
        let store = Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));

        // Derive HTTPS port from config listen address
        let https_port: u16 = {
            let config = Config::from_file(&config_path).unwrap_or_else(|_| {
                crate::config::Config::parse("[server]\nlisten=\"0.0.0.0:443\"").unwrap()
            });
            config
                .server
                .listen
                .parse::<SocketAddr>()
                .map(|a| a.port())
                .unwrap_or(443)
        };

        if !bind.is_empty() {
            let store_clone = store.clone();
            tokio::spawn(run_http_redirect_server(bind, https_port, store_clone));
        }

        let store_for_renewal = store.clone();
        let app_state_r = app_state.clone();
        let config_path_r = config_path.clone();
        tokio::spawn(async move {
            acme::renewal_loop(config_path_r, app_state_r, Some(store_for_renewal)).await;
        });

        Some(store)
    } else {
        None
    };
    let _ = challenge_store; // suppress unused warning if no TLS

    run_accept_loop(listener, app_state, pool, metrics, shutdown).await
}

async fn serve_plain(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    start_time: Arc<Instant>,
    metrics: Arc<crate::metrics::Metrics>,
) {
    let io = TokioIo::new(stream);
    let svc = service_fn(move |req| {
        let app_state = app_state.clone();
        let pool = pool.clone();
        let start_time = start_time.clone();
        let metrics = metrics.clone();
        async move { handler::handle(app_state, pool, addr, start_time, metrics, false, req).await }
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
    metrics: Arc<crate::metrics::Metrics>,
) {
    match acceptor.accept(stream).await {
        Ok(tls_stream) => {
            let io = TokioIo::new(tls_stream);
            let svc = service_fn(move |req| {
                let app_state = app_state.clone();
                let pool = pool.clone();
                let start_time = start_time.clone();
                let metrics = metrics.clone();
                async move { handler::handle(app_state, pool, addr, start_time, metrics, true, req).await }
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

/// Persistent HTTP server on `bind_addr` that:
/// - Serves ACME HTTP-01 challenges from `challenges` (written by the renewal loop)
/// - Returns 301 → HTTPS for all other requests
pub async fn run_http_redirect_server(
    bind_addr: String,
    https_port: u16,
    challenges: acme::ChallengeStore,
) {
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(addr = %bind_addr, "HTTP redirect server failed to bind: {e}");
            return;
        }
    };
    info!(addr = %bind_addr, https_port, "HTTP redirect server started");
    run_http_redirect_server_with_listener(listener, https_port, challenges).await;
}

/// Like [run_http_redirect_server] but accepts a pre-bound listener. Exported for testing.
pub async fn run_http_redirect_server_with_listener(
    listener: TcpListener,
    https_port: u16,
    challenges: acme::ChallengeStore,
) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("redirect server accept error: {e}");
                continue;
            }
        };
        let challenges = challenges.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let challenges = challenges.clone();
                async move {
                    let path = req.uri().path().to_string();
                    const PREFIX: &str = "/.well-known/acme-challenge/";
                    if let Some(token) = path.strip_prefix(PREFIX) {
                        let store = challenges.read().unwrap();
                        if let Some(key_auth) = store.get(token) {
                            return Ok::<_, Infallible>(
                                hyper::Response::builder()
                                    .status(200)
                                    .header("content-type", "text/plain")
                                    .body(Full::new(Bytes::from(key_auth.clone())))
                                    .unwrap(),
                            );
                        }
                    }
                    // Redirect to HTTPS
                    let host = req
                        .headers()
                        .get(hyper::header::HOST)
                        .and_then(|h| h.to_str().ok())
                        .map(|h| h.split(':').next().unwrap_or(h).to_string())
                        .unwrap_or_default();
                    let location = if https_port == 443 {
                        format!("https://{host}{}", req.uri())
                    } else {
                        format!("https://{host}:{https_port}{}", req.uri())
                    };
                    Ok(hyper::Response::builder()
                        .status(301)
                        .header("location", &location)
                        .body(Full::new(Bytes::new()))
                        .unwrap())
                }
            });
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        });
    }
}
