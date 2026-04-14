use bytes::Bytes;
use http_body_util::Empty;
use hyper::Request;
use hyper_util::rt::TokioIo;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tracing::{info, warn};

/// Polls `upstream_addr` with `GET /health` every `interval`.
/// Marks `healthy` false after `fail_threshold` consecutive failures.
/// Marks `healthy` true after `recover_threshold` consecutive successes.
pub async fn upstream_health_loop(
    upstream_addr: String,
    healthy: Arc<AtomicBool>,
    interval: Duration,
    fail_threshold: u32,
    recover_threshold: u32,
) {
    let mut consecutive_failures: u32 = 0;
    let mut consecutive_successes: u32 = 0;
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // skip immediate first tick

    loop {
        ticker.tick().await;

        let ok = ping(&upstream_addr).await;

        if ok {
            consecutive_failures = 0;
            consecutive_successes += 1;
            if !healthy.load(Relaxed) && consecutive_successes >= recover_threshold {
                healthy.store(true, Relaxed);
                info!(upstream = %upstream_addr, "upstream recovered");
            }
        } else {
            consecutive_successes = 0;
            consecutive_failures += 1;
            if healthy.load(Relaxed) && consecutive_failures >= fail_threshold {
                healthy.store(false, Relaxed);
                warn!(
                    upstream = %upstream_addr,
                    consecutive_failures,
                    "upstream marked unhealthy"
                );
            }
        }
    }
}

async fn ping(addr: &str) -> bool {
    let stream = match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await
    {
        Ok(Ok(s)) => s,
        _ => return false,
    };

    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        match hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(io).await {
            Ok(v) => v,
            Err(_) => return false,
        };
    tokio::spawn(conn);

    let req = match Request::builder()
        .method("GET")
        .uri("/health")
        .header("host", addr)
        .body(Empty::new())
    {
        Ok(r) => r,
        Err(_) => return false,
    };

    match tokio::time::timeout(Duration::from_secs(5), sender.send_request(req)).await {
        Ok(Ok(resp)) => resp.status().is_success(),
        _ => false,
    }
}
