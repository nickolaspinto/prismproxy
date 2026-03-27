use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};
use std::net::SocketAddr;
use std::time::Duration;

use crate::error::ProxyError;
use crate::pool::ConnectionPool;

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

pub async fn forward(
    req: Request<hyper::body::Incoming>,
    upstream_addr: &str,
    pool: &ConnectionPool,
    timeout: Duration,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    match tokio::time::timeout(
        timeout,
        forward_inner(req, upstream_addr, pool, client_addr),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ProxyError::Timeout(format!(
            "{upstream_addr}: exceeded {timeout:?}"
        ))),
    }
}

async fn forward_inner(
    req: Request<hyper::body::Incoming>,
    upstream_addr: &str,
    pool: &ConnectionPool,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    let (mut parts, body) = req.into_parts();
    let body_bytes = body.collect().await.map_err(ProxyError::Hyper)?.to_bytes();

    for h in HOP_BY_HOP {
        parts.headers.remove(*h);
    }
    if let Ok(val) = upstream_addr.parse() {
        parts.headers.insert(hyper::header::HOST, val);
    }
    parts.headers.insert(
        "x-forwarded-for",
        client_addr.ip().to_string().parse().unwrap(),
    );
    parts
        .headers
        .insert("x-forwarded-proto", "http".parse().unwrap());

    let build_request = |parts: &http::request::Parts, body: &Bytes| {
        let mut builder = Request::builder()
            .method(parts.method.clone())
            .uri(parts.uri.clone());
        for (name, value) in &parts.headers {
            builder = builder.header(name, value);
        }
        builder.body(Full::new(body.clone())).unwrap()
    };

    let mut sender = pool.acquire(upstream_addr).await?;
    let upstream_req = build_request(&parts, &body_bytes);
    let resp = match sender.send_request(upstream_req).await {
        Ok(resp) => {
            pool.release(upstream_addr, sender).await;
            resp
        }
        Err(e) => {
            // Pooled connection may be stale — retry with fresh connection
            tracing::warn!(upstream = upstream_addr, error = %e, "pooled connection stale, retrying");
            let upstream_req = build_request(&parts, &body_bytes);
            let mut fresh = pool.connect_fresh(upstream_addr).await?;
            let resp = fresh
                .send_request(upstream_req)
                .await
                .map_err(ProxyError::Hyper)?;
            pool.release(upstream_addr, fresh).await;
            resp
        }
    };

    let (resp_parts, resp_body) = resp.into_parts();
    let resp_bytes = resp_body
        .collect()
        .await
        .map_err(ProxyError::Hyper)?
        .to_bytes();

    Ok(Response::from_parts(resp_parts, Full::new(resp_bytes)))
}
