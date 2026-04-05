# Weekly Easy Commits

> **Quick reference:** 10 self-contained commits, 2 per day Mon–Fri. Each is independent — do them in any order. All are small (< 30 lines changed). Run `cargo test` after each to confirm nothing broke.

**Goal:** Keep the project moving forward with low-effort, high-value polish between milestones.

**No new dependencies required for any of these.**

---

## Monday — Day 1

### Commit 1: Add TLS example config file

**File:** `config/tls-example.toml` (create)

```toml
# TLS + HTTP/2 example — edit domains and paths before use
[server]
listen = "0.0.0.0:443"
http_challenge_listen = "0.0.0.0:80"
max_idle_connections = 10
timeout_ms = 30000

[tls]
acme_email = "you@example.com"
# acme_directory = "https://acme-staging-v02.api.letsencrypt.org/directory"  # uncomment to test
acme_directory = "https://acme-v02.api.letsencrypt.org/directory"
cache_dir = "./certs"
domains = ["example.com", "www.example.com"]

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:3000"
```

Steps:
- [ ] Create the file above
- [ ] Run: `cargo test 2>&1 | tail -3` — Expected: all pass
- [ ] Commit:
  ```bash
  git add config/tls-example.toml
  git commit -m "docs(config): add TLS example config with ACME"
  ```

---

### Commit 2: Enrich `/health` response — add `tls` and `routes` fields

**File:** `src/handler.rs` — modify `health_response` and `route` functions

The health response currently returns `{"status":"ok","version":"...","uptime_secs":...}`.  
Add `tls: true/false` and `routes: N` so operators can confirm the proxy is serving TLS.

The `health_response` function needs access to `AppState` to read these. Change it from taking only `start_time` to also taking the loaded state:

In `src/handler.rs`, update the `route` function's `/health` branch (around line 49):

```rust
if path == "/health" {
    let state = app_state.load_full();
    let routes = state.routes.len();
    let tls = state.tls.is_some();
    return Ok(health_response(&start_time, routes, tls));
}
```

Update `health_response` signature and body:

```rust
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
```

Steps:
- [ ] Apply both changes to `src/handler.rs`
- [ ] Run: `cargo check 2>&1 | grep "^error"` — Expected: no errors
- [ ] Run: `cargo test 2>&1 | tail -3` — Expected: all pass
- [ ] Commit:
  ```bash
  git add src/handler.rs
  git commit -m "feat(health): add routes count and tls flag to /health response"
  ```

---

## Tuesday — Day 2

### Commit 3: Add integration test — `/health` returns correct JSON fields

**File:** `tests/health.rs` (create)

```rust
mod common;
use common::{MockUpstream, test_config};

#[tokio::test]
async fn health_returns_status_ok_with_version_and_uptime() {
    let upstream = MockUpstream::start(200, "up").await;
    let proxy = common::TestProxy::start(test_config(vec![
        ("/", &format!("127.0.0.1:{}", upstream.addr.port())),
    ])).await;

    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
    assert!(body["uptime_secs"].is_number());
    assert_eq!(body["routes"], 1);
    assert_eq!(body["tls"], false);
}

#[tokio::test]
async fn health_endpoint_does_not_proxy_to_upstream() {
    // upstream returns 500 — /health should still return 200 from the proxy itself
    let upstream = MockUpstream::start(500, "err").await;
    let proxy = common::TestProxy::start(test_config(vec![
        ("/", &format!("127.0.0.1:{}", upstream.addr.port())),
    ])).await;

    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
}
```

Steps:
- [ ] Create `tests/health.rs` with the content above
- [ ] Run: `cargo test --test health 2>&1 | tail -10` — Expected: both tests pass
- [ ] Run: `cargo fmt && cargo fmt --check` — no diff
- [ ] Commit:
  ```bash
  git add tests/health.rs
  git commit -m "test(health): assert /health JSON fields and proxy independence"
  ```

---

### Commit 4: Add `cert_needs_renewal` edge case unit tests

**File:** `src/acme.rs` — add tests to existing `tests` module

Add these two tests to the `#[cfg(test)] mod tests` block at the bottom of `src/acme.rs`:

