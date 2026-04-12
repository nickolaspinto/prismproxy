# M5: Observability + Upstream Health

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make prismproxy production-observable and self-healing. Operators get a `/metrics` endpoint (Prometheus format), structured per-request access logs, and automatic upstream health checks that remove sick upstreams from rotation without a config change.

**Architecture:** Three independent subsystems added in order:

1. **Metrics** — an `Arc<Metrics>` (counters + histograms backed by `std::sync::atomic`) is built at startup, passed into `handler::handle`, incremented per request, and exposed via a new `/metrics` path that formats Prometheus text.
2. **Access logs** — a structured `tracing::info!` event with fixed fields (`method`, `path`, `status`, `upstream`, `elapsed_ms`, `client_ip`) emitted at the end of every proxied request.
3. **Upstream health checks** — a background task per configured route pings `GET /health` on the upstream every 10 s; after 3 consecutive failures it marks the route "unhealthy" via an `Arc<AtomicBool>` in `RouteState`; the handler skips unhealthy routes and returns 503. Routes recover automatically on 2 consecutive successes.

**No new crate dependencies required** (everything uses `std::sync::atomic`, `tokio::time`, and existing `tracing`).

**Tech Stack:** Rust, tokio, existing prismproxy codebase.

---

## File Structure

| File | Action | Purpose |
|---|---|---|
| `src/metrics.rs` | **NEW** | `Metrics` struct with atomics, `render()` → Prometheus text |
| `src/lib.rs` | Modify | Expose `pub mod metrics` |
| `src/handler.rs` | Modify | Accept `Arc<Metrics>`, record per-request counters/timings, expose `/metrics` |
| `src/server.rs` | Modify | Build `Metrics`, thread into handlers, log access lines |
| `src/state.rs` | Modify | Add `healthy: Arc<AtomicBool>` to `RouteState`; default `true` |
| `src/health_check.rs` | **NEW** | `upstream_health_loop(route_state, upstream_addr, interval, fail_threshold, recover_threshold)` |
| `src/server.rs` | Modify | Spawn one `upstream_health_loop` per route after `build_state` |
| `src/handler.rs` | Modify | Skip routes where `route_state.healthy.load(Relaxed) == false`; return 503 |
| `tests/metrics.rs` | **NEW** | Integration tests for `/metrics` endpoint format |
| `tests/health_check.rs` | **NEW** | Integration tests for upstream health failover + recovery |

---

## Summary Table

| Day | Task | Files | Type |
|---|---|---|---|
| Mon | 1 | `src/metrics.rs`, `src/lib.rs` | feat |
| Mon | 2 | `src/handler.rs`, `src/server.rs` | feat |
| Tue | 3 | `tests/metrics.rs` | test |
| Tue | 4 | `src/state.rs` — add healthy flag | feat |
| Wed | 5 | `src/health_check.rs` | feat |
| Wed | 6 | `src/server.rs` — spawn health loops | feat |
| Thu | 7 | `src/handler.rs` — skip unhealthy routes | feat |
| Thu | 8 | `tests/health_check.rs` | test |
| Fri | 9 | Access log structured fields | feat |
| Fri | 10 | Final: `cargo test`, `cargo clippy`, push | chore |

---

## Task 1: Add `src/metrics.rs`

**Files:**
- Create: `src/metrics.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/metrics.rs`**

