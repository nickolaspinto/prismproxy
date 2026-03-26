use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

use crate::config::Config;
use crate::error::ProxyError;
use crate::pool::ConnectionPool;
use crate::proxy;

pub async fn handle(
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    match route(config, pool, client_addr, req).await {
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
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    let path = req.uri().path().to_string();

    if path == "/health" {
        return Ok(health_response());
    }

    let route = config
        .routes
        .iter()
        .find(|r| path.starts_with(&r.path_prefix))
        .ok_or_else(|| ProxyError::NoRoute(path))?;

    let timeout = std::time::Duration::from_millis(config.server.timeout_ms);
    proxy::forward(req, &route.upstream, &pool, timeout, client_addr).await
}

fn health_response() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(r#"{"status":"ok"}"#)))
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
