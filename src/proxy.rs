use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};

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
) -> Result<Response<Full<Bytes>>, ProxyError> {
    let (mut parts, body) = req.into_parts();
    let body_bytes = body.collect().await.map_err(ProxyError::Hyper)?.to_bytes();

    for h in HOP_BY_HOP {
        parts.headers.remove(*h);
    }
    if let Ok(val) = upstream_addr.parse() {
        parts.headers.insert(hyper::header::HOST, val);
    }

    let upstream_req = Request::from_parts(parts, Full::new(body_bytes));

    let mut sender = pool.acquire(upstream_addr).await?;
    let resp = sender
        .send_request(upstream_req)
        .await
        .map_err(ProxyError::Hyper)?;

    // Collect full response body before releasing connection
    let (resp_parts, resp_body) = resp.into_parts();
    let resp_bytes = resp_body
        .collect()
        .await
        .map_err(ProxyError::Hyper)?
        .to_bytes();

    // Return connection to pool
    pool.release(upstream_addr, sender).await;

    Ok(Response::from_parts(resp_parts, Full::new(resp_bytes)))
}