```rust
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

/// Per-proxy counters and histograms exposed at /metrics (Prometheus text format).
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub requests_2xx: AtomicU64,
    pub requests_4xx: AtomicU64,
    pub requests_5xx: AtomicU64,
    pub requests_blocked: AtomicU64,
    /// Sum of all response times in milliseconds (for computing mean)
    pub response_time_ms_total: AtomicU64,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            requests_total: AtomicU64::new(0),
            requests_2xx: AtomicU64::new(0),
            requests_4xx: AtomicU64::new(0),
            requests_5xx: AtomicU64::new(0),
            requests_blocked: AtomicU64::new(0),
            response_time_ms_total: AtomicU64::new(0),
        })
    }

    pub fn record(&self, status: u16, elapsed_ms: u64, blocked: bool) {
        self.requests_total.fetch_add(1, Relaxed);
        self.response_time_ms_total.fetch_add(elapsed_ms, Relaxed);
        if blocked {
            self.requests_blocked.fetch_add(1, Relaxed);
            return;
        }
        match status {
            200..=299 => self.requests_2xx.fetch_add(1, Relaxed),
            400..=499 => self.requests_4xx.fetch_add(1, Relaxed),
            500..=599 => self.requests_5xx.fetch_add(1, Relaxed),
            _ => 0,
        };
    }

    /// Render Prometheus text format.
    pub fn render(&self) -> String {
        let total = self.requests_total.load(Relaxed);
        let elapsed = self.response_time_ms_total.load(Relaxed);
        let mean_ms = if total > 0 { elapsed / total } else { 0 };

        format!(
            "# HELP prismproxy_requests_total Total requests handled\n\
             # TYPE prismproxy_requests_total counter\n\
             prismproxy_requests_total {total}\n\
             # HELP prismproxy_requests_2xx 2xx responses\n\
             # TYPE prismproxy_requests_2xx counter\n\
             prismproxy_requests_2xx {}\n\
             # HELP prismproxy_requests_4xx 4xx responses\n\
             # TYPE prismproxy_requests_4xx counter\n\
             prismproxy_requests_4xx {}\n\
             # HELP prismproxy_requests_5xx 5xx responses\n\
             # TYPE prismproxy_requests_5xx counter\n\
             prismproxy_requests_5xx {}\n\
             # HELP prismproxy_requests_blocked Requests blocked by plugins\n\
             # TYPE prismproxy_requests_blocked counter\n\
             prismproxy_requests_blocked {}\n\
             # HELP prismproxy_response_time_ms_mean Mean response time in ms\n\
             # TYPE prismproxy_response_time_ms_mean gauge\n\
             prismproxy_response_time_ms_mean {mean_ms}\n",
            self.requests_2xx.load(Relaxed),
            self.requests_4xx.load(Relaxed),
            self.requests_5xx.load(Relaxed),
            self.requests_blocked.load(Relaxed),
        )
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_2xx: AtomicU64::new(0),
            requests_4xx: AtomicU64::new(0),
            requests_5xx: AtomicU64::new(0),
            requests_blocked: AtomicU64::new(0),
            response_time_ms_total: AtomicU64::new(0),
        }
    }
}
```

- [ ] **Step 2: Add `pub mod metrics;` to `src/lib.rs`**

- [ ] **Step 3: Run** — `cargo check 2>&1 | grep "^error"` — Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add src/metrics.rs src/lib.rs
git commit -m "feat(metrics): add Metrics struct with Prometheus text renderer"
```

---

## Task 2: Wire metrics into handler and server

**Files:**
- Modify: `src/handler.rs`, `src/server.rs`

The handler already tracks `req_start` for `x-response-time`. Extend it to call `metrics.record()`.

- [ ] **Step 1: Update `src/handler.rs`** — add `metrics: Arc<Metrics>` parameter

Update `handle` signature:
```rust
pub async fn handle(
    app_state: Arc<ArcSwap<AppState>>,
    pool: Arc<ConnectionPool>,
    client_addr: SocketAddr,
    start_time: Arc<std::time::Instant>,
    metrics: Arc<crate::metrics::Metrics>,
    is_tls: bool,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible>
```

In `handle`, after computing `elapsed_ms`:
```rust
let status = resp.status().as_u16();
metrics.record(status, elapsed_ms, false);
```

For the plugin-blocked response (inside `route`), pass `blocked = true`.

Add `/metrics` path to `route`:
```rust
if path == "/metrics" {
    let body = metrics.render();
    return Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; version=0.0.4")
        .body(Full::new(Bytes::from(body)))
        .unwrap());
}
```

- [ ] **Step 2: Thread `metrics` through `src/server.rs`**

In `run_with_listener` and `run_with_listener_hot`, create `metrics`:
```rust
let metrics = crate::metrics::Metrics::new();
```

Pass it to `handler::handle` in both `serve_plain` and `serve_tls`.

- [ ] **Step 3: Run** — `cargo test 2>&1 | tail -3` — Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add src/handler.rs src/server.rs
git commit -m "feat(handler): wire Metrics into request pipeline and /metrics endpoint"
```