```rust
#[test]
fn cert_needs_renewal_returns_true_when_cert_missing() {
    assert!(cert_needs_renewal("/nonexistent/path/certdir"));
}

#[test]
fn cert_needs_renewal_returns_false_when_recently_issued() {
    let dir = tempfile::TempDir::new().unwrap();
    // Write a dummy cert.pem so the file exists
    std::fs::write(dir.path().join("cert.pem"), "dummy").unwrap();
    // Write issued_at as now
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(dir.path().join("issued_at"), now.to_string()).unwrap();

    assert!(!cert_needs_renewal(dir.path().to_str().unwrap()));
}

#[test]
fn cert_needs_renewal_returns_true_when_issued_at_old() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("cert.pem"), "dummy").unwrap();
    // issued_at = 0 (epoch) → definitely expired
    std::fs::write(dir.path().join("issued_at"), "0").unwrap();

    assert!(cert_needs_renewal(dir.path().to_str().unwrap()));
}
```

Note: `tempfile` is already a dev-dependency — no new deps needed.

Steps:
- [ ] Add the three tests to `src/acme.rs` tests module
- [ ] Run: `cargo test --lib acme::tests 2>&1 | tail -10` — Expected: 5 tests pass (2 existing + 3 new)
- [ ] Commit:
  ```bash
  git add src/acme.rs
  git commit -m "test(acme): cert_needs_renewal edge cases (missing, fresh, old)"
  ```

---

## Wednesday — Day 3

### Commit 5: Add `x-response-time` header to all responses

**File:** `src/handler.rs` — add timing header in `handle`

After `proxy::forward` returns, inject an `x-response-time` header with elapsed milliseconds.

In `handle`, change the `match route(...)` block:

```rust
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
    let req_start = std::time::Instant::now();

    match route(app_state, pool, client_addr, start_time, is_tls, req).await {
        Ok(mut resp) => {
            let elapsed_ms = req_start.elapsed().as_millis();
            resp.headers_mut().insert(
                "x-response-time",
                format!("{elapsed_ms}ms").parse().unwrap(),
            );
            info!(%method, %path, status = %resp.status(), elapsed_ms, "response");
            Ok(resp)
        }
        Err(e) => {
            error!(%method, %path, error = %e, "failed");
            Ok(error_response(e))
        }
    }
}
```

Steps:
- [ ] Apply the change to `src/handler.rs`
- [ ] Run: `cargo test 2>&1 | tail -3` — Expected: all pass
- [ ] Commit:
  ```bash
  git add src/handler.rs
  git commit -m "feat(handler): add x-response-time header with elapsed ms"
  ```

---

### Commit 6: Add `warn!` when proxy starts with zero routes

**File:** `src/state.rs` — add a warning in `build_state`

In `build_state`, after building the routes vec, add:

```rust
if routes.is_empty() {
    tracing::warn!("no routes configured — all requests will return 404");
}
```

Place it right before `Ok(AppState { ... })`.

Steps:
- [ ] Add the `warn!` to `src/state.rs`
- [ ] Run: `cargo test --lib 2>&1 | tail -3` — Expected: all pass
- [ ] Commit:
  ```bash
  git add src/state.rs
  git commit -m "feat(state): warn when starting with zero routes configured"
  ```

---

## Thursday — Day 4

### Commit 7: Write `docs/plugin-abi.md` — WASM plugin ABI reference

**File:** `docs/plugin-abi.md` (create)

