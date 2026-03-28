# Changelog

## [0.1.0] - 2026-03-27

### Added
- Async TCP listener with tokio
- HTTP/1.1 reverse proxy via hyper 1.x
- TOML configuration with route definitions and validation
- Prefix-based route matching
- Connection pooling with stale connection retry
- `/health` JSON endpoint with version and uptime
- Upstream request timeouts (configurable `timeout_ms`, returns 504)
- Graceful shutdown via CTRL+C signal handling
- X-Forwarded-For and X-Forwarded-Proto proxy headers
- Structured JSON logging
- Hop-by-hop header filtering
- Config validation for listen/upstream addresses and route prefixes
- CI with GitHub Actions (fmt, clippy, test)
