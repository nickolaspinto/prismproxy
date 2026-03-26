# M1 Hardening: Graceful Shutdown, Timeouts, Config Validation, Body Forwarding

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

## Context

M1 is functionally complete — the proxy accepts TCP, routes HTTP/1.1, forwards to upstreams, and pools connections. But there are production-readiness gaps: the server can't shut down cleanly (CTRL+C doesn't work), slow upstreams hang forever, invalid config addresses only fail at runtime, and POST/PUT/DELETE with request bodies are untested. This plan closes those gaps.

**Goal:** Harden the M1 proxy with graceful shutdown, upstream timeouts, config validation, and verified HTTP method + body forwarding.

**Architecture:** Add `tokio::signal` for CTRL+C handling in main.rs, wrap upstream operations with `tokio::time::timeout` in proxy.rs, add `validate()` to Config, and extend MockUpstream to echo request bodies/methods for testing.

**Tech Stack:** Rust, tokio (signal + time), hyper 1.x, existing prismproxy codebase

---

## Prerequisites

- prismproxy M1 complete (all 14 tests passing)
- Working directory: `/Users/nickolaspinto/prismproxy`

## File Structure

```
src/
├── main.rs          # Add: signal handler → shutdown channel
├── config.rs        # Add: validate() method, address parsing checks
├── proxy.rs         # Add: timeout wrapping around connect + send
├── error.rs         # Add: Timeout variant
├── server.rs        # (unchanged)
├── handler.rs       # (unchanged)
├── pool.rs          # (unchanged)
├── lib.rs           # (unchanged)
tests/
├── common/mod.rs    # Add: EchoUpstream (echoes method + body)
├── proxy_pass.rs    # Add: POST/PUT/DELETE body tests, timeout test
├── config_validation.rs  # Create: config validation tests
└── shutdown.rs      # Create: graceful shutdown test
config/
└── default.toml     # Add: timeout_ms field
```

---

### Task 1: Graceful Shutdown via CTRL+C

**Files:**
- Modify: `src/main.rs`
- Create: `tests/shutdown.rs`

- [ ] **Step 1: Write failing test — `tests/shutdown.rs`**

```rust
mod common;
use common::test_config;
use prismproxy::server;
use tokio::net::TcpListener;

#[tokio::test]
async fn shutdown_signal_stops_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        server::run_with_listener(listener, test_config(vec![]), async {
            rx.await.ok();
        })
        .await
        .unwrap();
    });

    // Server is running — health check works
    let resp = reqwest::get(format!("http://{addr}/health")).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Send shutdown signal
    tx.send(()).unwrap();

    // Server task should complete within 1 second
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
    assert!(result.is_ok(), "server did not shut down in time");
}
```

- [ ] **Step 2: Run** — `cargo test shutdown` — expect PASS (shutdown via oneshot already works in server.rs; this test validates the contract)

- [ ] **Step 3: Update `src/main.rs`** — replace entire file with signal handler

```rust
use prismproxy::config;
use prismproxy::server;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("prismproxy=info".parse()?))
        .json()
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/default.toml".to_string());

    let config = config::Config::from_file(&config_path)?;
    tracing::info!(listen = %config.server.listen, routes = config.routes.len(), "loaded config");

    let listener = TcpListener::bind(&config.server.listen).await?;
    tracing::info!("listening on {}", config.server.listen);

    server::run_with_listener(listener, config, async {
        tokio::signal::ctrl_c().await.ok();
    })
    .await?;
    Ok(())
}
```

Key change: `server::run(config)` → `TcpListener::bind` + `run_with_listener` with `tokio::signal::ctrl_c()` as the shutdown future. Now CTRL+C triggers the existing shutdown branch in `server.rs:50`.

- [ ] **Step 4: Verify** — `cargo build` compiles

- [ ] **Step 5: Run** — `cargo test` — all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/shutdown.rs
git commit -m "feat: add graceful shutdown via CTRL+C signal handling"
```

---

### Task 2: Config Validation

**Files:**
- Modify: `src/config.rs`
- Create: `tests/config_validation.rs`

- [ ] **Step 1: Write failing tests — `tests/config_validation.rs`**

```rust
use prismproxy::config::Config;

#[test]
fn rejects_invalid_listen_address() {
    let toml = r#"
[server]
listen = "not-an-address"

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("listen"));
}

#[test]
fn rejects_invalid_upstream_address() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/api"
upstream = "not-valid"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("upstream"));
}

#[test]
fn rejects_empty_path_prefix() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = ""
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("path_prefix"));
}

#[test]
fn rejects_prefix_without_leading_slash() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "api"
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("path_prefix"));
}

#[test]
fn accepts_valid_config() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert!(cfg.validate().is_ok());
}

