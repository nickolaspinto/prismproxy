# prismproxy

A programmable HTTPS reverse proxy with WebAssembly plugin support, built in Rust.

## Architecture

```
Client (HTTPS/H2 or HTTPS/H1.1)
  → TcpListener
  → TlsAcceptor (rustls, ALPN: ["h2", "http/1.1"])
  → auto::Builder — selects H2 or H1.1
  → Route Match → WASM Plugin Chain → Proxy Pass
  → Upstream (TCP, HTTP/1.1)

ACME renewal_loop (daily)
  → check cert expiry
  → if < 30 days: provision via Let's Encrypt HTTP-01
  → atomic cert write → ArcSwap state reload
```

Plain HTTP/1.1 mode is preserved — `[tls]` config section is optional.

## Features

- **TLS termination** with automatic certificate provisioning via ACME/Let's Encrypt (HTTP-01)
- **HTTP/2** support via ALPN negotiation (H2 or H1.1 selected per connection)
- **Daily cert renewal** — atomically reloads cert without restart when expiry < 30 days
- **WASM plugin chains** per route — load `.wasm` plugins that can allow or block requests
- **Hot config reload** — watches config file, reloads routes and plugins without restart
- **Connection pooling** with stale connection retry
- **Prometheus metrics** endpoint (`/metrics`) — request counters, status class buckets, mean response time
- **Upstream health checks** — background loop polls `GET /health` per route every 10s; marks unhealthy after 3 failures (503), auto-recovers after 2 successes
- **Structured access logs** — per-request `tracing::info!` with `method`, `path`, `status`, `elapsed_ms`, `client_ip`
- **Health check** endpoint (`/health`) with version and uptime
- **Upstream timeouts** (504 Gateway Timeout)
- **Structured JSON logging** via tracing
- **Graceful shutdown** via CTRL+C
- `x-forwarded-for`, `x-forwarded-proto: https/http`, hop-by-hop header filtering

## Quick Start

```bash
# Build
cargo build --release

# Configure
cat config/default.toml

# Run (plain HTTP)
cargo run -- config/default.toml

# Run (HTTPS — provisions cert on first start)
cargo run -- config/tls.toml
```

## Configuration

### Plain HTTP

```toml
[server]
listen = "127.0.0.1:8080"
max_idle_connections = 10
timeout_ms = 30000

[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:8000"
```

### HTTPS + HTTP/2 (ACME/Let's Encrypt)

```toml
[server]
listen = "0.0.0.0:443"
http_challenge_listen = "0.0.0.0:80"   # required for ACME HTTP-01
max_idle_connections = 10
timeout_ms = 30000

[tls]
acme_email = "admin@example.com"
acme_directory = "https://acme-v02.api.letsencrypt.org/directory"
cache_dir = "./certs"
domains = ["example.com", "www.example.com"]

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:3000"
```

Set `acme_directory` to `https://acme-staging-v02.api.letsencrypt.org/directory` for testing.

### Per-route WASM plugins

```toml
[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"
plugins = ["./plugins/auth.wasm", "./plugins/rate.wasm"]
```

Plugins are called in order; returning non-zero blocks the request with 403.

Routes are matched top-to-bottom; first prefix match wins.

## Development

```bash
cargo test          # Run tests (78 tests)
cargo clippy        # Lint
cargo fmt           # Format
```

## Roadmap

- [x] **M1:** TCP + HTTP/1.1 reverse proxy
- [x] **M2:** WASM plugin runtime (wasmtime)
- [x] **M3:** Hot reload + per-route plugin chains
- [x] **M4:** TLS termination + HTTP/2 + ACME auto-provisioning
- [x] **M5:** Observability + metrics
- [ ] **M6:** Documentation + demo

## License

MIT
