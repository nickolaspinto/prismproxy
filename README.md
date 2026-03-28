# prismproxy

A programmable HTTP reverse proxy with WebAssembly plugin support, built in Rust.

## Architecture

```
Client ──► TCP Accept ──► HTTP/1.1 Parse ──► Route Match ──► Proxy Pass ──► Upstream
                                                  │
                                             /health ──► 200 OK
```

## Features (Milestone 1)

- Async TCP listener with tokio
- HTTP/1.1 reverse proxy via hyper
- TOML-based route configuration with validation
- Prefix-based route matching (first match wins)
- Connection pooling with stale connection retry
- Health check endpoint (`/health`) with version and uptime
- Upstream request timeouts (504 Gateway Timeout)
- Graceful shutdown via CTRL+C
- X-Forwarded-For and X-Forwarded-Proto headers
- Structured JSON logging via tracing
- Hop-by-hop header filtering

## Quick Start

```bash
# Build
cargo build --release

# Configure (edit config/default.toml)
cat config/default.toml

# Run
cargo run -- config/default.toml

# Test
curl http://localhost:8080/health
```

## Configuration

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

Routes are matched top-to-bottom; first prefix match wins.

## Development

```bash
cargo test          # Run tests
cargo clippy        # Lint
cargo fmt           # Format
```

## Roadmap

- [x] **M1:** TCP + HTTP/1.1 reverse proxy
- [ ] **M2:** WASM plugin runtime (wasmtime)
- [ ] **M3:** Hot reload + plugin chain
- [ ] **M4:** TLS termination + HTTP/2
- [ ] **M5:** Observability + metrics
- [ ] **M6:** Documentation + demo

## License

MIT