---

## Task 3: Integration tests for `/metrics`

**File:** `tests/metrics.rs` (create)

```rust
mod common;
use common::{test_config, MockUpstream, TestProxy};

#[tokio::test]
async fn metrics_endpoint_returns_200() {
    let proxy = TestProxy::start(test_config(vec![])).await;
    let resp = reqwest::get(proxy.url("/metrics")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap();
    assert!(ct.contains("text/plain"));
}

#[tokio::test]
async fn metrics_counts_requests() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    // Make 3 requests
    for _ in 0..3 {
        reqwest::get(proxy.url("/anything")).await.unwrap();
    }

    let body = reqwest::get(proxy.url("/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // /metrics itself is counted too — so at least 3 requests total
    assert!(body.contains("prismproxy_requests_total"));
    // Extract count — should be >= 3
    let total: u64 = body
        .lines()
        .find(|l| l.starts_with("prismproxy_requests_total "))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    assert!(total >= 3, "expected >= 3 requests, got {total}");
}

#[tokio::test]
async fn metrics_counts_2xx() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    reqwest::get(proxy.url("/anything")).await.unwrap();

    let body = reqwest::get(proxy.url("/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let count_2xx: u64 = body
        .lines()
        .find(|l| l.starts_with("prismproxy_requests_2xx "))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    assert!(count_2xx >= 1, "expected at least 1 2xx, got {count_2xx}");
}
```

Steps:
- [ ] Create `tests/metrics.rs`
- [ ] Run: `cargo test --test metrics 2>&1 | tail -5` — Expected: all pass
- [ ] Commit:
  ```bash
  git add tests/metrics.rs
  git commit -m "test(metrics): assert /metrics endpoint format and request counting"
  ```

---

## Task 4: Add `healthy` flag to `RouteState`

**File:** `src/state.rs`

- [ ] **Step 1: Add import and field**

Add to `src/state.rs`:
```rust
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
```

Update `RouteState`:
```rust
pub struct RouteState {
    pub route: RouteConfig,
    pub runtime: PluginRuntime,
    /// Set to false by the health-check loop after consecutive failures.
    pub healthy: Arc<AtomicBool>,
}
```

Update `build_state` where `RouteState` is constructed:
```rust
routes.push(RouteState {
    route,
    runtime,
    healthy: Arc::new(AtomicBool::new(true)),
});
```

- [ ] **Step 2: Run** — `cargo check 2>&1 | grep "^error"` — Expected: no errors

- [ ] **Step 3: Run** — `cargo test 2>&1 | tail -3` — Expected: all pass (no behavior change yet)

- [ ] **Step 4: Commit**

```bash
git add src/state.rs
git commit -m "feat(state): add healthy AtomicBool to RouteState for upstream health tracking"
```

---

## Task 5: Add `src/health_check.rs`

**File:** `src/health_check.rs` (create)

```rust
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Duration;
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("http://{upstream_addr}/health");

    let mut consecutive_failures: u32 = 0;
    let mut consecutive_successes: u32 = 0;
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        let ok = client.get(&url).send().await.map_or(false, |r| r.status().is_success());

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
                warn!(upstream = %upstream_addr, "upstream marked unhealthy after {consecutive_failures} failures");
            }
        }
    }
}
```

