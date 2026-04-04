# M4: TLS Termination + HTTP/2 — Design

**Date:** 2026-04-04
**Status:** Approved
**Milestone:** M4 of 6

---

## Goal

Add HTTPS termination with automatic certificate provisioning (ACME/Let's Encrypt HTTP-01) and HTTP/2 support on the downstream (client-to-proxy) connection. Upstream connections remain HTTP/1.1. Plain HTTP mode is preserved via backward-compatible config.

---

## Architecture

`AppState` gains an optional `tls: Option<TlsState>`. `TlsState` holds a `tokio_rustls::TlsAcceptor` built from the current certificate. The accept loop branches on whether TLS is configured:

- **No TLS:** plain TCP → HTTP/1.1 (existing path, unchanged)
- **TLS:** `TlsAcceptor.accept()` → `hyper_util::server::conn::auto::Builder` → H2 or H1.1 selected by ALPN negotiation

A dedicated `renewal_loop` task runs daily. It checks whether the cached cert is missing or expires within 30 days. If so: spins up a minimal HTTP-01 challenge server on `http_challenge_listen`, completes the ACME flow via `instant-acme`, writes cert + key PEM to `cache_dir`, then calls `arc_swap.store(Arc::new(new_state))` — the same atomic swap used by hot reload.

```
client (HTTPS/H2 or HTTPS/H1.1)
  → TcpListener (443)
  → TlsAcceptor (rustls, ALPN: ["h2", "http/1.1"])
  → auto::Builder (hyper_util) — selects H2 or H1.1
  → handler (ArcSwap load → route → plugin → forward)
  → upstream (TCP, HTTP/1.1 only)

ACME renewal_loop (daily)
  → check cert expiry in cache_dir
  → if missing or < 30 days: provision via instant-acme
      → start HTTP-01 challenge server on http_challenge_listen
      → complete challenge → receive cert
      → write cert.pem + key.pem to cache_dir
      → build_tls_state → arc_swap.store(new AppState)
```

---

## Config Format

`[tls]` is optional. When absent, prismproxy runs plain HTTP/1.1 as before (fully backward compatible).

```toml
[server]
listen = "0.0.0.0:443"
http_challenge_listen = "0.0.0.0:80"   # required when [tls] is present
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

`acme_directory` defaults to the Let's Encrypt production URL. Set it to `https://acme-staging-v02.api.letsencrypt.org/directory` for testing.

---

## New Files

| File | Responsibility |
|---|---|
| `src/tls.rs` | `TlsState { acceptor: TlsAcceptor, domains: Vec<String> }`, `build_tls_state(tls_config: &TlsConfig, cert_pem: &str, key_pem: &str) -> Result<TlsState, ProxyError>` |
| `src/acme.rs` | `provision(tls_config: &TlsConfig) -> Result<(cert_pem, key_pem), ProxyError>` — runs full HTTP-01 ACME flow; `renewal_loop(config_path, app_state)` — daily check + renewal |

---

## Modified Files

| File | Change |
|---|---|
| `Cargo.toml` | Add `rustls`, `tokio-rustls`, `instant-acme`, `rcgen`; enable hyper `http2` feature |
| `src/config.rs` | Add `TlsConfig` struct; add `tls: Option<TlsConfig>` to `Config`; add `http_challenge_listen: Option<String>` to `ServerConfig` |
| `src/state.rs` | Add `tls: Option<TlsState>` to `AppState`; `build_state` calls `build_tls_state` when `config.tls` is `Some` and a cached cert exists |
| `src/server.rs` | Accept loop branches on `state.tls`; TLS path uses `auto::Builder`; `run_with_listener_hot` spawns `renewal_loop` when `[tls]` is configured |
| `src/proxy.rs` | Set `x-forwarded-proto: https` when request arrives on TLS connection (pass a `bool is_tls` flag from server to handler to proxy) |
| `src/lib.rs` | Expose `pub mod tls` and `pub mod acme` |

---

## ACME Provisioning Detail

`provision()` in `src/acme.rs`:

1. Create ACME account with `acme_email` at `acme_directory`
2. Create order for all `domains`
3. For each domain, fetch HTTP-01 challenge token + key authorization
4. Start a tokio HTTP server on `http_challenge_listen` serving `/.well-known/acme-challenge/<token>` → key authorization string
5. Notify ACME server challenges are ready; poll until valid (max 30s per domain)
6. Shut down challenge server
7. Generate RSA-2048 key pair + CSR via `rcgen`
8. Finalize order, download cert chain PEM
9. Write `{cache_dir}/cert.pem` and `{cache_dir}/key.pem`
10. Return `(cert_pem, key_pem)`

`renewal_loop()` in `src/acme.rs`:

- Runs in `tokio::spawn`, ticks every 24 hours
- Reads `{cache_dir}/cert.pem`, parses expiry
- If expiry < 30 days OR cert missing: calls `provision()`, on success calls `try_reload_with_tls()` (builds new AppState, stores via ArcSwap)
- On error: `warn!` + keep current state (same lenient policy as config hot reload)

Cert expiry parsing uses `rcgen`'s or `rustls`'s certificate parsing — no additional crate needed.

---

## x-forwarded-proto Propagation

`server.rs` passes a `bool` (`is_tls: true/false`) down to `handler::handle`, which passes it to `proxy::forward`. `proxy.rs` sets:

```
x-forwarded-proto: https   // TLS path
x-forwarded-proto: http    // plain path (existing behavior)
```

---

## Error Handling

| Scenario | Behavior |
|---|---|
| ACME provisioning fails at startup (no cached cert) | `Err(ProxyError)` → process exits with error message |
| Cached cert present but ACME renewal fails | `warn!` + keep serving with current cert |
| TLS handshake fails (bad client cert, etc.) | Log error per connection, keep accepting |
| Cert is expired and renewal fails | `error!` logged; server keeps running with expired cert |
| `cache_dir` not writable | Provisioning fails → startup error |
| `http_challenge_listen` already in use | Provisioning fails → startup error |

---

## Dependencies Added

| Crate | Version | Why |
|---|---|---|
| `rustls` | `0.23` | TLS implementation (pure Rust) |
| `tokio-rustls` | `0.26` | Async TLS wrapping tokio streams |
| `instant-acme` | `0.7` | ACME protocol client |
| `rcgen` | `0.13` | RSA key + CSR generation |
| hyper `http2` feature | — | H2 server support |
| hyper-util `server-auto` feature | — | `auto::Builder` for H1/H2 multiplexing |

---

## Tests

### Unit (`src/config.rs`)
- `tls_config_parses` — `[tls]` section deserializes correctly
- `missing_tls_defaults_none` — no `[tls]` → `config.tls` is `None`

### Unit (`src/tls.rs`)
- `build_tls_state_with_valid_cert_succeeds` — rcgen self-signed cert → `TlsState` builds without error
- `build_tls_state_with_invalid_pem_fails` — garbage PEM → `Err(ProxyError)`

### Unit (`src/acme.rs`)
- `challenge_server_responds_to_well_known_path` — start challenge server, GET `/.well-known/acme-challenge/<token>`, assert response matches key authorization

### Integration (`tests/tls.rs`)
- `https_request_returns_200` — proxy with rcgen self-signed cert; `reqwest::Client` with `danger_accept_invalid_certs(true)`; assert 200
- `http2_negotiated_via_alpn` — same setup; assert `resp.version() == Version::HTTP_2`
- `http1_also_works_over_tls` — force H1.1 via reqwest; assert 200 and `Version::HTTP_11`
- `x_forwarded_proto_is_https` — upstream echoes headers; assert `x-forwarded-proto: https`
- `plain_http_mode_unaffected` — config with no `[tls]`; plain HTTP/1.1 still works

---

## Out of Scope (M5+)

- HTTP/2 upstream connections
- Mutual TLS (mTLS)
- Multiple certificates (SNI for different domains)
- Certificate pinning
- OCSP stapling
- DNS-01 ACME challenge
