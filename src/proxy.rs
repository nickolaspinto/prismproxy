use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tracing::error;

use crate::error::ProxyError;

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
) -> Result<Response<Full<Bytes>>, ProxyError> {
    let (mut parts, body) = req.into_parts();
    let body_bytes = body.collect().await.map_err(ProxyError::Hyper)?.to_bytes();

    // Remove hop-by-hop headers
    for h in HOP_BY_HOP {
        parts.headers.remove(*h);
    }

    // Set Host to upstream
    if let Ok(val) = upstream_addr.parse() {
        parts.headers.insert(hyper::header::HOST, val);
    }

    let upstream_req = Request::from_parts(parts, Full::new(body_bytes));

    // Connect to upstream
    let stream = TcpStream::connect(upstream_addr)
        .await
        .map_err(|e| ProxyError::UpstreamConnect(format!("{upstream_addr}: {e}")))?;
    let io = TokioIo::new(stream);

    let (mut sender, conn): (SendRequest<Full<Bytes>>, _) =
        hyper::client::conn::http1::handshake(io)
            .await
            .map_err(ProxyError::Hyper)?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            error!("upstream connection error: {e}");
        }
    });

    let resp = sender.send_request(upstream_req).await.map_err(ProxyError::Hyper)?;

    let (resp_parts, resp_body) = resp.into_parts();
    let resp_bytes = resp_body.collect().await.map_err(ProxyError::Hyper)?.to_bytes();

    Ok(Response::from_parts(resp_parts, Full::new(resp_bytes)))
}