- [ ] **Step 1: Create `src/health_check.rs`**
- [ ] **Step 2: Add `pub mod health_check;` to `src/lib.rs`**
- [ ] **Step 3: Run** — `cargo check 2>&1 | grep "^error"` — no errors
- [ ] **Step 4: Commit**

```bash
git add src/health_check.rs src/lib.rs
git commit -m "feat(health_check): add upstream health polling loop"
```

---

## Task 6: Spawn health loops in server

**File:** `src/server.rs`

After `build_state` in both `run_with_listener` and `run_with_listener_hot`, iterate routes and spawn health loops:

```rust
// Spawn upstream health-check loops
{
    let state = app_state.load_full();
    for rs in &state.routes {
        let addr = rs.route.upstream.clone();
        let healthy = rs.healthy.clone();
        tokio::spawn(crate::health_check::upstream_health_loop(
            addr,
            healthy,
            std::time::Duration::from_secs(10),
            3,
            2,
        ));
    }
}
```

Note: on hot reload, new routes get new `healthy` flags — their health loops spawn fresh from `run_with_listener_hot`'s initial startup only. The reload loop only swaps state; it does not spawn new health loops for added routes. A follow-up improvement can address this — for now, health loops for routes added after startup are not spawned (the route still works, just without health checking).

- [ ] **Step 1: Apply changes to `src/server.rs`**
- [ ] **Step 2: Run** — `cargo test 2>&1 | tail -3` — Expected: all pass
- [ ] **Step 3: Commit**

```bash
git add src/server.rs
git commit -m "feat(server): spawn upstream health-check loop per route at startup"
```

---

## Task 7: Skip unhealthy routes in handler

**File:** `src/handler.rs`

In the `route` function, update the route selection to skip unhealthy routes:

```rust
use std::sync::atomic::Ordering::Relaxed;

let route_state = state
    .routes
    .iter()
    .find(|rs| path.starts_with(&rs.route.path_prefix) && rs.healthy.load(Relaxed))
    .ok_or_else(|| ProxyError::NoRoute(path.clone()))?;
```

Update `error_response` — add a new `Unhealthy` error variant or reuse `NoRoute` with 503:

Actually, to distinguish "no route configured" (404) from "route exists but upstream is sick" (503), add a new error variant to `src/error.rs`:

```rust
#[error("upstream unhealthy: {0}")]
UpstreamUnhealthy(String),
```

And in handler routing logic:
```rust
let route_state = state
    .routes
    .iter()
    .find(|rs| path.starts_with(&rs.route.path_prefix))
    .ok_or_else(|| ProxyError::NoRoute(path.clone()))?;

if !route_state.healthy.load(Relaxed) {
    return Err(ProxyError::UpstreamUnhealthy(
        route_state.route.upstream.clone(),
    ));
}
```

Add to `error_response`:
```rust
ProxyError::UpstreamUnhealthy(_) => (StatusCode::SERVICE_UNAVAILABLE, err.to_string()),
```

- [ ] **Step 1: Add `UpstreamUnhealthy` to `src/error.rs`**
- [ ] **Step 2: Update `src/handler.rs`**
- [ ] **Step 3: Run** — `cargo check 2>&1 | grep "^error"` — no errors
- [ ] **Step 4: Run** — `cargo test 2>&1 | tail -3` — all pass
- [ ] **Step 5: Commit**

```bash
git add src/error.rs src/handler.rs
git commit -m "feat(handler): return 503 when matched route's upstream is unhealthy"
```

---

## Task 8: Integration tests for upstream health failover

**File:** `tests/health_check.rs` (create)