#[test]
fn accepts_config_with_no_routes() {
    let toml = r#"
[server]
listen = "0.0.0.0:80"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert!(cfg.validate().is_ok());
}
```

- [ ] **Step 2: Run** — `cargo test config_validation` — expect FAIL (`validate` method doesn't exist)

- [ ] **Step 3: Add `validate()` to `src/config.rs`** — append to `impl Config` block (after `parse`)

```rust
    pub fn validate(&self) -> Result<(), ProxyError> {
        // Validate listen address
        self.server
            .listen
            .parse::<std::net::SocketAddr>()
            .map_err(|e| ProxyError::Config(format!("invalid listen address '{}': {e}", self.server.listen)))?;

        // Validate routes
        for (i, route) in self.routes.iter().enumerate() {
            if route.path_prefix.is_empty() || !route.path_prefix.starts_with('/') {
                return Err(ProxyError::Config(format!(
                    "route[{i}]: path_prefix must start with '/', got '{}'",
                    route.path_prefix
                )));
            }
            route.upstream.parse::<std::net::SocketAddr>().map_err(|e| {
                ProxyError::Config(format!(
                    "route[{i}]: invalid upstream address '{}': {e}",
                    route.upstream
                ))
            })?;
        }

        Ok(())
    }
```

- [ ] **Step 4: Run** — `cargo test config_validation` — all 6 PASS

- [ ] **Step 5: Wire validation into `src/main.rs`** — add after `Config::from_file`:

Replace:
```rust
    let config = config::Config::from_file(&config_path)?;
```
With:
```rust
    let config = config::Config::from_file(&config_path)?;
    config.validate()?;
```

- [ ] **Step 6: Run** — `cargo test` — all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/config.rs src/main.rs tests/config_validation.rs
git commit -m "feat: add config validation for addresses and route prefixes"
```

---

### Task 3: Upstream Timeouts

**Files:**
- Modify: `src/error.rs`, `src/proxy.rs`, `src/config.rs`, `config/default.toml`
- Modify: `tests/common/mod.rs`, `tests/proxy_pass.rs`

- [ ] **Step 1: Add `Timeout` variant to `src/error.rs`**

Add after the `UpstreamConnect` variant:

```rust
    #[error("timeout: {0}")]
    Timeout(String),
```

- [ ] **Step 2: Add `timeout_ms` to config — modify `src/config.rs`**

Add to `ServerConfig`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    #[serde(default = "default_max_idle")]
    pub max_idle_connections: usize,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}
