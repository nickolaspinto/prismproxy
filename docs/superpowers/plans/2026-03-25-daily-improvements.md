# Daily Improvements: X-Forwarded Headers + Enhanced Health Check

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add standard proxy forwarding headers and an enhanced health check with version/uptime info.

**Architecture:** Add X-Forwarded-For/Host/Proto headers in proxy.rs before forwarding. Track server start time and expose it in the /health endpoint via handler.rs + server.rs.

**Tech Stack:** Rust, hyper 1.x, existing prismproxy codebase

---

## Prerequisites

- prismproxy M1 hardening complete (25 tests passing)
- Working directory: `/Users/nickolaspinto/prismproxy`

---

### Task 1: X-Forwarded Headers

**Files:**
- Modify: `src/proxy.rs`
- Modify: `tests/proxy_pass.rs`

- [ ] **Step 1: Write failing test — append to `tests/proxy_pass.rs`**

```rust
#[tokio::test]
async fn adds_x_forwarded_headers() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    assert_eq!(resp.status(), 200);
    // EchoUpstream echoes back the request — but it only returns method/path/body.
    // We need to verify headers were sent to upstream.
    // For now, just verify the proxy doesn't break when adding headers.
    // A proper test needs an upstream that echoes headers.
}
```

Actually — EchoUpstream only echoes method/path/body, not headers. We need to extend it or add a simpler test. Instead, we'll add a HeaderEchoUpstream OR just verify the proxy still works (headers are added transparently). Let's keep it simple: add the headers in proxy.rs, and add a test with a header-echoing upstream.

- [ ] **Step 1: Add `HeaderEchoUpstream` to `tests/common/mod.rs`** — append after EchoUpstream

An upstream that echoes request headers as JSON:

```rust
pub struct HeaderEchoUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl HeaderEchoUpstream {
    pub async fn start() -> Self {
        use http_body_util::BodyExt;
        use hyper::body::Incoming;
        use hyper::Request;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            http1::Builder::new()
                                .serve_connection(io, service_fn(|req: Request<Incoming>| async move {
                                    let mut headers_json = String::from("{");
                                    for (name, value) in req.headers() {
                                        if headers_json.len() > 1 {
                                            headers_json.push(',');
                                        }
                                        headers_json.push_str(&format!(
                                            "\"{}\":\"{}\"",
                                            name.as_str(),
                                            value.to_str().unwrap_or("")
                                        ));
                                    }
                                    headers_json.push('}');

                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .header("content-type", "application/json")
                                            .body(Full::new(Bytes::from(headers_json)))
                                            .unwrap(),
                                    )
                                }))
                                .await
                                .ok();
                        });
                    }
                    _ = &mut rx => break,
                }
            }
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }
}
```

- [ ] **Step 2: Write failing test — append to `tests/proxy_pass.rs`**

Add `HeaderEchoUpstream` to the import line.

```rust
#[tokio::test]
async fn adds_x_forwarded_for_header() {
    let upstream = HeaderEchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    let headers: serde_json::Value = resp.json().await.unwrap();
    assert!(headers["x-forwarded-for"].as_str().is_some());
    assert!(headers["x-forwarded-proto"].as_str().unwrap().contains("http"));
}
```

- [ ] **Step 3: Run** — `cargo test adds_x_forwarded` — expect FAIL

- [ ] **Step 4: Update `src/proxy.rs`** — add X-Forwarded headers in `forward_inner`, after hop-by-hop removal and Host setting.

Add these lines after the Host header insertion (after line that sets HOST):

```rust
    // Add X-Forwarded headers
    if !parts.headers.contains_key("x-forwarded-for") {
        // We don't have the client IP here, so we set it from the Host
        // In a real setup, the server would pass client_addr through
    }
    parts
        .headers
        .insert("x-forwarded-proto", "http".parse().unwrap());
    if let Some(host) = parts.headers.get(hyper::header::HOST) {
        parts
            .headers
            .insert("x-forwarded-host", host.clone());
    }
```

Actually, to properly set X-Forwarded-For we need the client address. The current `forward` function doesn't receive it. We need to thread it through from server.rs → handler.rs → proxy.rs.

Better approach: pass `client_addr: SocketAddr` through the chain.

