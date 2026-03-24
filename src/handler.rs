use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use std::convert::Infallible;

pub async fn handle(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let _ = req;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::from("prismproxy")))
        .unwrap())
}