```

Add default function (after `default_max_idle`):

```rust
fn default_timeout_ms() -> u64 {
    30_000
}
```

- [ ] **Step 3: Update `config/default.toml`**

```toml
[server]
listen = "127.0.0.1:8080"
max_idle_connections = 10
timeout_ms = 30000

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:3000"
```

- [ ] **Step 4: Update `tests/common/mod.rs`** — update `test_config` to include `timeout_ms`

Replace the `test_config` function:

```rust
pub fn test_config(routes: Vec<(&str, &str)>) -> Config {
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms: 5000,
        },
        routes: routes
            .into_iter()
            .map(|(prefix, upstream)| RouteConfig {
                path_prefix: prefix.to_string(),
                upstream: upstream.to_string(),
            })
            .collect(),
    }
}
```

Also add a `test_config_with_timeout` helper after `test_config`:

```rust
pub fn test_config_with_timeout(routes: Vec<(&str, &str)>, timeout_ms: u64) -> Config {
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms,
        },
        routes: routes
            .into_iter()
            .map(|(prefix, upstream)| RouteConfig {
                path_prefix: prefix.to_string(),
                upstream: upstream.to_string(),
            })
            .collect(),
    }
}
```

Add `SlowUpstream` after `MockUpstream` — an upstream that delays before responding:

```rust
pub struct SlowUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl SlowUpstream {
    pub async fn start(delay_ms: u64) -> Self {
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
                                .serve_connection(io, service_fn(move |_req| async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .body(Full::new(Bytes::from("slow")))
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

- [ ] **Step 5: Write failing timeout test — append to `tests/proxy_pass.rs`**

Replace the existing import line at the top of the file:

```rust
use common::{test_config, test_config_with_timeout, MockUpstream, SlowUpstream, TestProxy};
```

Append test:

```rust
#[tokio::test]
async fn returns_504_when_upstream_times_out() {
    let upstream = SlowUpstream::start(5000).await; // 5s delay
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    // Proxy with 200ms timeout — upstream will never respond in time
    let proxy = TestProxy::start(test_config_with_timeout(vec![("/", &addr)], 200)).await;

    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    assert_eq!(resp.status(), 504);
}
```

- [ ] **Step 6: Run** — `cargo test returns_504` — expect FAIL (no timeout logic yet, request will hang or return 200)

- [ ] **Step 7: Update `src/proxy.rs`** — add timeout wrapping. Replace entire file:

```rust
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};
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
) -> Result<Response<Full<Bytes>>, ProxyError> {
    match tokio::time::timeout(timeout, forward_inner(req, upstream_addr, pool)).await {
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

    let (resp_parts, resp_body) = resp.into_parts();
    let resp_bytes = resp_body
        .collect()
        .await
        .map_err(ProxyError::Hyper)?
        .to_bytes();

    pool.release(upstream_addr, sender).await;

    Ok(Response::from_parts(resp_parts, Full::new(resp_bytes)))
}
```

- [ ] **Step 8: Update `src/handler.rs`** — pass timeout to proxy::forward

Replace the `route` function:

```rust
async fn route(
    config: Arc<Config>,
    pool: Arc<ConnectionPool>,
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
    proxy::forward(req, &route.upstream, &pool, timeout).await
}
```

- [ ] **Step 9: Update `error_response` in `src/handler.rs`** — add Timeout → 504

Replace the `error_response` function:

```rust
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
```

- [ ] **Step 10: Run** — `cargo test` — ALL tests pass (including timeout test)

- [ ] **Step 11: Commit**

```bash
git add src/error.rs src/config.rs src/proxy.rs src/handler.rs config/default.toml tests/
git commit -m "feat: add upstream timeout with 504 Gateway Timeout response"
```

---

### Task 4: HTTP Method + Body Forwarding Tests

**Files:**
- Modify: `tests/common/mod.rs`, `tests/proxy_pass.rs`

- [ ] **Step 1: Add `EchoUpstream` to `tests/common/mod.rs`** — append after `SlowUpstream`

An upstream that echoes back the request method and body as JSON:

```rust
pub struct EchoUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl EchoUpstream {
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
                                    let method = req.method().to_string();
                                    let path = req.uri().path().to_string();
                                    let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
                                    let body_str = String::from_utf8_lossy(&body_bytes).to_string();

                                    let json = format!(
                                        r#"{{"method":"{}","path":"{}","body":"{}"}}"#,
                                        method, path, body_str
                                    );

                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .header("content-type", "application/json")
                                            .body(Full::new(Bytes::from(json)))
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

- [ ] **Step 2: Write body forwarding tests — append to `tests/proxy_pass.rs`**

Replace the import line at the top of the file (adds `EchoUpstream`):

```rust
use common::{test_config, test_config_with_timeout, EchoUpstream, MockUpstream, SlowUpstream, TestProxy};
```

Append tests:

```rust
#[tokio::test]
async fn forwards_post_with_body() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(proxy.url("/submit"))
        .body("hello world")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "POST");
    assert_eq!(body["path"], "/submit");
    assert_eq!(body["body"], "hello world");
}

#[tokio::test]
async fn forwards_put_with_json_body() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(proxy.url("/resource/1"))
        .header("content-type", "application/json")
        .body(r#"{"name":"test"}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "PUT");
    assert_eq!(body["body"], r#"{"name":"test"}"#);
}

#[tokio::test]
async fn forwards_delete_request() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let client = reqwest::Client::new();
    let resp = client
        .delete(proxy.url("/resource/1"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "DELETE");
    assert_eq!(body["path"], "/resource/1");
}
```

- [ ] **Step 3: Run** — `cargo test forwards_` — all 3 PASS (proxy already forwards all methods + bodies)

- [ ] **Step 4: Commit**

```bash
git add tests/common/mod.rs tests/proxy_pass.rs
git commit -m "test: add HTTP method and body forwarding integration tests"
```

---

### Task 5: Final Verification + Push

- [ ] **Step 1: Full test suite** — `cargo test` — all pass
- [ ] **Step 2: Lint** — `cargo clippy -- -D warnings` — clean
- [ ] **Step 3: Format** — `cargo fmt --check` — clean (run `cargo fmt` if needed)
- [ ] **Step 4: Push** — `git push origin main`

---

## Verification

After completing all tasks:

```bash
# All tests pass
cargo test

# Start a slow upstream to test timeout
python3 -c "
import http.server, time

class SlowHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        time.sleep(60)
handler = SlowHandler
http.server.HTTPServer(('127.0.0.1', 3000), handler).serve_forever()
" &

# Start prismproxy with 5s timeout
RUST_LOG=prismproxy=debug cargo run -- config/default.toml &

# Test health
curl http://localhost:8080/health              # → 200 {"status":"ok"}

# Test timeout (should return 504 after ~30s)
curl -v http://localhost:8080/                 # → 504 Gateway Timeout

# Test CTRL+C (should log "shutting down" and exit cleanly)
# Press Ctrl+C → clean exit
```