Update `src/handler.rs` handle signature to accept `client_addr: SocketAddr`:

```rust
pub async fn handle(
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
```

Update route to pass it:
```rust
    match route(config, pool, client_addr, req).await {
```

Update route signature and forward call:
```rust
async fn route(
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    ...
    proxy::forward(req, &route.upstream, &pool, timeout, client_addr).await
}
```

Add `use std::net::SocketAddr;` to handler.rs imports.

Update `src/server.rs` — pass `addr` to handler:

```rust
    let svc = service_fn(move |req| {
        let config = config.clone();
        let pool = pool.clone();
        async move { handler::handle(config, pool, addr, req).await }
    });
```

Update `src/proxy.rs` forward and forward_inner to accept `client_addr: SocketAddr`:

```rust
pub async fn forward(
    req: Request<hyper::body::Incoming>,
    upstream_addr: &str,
    pool: &ConnectionPool,
    timeout: Duration,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    match tokio::time::timeout(timeout, forward_inner(req, upstream_addr, pool, client_addr)).await {
        Ok(result) => result,
        Err(_) => Err(ProxyError::Timeout(format!(
            "{upstream_addr}: exceeded {timeout:?}"
        ))),
    }
}
```

In `forward_inner`, after hop-by-hop removal + Host setting, add:

```rust
    // X-Forwarded headers
    parts
        .headers
        .insert("x-forwarded-for", client_addr.ip().to_string().parse().unwrap());
    parts
        .headers
        .insert("x-forwarded-proto", "http".parse().unwrap());
```

- [ ] **Step 5: Run** — `cargo test` — all tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/proxy.rs src/handler.rs src/server.rs tests/common/mod.rs tests/proxy_pass.rs
git commit -m "feat: add X-Forwarded-For and X-Forwarded-Proto headers"
```

---

### Task 2: Enhanced Health Check with Version + Uptime

**Files:**
- Modify: `src/handler.rs`
- Modify: `src/server.rs`
- Modify: `tests/health_check.rs`

- [ ] **Step 1: Write failing test — append to `tests/health_check.rs`**

```rust
#[tokio::test]
async fn health_includes_version_and_uptime() {
    let proxy = TestProxy::start(test_config(vec![])).await;
    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], "0.1.0");
    assert!(body["uptime_secs"].as_f64().unwrap() >= 0.0);
}
```

- [ ] **Step 2: Run** — `cargo test health_includes` — expect FAIL (current health returns only `{"status":"ok"}`)

- [ ] **Step 3: Thread `start_time` through server → handler**

Update `src/server.rs` `run_with_listener` — add `Instant::now()` at the start and pass to handler:

Add `use std::time::Instant;` to imports.

After `let config = Arc::new(config);` add:
```rust
    let start_time = Arc::new(Instant::now());
```

In the service_fn closure, clone and pass it:
```rust
    let start_time = start_time.clone();
    // inside spawned task:
    let svc = service_fn(move |req| {
        let config = config.clone();
        let pool = pool.clone();
        let start_time = start_time.clone();
        async move { handler::handle(config, pool, addr, start_time, req).await }
    });
```

- [ ] **Step 4: Update `src/handler.rs`** — accept `start_time`, update health response

Update handle signature:
```rust
pub async fn handle(
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    start_time: Arc<std::time::Instant>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
```

Pass start_time to route:
```rust
    match route(config, pool, client_addr, start_time, req).await {
```

Update route:
```rust
async fn route(
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    start_time: Arc<std::time::Instant>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, ProxyError> {
    let path = req.uri().path().to_string();

    if path == "/health" {
        return Ok(health_response(&start_time));
    }
    ...
}
```

Update `health_response`:
```rust
fn health_response(start_time: &std::time::Instant) -> Response<Full<Bytes>> {
    let uptime = start_time.elapsed().as_secs_f64();
    let body = format!(
        r#"{{"status":"ok","version":"{}","uptime_secs":{:.1}}}"#,
        env!("CARGO_PKG_VERSION"),
        uptime
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}
```

- [ ] **Step 5: Run** — `cargo test` — all tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/handler.rs src/server.rs tests/health_check.rs
git commit -m "feat: add version and uptime to /health endpoint"
```
