use arc_swap::ArcSwap;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

use crate::error::ProxyError;
use crate::pool::ConnectionPool;
use crate::proxy;
use crate::state::AppState;

pub async fn handle(
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    start_time: Arc<std::time::Instant>,
    is_tls: bool,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    match route(app_state, pool, client_addr, start_time, is_tls, req).await {
        Ok(resp) => {
            info!(%method, %path, status = %resp.status(), "response");
            Ok(resp)
        }
        Err(e) => {
            error!(%method, %path, error = %e, "failed");
            Ok(error_response(e))
        }
    }
}

async fn route(
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    start_time: Arc<std::time::Instant>,
    is_tls: bool,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    let path = req.uri().path().to_string();

    if path == "/health" {
        let state = app_state.load_full();
        let routes = state.routes.len();
        let tls = state.tls.is_some();
        return Ok(health_response(&start_time, routes, tls));
    }

    let state = app_state.load_full();
    let timeout = std::time::Duration::from_millis(state.timeout_ms);

    let route_state = state
        .routes
        .iter()
        .find(|rs| path.starts_with(&rs.route.path_prefix))
        .ok_or_else(|| ProxyError::NoRoute(path.clone()))?;

    if route_state
        .runtime
        .run_on_request(req.method().as_str(), &path)?
    {
        return Ok(blocked_response());
    }

    proxy::forward(
        req,
        &route_state.route.upstream,
        &pool,
        timeout,
        client_addr,
        is_tls,
    )
    .await
}

fn health_response(
    start_time: &std::time::Instant,
    routes: usize,
    tls: bool,
) -> Response<Full<Bytes>> {
    let uptime = start_time.elapsed().as_secs_f64();
    let body = format!(
        r#"{{"status":"ok","version":"{}","uptime_secs":{:.1},"routes":{},"tls":{}}}"#,
        env!("CARGO_PKG_VERSION"),
        uptime,
        routes,
        tls,
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn blocked_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from("403 Forbidden\n")))
        .unwrap()
}

fn error_response(err: ProxyError) -> Response<Full<Bytes>> {
    let (status, body) = match &err {
        ProxyError::NoRoute(_) => (StatusCode::NOT_FOUND, err.to_string()),
        ProxyError::UpstreamConnect(_) => (StatusCode::BAD_GATEWAY, err.to_string()),
        ProxyError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, err.to_string()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into()),
    };
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}