```markdown
# WASM Plugin ABI

prismproxy loads WebAssembly plugins per route. Plugins are called on every incoming request before it is forwarded to the upstream.

## Interface

Your plugin module must export exactly one function:

```
on_request(method_ptr: i32, method_len: i32, path_ptr: i32, path_len: i32) -> i32
```

### Parameters

| Parameter | Type | Description |
|---|---|---|
| `method_ptr` | `i32` | Pointer into WASM linear memory where the HTTP method string starts (UTF-8) |
| `method_len` | `i32` | Length of the method string in bytes |
| `path_ptr` | `i32` | Pointer into WASM linear memory where the request path starts (UTF-8) |
| `path_len` | `i32` | Length of the path string in bytes |

### Return value

- `0` — allow the request (pass through to upstream)
- any non-zero — block the request (proxy returns `403 Forbidden`)

## Memory model

The proxy writes method and path into the plugin's linear memory before calling `on_request`. You do not need to allocate or export a memory — the host manages this.

## Example (WAT)

```wat
(module
  (func (export "on_request")
    (param $method_ptr i32) (param $method_len i32)
    (param $path_ptr i32)   (param $path_len i32)
    (result i32)
    ;; allow all requests
    i32.const 0
  )
)
```

## Example (Rust)

```rust
#[no_mangle]
pub extern "C" fn on_request(
    _method_ptr: i32, _method_len: i32,
    path_ptr: i32, path_len: i32,
) -> i32 {
    let path = unsafe {
        let slice = std::slice::from_raw_parts(path_ptr as *const u8, path_len as usize);
        std::str::from_utf8_unchecked(slice)
    };
    if path.starts_with("/admin") { 1 } else { 0 }
}
```

Compile with: `cargo build --target wasm32-unknown-unknown --release`

## Plugin chain

Routes can have multiple plugins. They are called in order. If any plugin returns non-zero, the request is blocked immediately — subsequent plugins in the chain are not called.

```toml
[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"
plugins = ["./plugins/auth.wasm", "./plugins/rate-limit.wasm"]
```
```

Steps:
- [ ] Create `docs/plugin-abi.md` with the content above
- [ ] Run: `cargo test 2>&1 | tail -3` — Expected: all pass (no code changed)
- [ ] Commit:
  ```bash
  git add docs/plugin-abi.md
  git commit -m "docs: add WASM plugin ABI reference"
  ```

---

### Commit 8: Add `Dockerfile`

**File:** `Dockerfile` (create)

```dockerfile
FROM rust:1.77-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/prismproxy /usr/local/bin/prismproxy
COPY config/default.toml /etc/prismproxy/config.toml
EXPOSE 8080 443 80
ENTRYPOINT ["prismproxy"]
CMD ["/etc/prismproxy/config.toml"]
```

Also add `.dockerignore`:

```
target/
.worktrees/
.git/
certs/
*.wasm
```

Steps:
- [ ] Create `Dockerfile`
- [ ] Create `.dockerignore`
- [ ] Run: `cargo test 2>&1 | tail -3` — Expected: all pass (no code changed)
- [ ] Commit:
  ```bash
  git add Dockerfile .dockerignore
  git commit -m "build: add Dockerfile for containerized deployment"
  ```

---

## Friday — Day 5

### Commit 9: Add integration test — `x-response-time` header present

**File:** `tests/proxy.rs` or nearest existing integration test file — add one test

Check which integration test file to add to:
```bash
ls tests/
```

Add to the most relevant existing file (e.g. `tests/proxy.rs` if it exists, otherwise create it):

```rust
#[tokio::test]
async fn response_includes_x_response_time_header() {
    let upstream = common::MockUpstream::start(200, "ok").await;
    let proxy = common::TestProxy::start(common::test_config(vec![
        ("/", &format!("127.0.0.1:{}", upstream.addr.port())),
    ])).await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert!(
        resp.headers().contains_key("x-response-time"),
        "x-response-time header should be present"
    );
    let val = resp.headers()["x-response-time"].to_str().unwrap();
    assert!(val.ends_with("ms"), "expected format like '5ms', got: {val}");
}
```

Steps:
- [ ] Find the right test file with `ls tests/`
- [ ] Add the test to that file (or create `tests/proxy.rs` with `mod common;` at top)
- [ ] Run: `cargo test --test <filename> 2>&1 | tail -5` — Expected: test passes
- [ ] Run: `cargo fmt && cargo fmt --check`
- [ ] Commit:
  ```bash
  git add tests/<filename>.rs
  git commit -m "test: assert x-response-time header is present on all responses"
  ```

---

### Commit 10: Merge M4 PR and clean up worktree

**Steps:**
- [ ] Go to https://github.com/nickolaspinto/prismproxy/pull/1 and merge (or merge locally)
- [ ] Locally:
  ```bash
  git checkout main
  git pull origin main
  git worktree remove .worktrees/m4-tls-http2
  git branch -d feat/m4-tls-http2
  ```
- [ ] Verify: `cargo test 2>&1 | tail -3` on main — all pass
- [ ] No commit needed — this is a cleanup step

---

## Summary table

| Day | Commit | Files | Type |
|---|---|---|---|
| Mon | 1 | `config/tls-example.toml` | docs |
| Mon | 2 | `src/handler.rs` | feat |
| Tue | 3 | `tests/health.rs` | test |
| Tue | 4 | `src/acme.rs` | test |
| Wed | 5 | `src/handler.rs` | feat |
| Wed | 6 | `src/state.rs` | feat |
| Thu | 7 | `docs/plugin-abi.md` | docs |
| Thu | 8 | `Dockerfile`, `.dockerignore` | build |
| Fri | 9 | `tests/*.rs` | test |
| Fri | 10 | cleanup | chore |