```rust
mod common;
use common::{test_config, MockUpstream, TestProxy};
use prismproxy::state::RouteState;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;

#[tokio::test]
async fn healthy_route_proxies_normally() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;
    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn unhealthy_flag_causes_503() {
    use arc_swap::ArcSwap;
    use prismproxy::config::{Config, RouteConfig, ServerConfig};
    use prismproxy::plugin::PluginRuntime;
    use prismproxy::state::{AppState, RouteState};
    use tokio::net::TcpListener;

    let upstream = MockUpstream::start(200, "ok").await;
    let upstream_addr = format!("127.0.0.1:{}", upstream.addr.port());

    let healthy = Arc::new(AtomicBool::new(false)); // start unhealthy

    let config = Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms: 5000,
            http_challenge_listen: None,
        },
        routes: vec![RouteConfig {
            path_prefix: "/".to_string(),
            upstream: upstream_addr.clone(),
            plugins: vec![],
        }],
        tls: None,
    };

    let app_state = Arc::new(ArcSwap::from_pointee(AppState {
        timeout_ms: 5000,
        routes: vec![RouteState {
            route: config.routes[0].clone(),
            runtime: PluginRuntime::new().unwrap(),
            healthy: healthy.clone(),
        }],
        tls: None,
    }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        prismproxy::server::run_with_listener(listener, config, async {
            rx.await.ok();
        })
        .await
        .unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let resp = reqwest::get(format!("http://{addr}/anything"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 503, "unhealthy route should return 503");

    // Mark healthy — next request should succeed
    healthy.store(true, Relaxed);
    let resp = reqwest::get(format!("http://{addr}/anything"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let _ = tx.send(());
}
```

Steps:
- [ ] Create `tests/health_check.rs`
- [ ] Run: `cargo test --test health_check 2>&1 | tail -5` — Expected: both pass
- [ ] Commit:
  ```bash
  git add tests/health_check.rs
  git commit -m "test(health_check): assert healthy routes proxy normally, unhealthy return 503"
  ```

---

## Task 9: Structured access logs

**File:** `src/handler.rs`

The current `info!` log line in `handle` only logs method, path, status, and elapsed_ms. Extend it to include `upstream` and `client_ip`:

```rust
info!(
    method = %method,
    path = %path,
    status = %resp.status(),
    elapsed_ms,
    upstream = ?matched_upstream,  // Option<&str>, None for /health and /metrics
    client_ip = %client_addr.ip(),
    "access"
);
```

This requires threading the matched upstream address back from `route()` to `handle()`. Change `route()` to return `(Response<Full<Bytes>>, Option<String>)` or add a small `RouteOutcome` struct.

Alternatively, the simpler approach: add access log emission inside `route()` before returning, where the upstream is known. Only the error path in `handle()` needs special treatment.

Steps:
- [ ] Update `src/handler.rs` to emit structured access log with `upstream` and `client_ip` fields
- [ ] Verify log output with: `RUST_LOG=prismproxy=info cargo test --test proxy_pass 2>&1 | grep "access"`
- [ ] Run: `cargo test 2>&1 | tail -3` — all pass
- [ ] Commit:
  ```bash
  git add src/handler.rs
  git commit -m "feat(handler): structured access logs with upstream and client_ip fields"
  ```

---

## Task 10: Final verification

- [ ] `cargo test` — all tests pass
- [ ] `cargo clippy -- -D warnings` — no warnings
- [ ] `cargo fmt --check` — clean (run `cargo fmt` if needed)
- [ ] `git push origin main`

---

## Verification

After completion:

```bash
# Start prismproxy
cargo run -- config/default.toml &

# Check /metrics
curl http://localhost:8080/metrics
# Expected: Prometheus text with prismproxy_requests_total, etc.

# Generate some traffic
for i in $(seq 1 10); do curl -s http://localhost:8080/ > /dev/null; done

# Check metrics updated
curl http://localhost:8080/metrics | grep requests_total
# Expected: prismproxy_requests_total 11 (10 requests + 1 /metrics call)

# Kill upstream and watch logs
# After 30s (3 * 10s): warn "upstream marked unhealthy"
# Requests return 503
# Restart upstream: after 20s (2 * 10s): info "upstream recovered"
# Requests return 200 again
```
