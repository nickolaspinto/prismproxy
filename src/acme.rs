use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use instant_acme::{Account, ChallengeType, Identifier, NewAccount, NewOrder, OrderStatus};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::config::{Config, TlsConfig};
use crate::error::ProxyError;
use crate::state::{build_state, AppState};

/// Shared map of ACME HTTP-01 challenge tokens → key authorizations.
/// Written by [provision_with_store] during renewal; read by the redirect server.
pub type ChallengeStore = Arc<std::sync::RwLock<HashMap<String, String>>>;

/// Returns true if no cert exists or the cert was issued more than 60 days ago.
/// Uses a sidecar `issued_at` file (Unix epoch seconds) written by provision().
/// LE certs are 90 days; we renew at 60 days to leave a 30-day buffer.
pub fn cert_needs_renewal(cache_dir: &str) -> bool {
    let cert_path = Path::new(cache_dir).join("cert.pem");
    if !cert_path.exists() {
        return true;
    }
    let issued_at: u64 = std::fs::read_to_string(Path::new(cache_dir).join("issued_at"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 60 days = 5_184_000 seconds
    now > issued_at + 5_184_000
}

/// Provision a new certificate via ACME HTTP-01.
///
/// Uses a `.renewing` lockfile to prevent concurrent provisioning.
/// Writes cert + key atomically via temp files then rename.
/// Returns (cert_pem, key_pem) on success.
pub async fn provision(
    tls_config: &TlsConfig,
    http_challenge_listen: &str,
) -> Result<(String, String), ProxyError> {
    let cache = Path::new(&tls_config.cache_dir);
    std::fs::create_dir_all(cache)
        .map_err(|e| ProxyError::Acme(format!("create cache_dir: {e}")))?;

    let lock_path = cache.join(".renewing");
    if lock_path.exists() {
        return Err(ProxyError::Acme(
            "another renewal is in progress (.renewing exists)".to_string(),
        ));
    }
    std::fs::write(&lock_path, b"")
        .map_err(|e| ProxyError::Acme(format!("create lockfile: {e}")))?;

    let result = provision_inner(tls_config, http_challenge_listen).await;

    // Always clean up lockfile and temps on failure
    if result.is_err() {
        let _ = std::fs::remove_file(&lock_path);
        let _ = std::fs::remove_file(cache.join("cert.pem.tmp"));
        let _ = std::fs::remove_file(cache.join("key.pem.tmp"));
    } else {
        let _ = std::fs::remove_file(&lock_path);
    }

    result
}

async fn provision_inner(
    tls_config: &TlsConfig,
    http_challenge_listen: &str,
) -> Result<(String, String), ProxyError> {
    // Create ACME account — returns (Account, AccountCredentials)
    let (account, _credentials) = Account::create(
        &NewAccount {
            contact: &[&format!("mailto:{}", tls_config.acme_email)],
            terms_of_service_agreed: true,
            only_return_existing: false,
        },
        &tls_config.acme_directory,
        None,
    )
    .await
    .map_err(|e| ProxyError::Acme(format!("create account: {e}")))?;

    // Create order
    let identifiers: Vec<Identifier> = tls_config
        .domains
        .iter()
        .map(|d| Identifier::Dns(d.clone()))
        .collect();
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .map_err(|e| ProxyError::Acme(format!("new order: {e}")))?;

    // Collect HTTP-01 challenge tokens
    let authorizations = order
        .authorizations()
        .await
        .map_err(|e| ProxyError::Acme(format!("get authorizations: {e}")))?;

    let mut challenges_map: HashMap<String, String> = HashMap::new();
    let mut challenge_urls: Vec<String> = Vec::new();

    for auth in &authorizations {
        let challenge = auth
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Http01)
            .ok_or_else(|| ProxyError::Acme("no HTTP-01 challenge available".to_string()))?;
        let key_auth = order.key_authorization(challenge);
        challenges_map.insert(challenge.token.clone(), key_auth.as_str().to_string());
        challenge_urls.push(challenge.url.clone());
    }

    // Start challenge server
    let listener = TcpListener::bind(http_challenge_listen)
        .await
        .map_err(|e| ProxyError::Acme(format!("bind challenge server: {e}")))?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(run_challenge_server_with_listener(
        listener,
        challenges_map,
        shutdown_rx,
    ));

    // Notify ACME that challenges are ready
    for url in &challenge_urls {
        order
            .set_challenge_ready(url)
            .await
            .map_err(|e| ProxyError::Acme(format!("set challenge ready: {e}")))?;
    }

    // Poll until order is Ready or Invalid (max 120s)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state = order
            .refresh()
            .await
            .map_err(|e| ProxyError::Acme(format!("refresh order: {e}")))?;
        match state.status {
            OrderStatus::Ready => break,
            OrderStatus::Invalid => {
                return Err(ProxyError::Acme("ACME order became invalid".to_string()))
            }
            _ => {}
        }
        if tokio::time::Instant::now() > deadline {
            return Err(ProxyError::Acme(
                "ACME order timed out after 120s".to_string(),
            ));
        }
    }

    // Shut down challenge server
    let _ = shutdown_tx.send(());

    // Generate key pair and CSR
    let params = rcgen::CertificateParams::new(tls_config.domains.clone())
        .map_err(|e| ProxyError::Acme(format!("cert params: {e}")))?;
    let key_pair =
        rcgen::KeyPair::generate().map_err(|e| ProxyError::Acme(format!("key generation: {e}")))?;
    let csr = params
        .serialize_request(&key_pair)
        .map_err(|e| ProxyError::Acme(format!("serialize CSR: {e}")))?;

    // Finalize order
    order
        .finalize(csr.der())
        .await
        .map_err(|e| ProxyError::Acme(format!("finalize order: {e}")))?;

    // Download cert chain (poll until available)
    let cert_chain_pem = loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        match order
            .certificate()
            .await
            .map_err(|e| ProxyError::Acme(format!("download cert: {e}")))?
        {
            Some(cert) => break cert,
            None => continue,
        }
    };

    let key_pem = key_pair.serialize_pem();

    // Atomic write: temp files → rename
    let cache = Path::new(&tls_config.cache_dir);
    let cert_tmp = cache.join("cert.pem.tmp");
    let key_tmp = cache.join("key.pem.tmp");

    std::fs::write(&cert_tmp, &cert_chain_pem)
        .map_err(|e| ProxyError::Acme(format!("write cert.pem.tmp: {e}")))?;
    std::fs::write(&key_tmp, &key_pem)
        .map_err(|e| ProxyError::Acme(format!("write key.pem.tmp: {e}")))?;
    std::fs::rename(&cert_tmp, cache.join("cert.pem"))
        .map_err(|e| ProxyError::Acme(format!("rename cert.pem: {e}")))?;
    std::fs::rename(&key_tmp, cache.join("key.pem"))
        .map_err(|e| ProxyError::Acme(format!("rename key.pem: {e}")))?;

    // Write issued_at for renewal tracking
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::write(cache.join("issued_at"), now.to_string());

    info!(
        domains = ?tls_config.domains,
        "TLS certificate provisioned"
    );
    Ok((cert_chain_pem, key_pem))
}

/// Like [provision] but injects challenge tokens into `store` instead of binding a new listener.
/// Used during renewal when the redirect server already owns the http_challenge_listen port.
pub async fn provision_with_store(
    tls_config: &TlsConfig,
    store: &ChallengeStore,
) -> Result<(String, String), ProxyError> {
    let cache = Path::new(&tls_config.cache_dir);
    std::fs::create_dir_all(cache)
        .map_err(|e| ProxyError::Acme(format!("create cache_dir: {e}")))?;

    let lock_path = cache.join(".renewing");
    if lock_path.exists() {
        return Err(ProxyError::Acme(
            "another renewal is in progress (.renewing exists)".to_string(),
        ));
    }
    std::fs::write(&lock_path, b"")
        .map_err(|e| ProxyError::Acme(format!("create lockfile: {e}")))?;

    let result = provision_inner_with_store(tls_config, store).await;

    if result.is_err() {
        let _ = std::fs::remove_file(&lock_path);
        let _ = std::fs::remove_file(cache.join("cert.pem.tmp"));
        let _ = std::fs::remove_file(cache.join("key.pem.tmp"));
    } else {
        let _ = std::fs::remove_file(&lock_path);
    }
    // Always clear challenge tokens from store
    store.write().unwrap().clear();

    result
}

async fn provision_inner_with_store(
    tls_config: &TlsConfig,
    store: &ChallengeStore,
) -> Result<(String, String), ProxyError> {
    let (account, _credentials) = Account::create(
        &NewAccount {
            contact: &[&format!("mailto:{}", tls_config.acme_email)],
            terms_of_service_agreed: true,
            only_return_existing: false,
        },
        &tls_config.acme_directory,
        None,
    )
    .await
    .map_err(|e| ProxyError::Acme(format!("create account: {e}")))?;

    let identifiers: Vec<Identifier> = tls_config
        .domains
        .iter()
        .map(|d| Identifier::Dns(d.clone()))
        .collect();
    let mut order = account
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .map_err(|e| ProxyError::Acme(format!("new order: {e}")))?;

    let authorizations = order
        .authorizations()
        .await
        .map_err(|e| ProxyError::Acme(format!("get authorizations: {e}")))?;

    let mut challenge_urls: Vec<String> = Vec::new();
    for auth in &authorizations {
        let challenge = auth
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Http01)
            .ok_or_else(|| ProxyError::Acme("no HTTP-01 challenge available".to_string()))?;
        let key_auth = order.key_authorization(challenge);
        store
            .write()
            .unwrap()
            .insert(challenge.token.clone(), key_auth.as_str().to_string());
        challenge_urls.push(challenge.url.clone());
    }

    for url in &challenge_urls {
        order
            .set_challenge_ready(url)
            .await
            .map_err(|e| ProxyError::Acme(format!("set challenge ready: {e}")))?;
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state = order
            .refresh()
            .await
            .map_err(|e| ProxyError::Acme(format!("refresh order: {e}")))?;
        match state.status {
            OrderStatus::Ready => break,
            OrderStatus::Invalid => {
                return Err(ProxyError::Acme("ACME order became invalid".to_string()))
            }
            _ => {}
        }
        if tokio::time::Instant::now() > deadline {
            return Err(ProxyError::Acme(
                "ACME order timed out after 120s".to_string(),
            ));
        }
    }

    // Reuse the finalize + write logic from provision_inner
    let params = rcgen::CertificateParams::new(tls_config.domains.clone())
        .map_err(|e| ProxyError::Acme(format!("cert params: {e}")))?;
    let key_pair =
        rcgen::KeyPair::generate().map_err(|e| ProxyError::Acme(format!("key generation: {e}")))?;
    let csr = params
        .serialize_request(&key_pair)
        .map_err(|e| ProxyError::Acme(format!("serialize CSR: {e}")))?;

    order
        .finalize(csr.der())
        .await
        .map_err(|e| ProxyError::Acme(format!("finalize order: {e}")))?;

    let deadline2 = tokio::time::Instant::now() + Duration::from_secs(60);
    let cert_chain_pem = loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        match order.certificate().await {
            Ok(Some(chain)) => break chain,
            Ok(None) => {}
            Err(e) => return Err(ProxyError::Acme(format!("get certificate: {e}"))),
        }
        if tokio::time::Instant::now() > deadline2 {
            return Err(ProxyError::Acme("cert not ready after 60s".to_string()));
        }
    };

    let cache = Path::new(&tls_config.cache_dir);
    let cert_tmp = cache.join("cert.pem.tmp");
    let key_tmp = cache.join("key.pem.tmp");
    let key_pem = key_pair.serialize_pem();
    std::fs::write(&cert_tmp, &cert_chain_pem)
        .map_err(|e| ProxyError::Acme(format!("write cert.pem.tmp: {e}")))?;
    std::fs::write(&key_tmp, &key_pem)
        .map_err(|e| ProxyError::Acme(format!("write key.pem.tmp: {e}")))?;
    std::fs::rename(&cert_tmp, cache.join("cert.pem"))
        .map_err(|e| ProxyError::Acme(format!("rename cert.pem: {e}")))?;
    std::fs::rename(&key_tmp, cache.join("key.pem"))
        .map_err(|e| ProxyError::Acme(format!("rename key.pem: {e}")))?;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    std::fs::write(cache.join("issued_at"), now_secs.to_string())
        .map_err(|e| ProxyError::Acme(format!("write issued_at: {e}")))?;

    info!(domains = ?tls_config.domains, "certificate provisioned via store");
    Ok((cert_chain_pem, key_pem))
}

/// Daily renewal loop. Reads config from disk each tick to pick up any changes.
/// On renewal success, rebuilds AppState and atomically swaps via ArcSwap.
///
/// If `challenge_store` is Some, challenge tokens are injected into the running redirect server
/// instead of binding the http_challenge_listen port again.
pub async fn renewal_loop(
    config_path: PathBuf,
    app_state: Arc<ArcSwap<AppState>>,
    challenge_store: Option<ChallengeStore>,
) {
    // Tick immediately on first call (interval fires after first period),
    // then every 24 hours.
    let mut interval = tokio::time::interval(Duration::from_secs(86_400));
    loop {
        interval.tick().await;

        let config = match Config::from_file(&config_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("renewal: failed to read config: {e}");
                continue;
            }
        };

        let tls_config = match config.tls {
            Some(ref tc) => tc.clone(),
            None => continue,
        };

        if !cert_needs_renewal(&tls_config.cache_dir) {
            continue;
        }

        info!(domains = ?tls_config.domains, "renewing TLS certificate");
        let result = if let Some(ref store) = challenge_store {
            provision_with_store(&tls_config, store).await
        } else {
            let challenge_listen = match config.server.http_challenge_listen {
                Some(ref l) => l.clone(),
                None => {
                    warn!("renewal: http_challenge_listen not configured, skipping");
                    continue;
                }
            };
            provision(&tls_config, &challenge_listen).await
        };

        match result {
            Ok(_) => match build_state(config) {
                Ok(new_state) => {
                    app_state.store(Arc::new(new_state));
                    info!("TLS certificate renewed, state reloaded");
                }
                Err(e) => warn!("renewal: build_state failed after provision: {e}"),
            },
            Err(e) => warn!("cert renewal failed, keeping current cert: {e}"),
        }
    }
}

/// Run the HTTP-01 challenge server using an already-bound listener.
/// Exported for testing.
pub async fn run_challenge_server_with_listener(
    listener: TcpListener,
    challenges: HashMap<String, String>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let challenges = Arc::new(challenges);
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = match result {
                    Ok(v) => v,
                    Err(e) => { error!("challenge server accept error: {e}"); continue; }
                };
                let challenges = challenges.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                        let challenges = challenges.clone();
                        async move {
                            let path = req.uri().path().to_string();
                            const PREFIX: &str = "/.well-known/acme-challenge/";
                            if let Some(token) = path.strip_prefix(PREFIX) {
                                if let Some(key_auth) = challenges.get(token) {
                                    return Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .header("content-type", "text/plain")
                                            .body(Full::new(Bytes::from(key_auth.clone())))
                                            .unwrap(),
                                    );
                                }
                            }
                            Ok(Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Full::new(Bytes::new()))
                                .unwrap())
                        }
                    });
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
            _ = &mut shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn challenge_server_responds_to_well_known_path() {
        let mut challenges = HashMap::new();
        challenges.insert(
            "test-token-123".to_string(),
            "test-token-123.keyauth456".to_string(),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(run_challenge_server_with_listener(
            listener,
            challenges,
            shutdown_rx,
        ));

        tokio::time::sleep(Duration::from_millis(50)).await;

        let resp = reqwest::get(format!(
            "http://127.0.0.1:{port}/.well-known/acme-challenge/test-token-123"
        ))
        .await
        .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "test-token-123.keyauth456");

        let _ = shutdown_tx.send(());
    }

    #[tokio::test]
    async fn challenge_server_returns_404_for_unknown_token() {
        let challenges = HashMap::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(run_challenge_server_with_listener(
            listener,
            challenges,
            shutdown_rx,
        ));

        tokio::time::sleep(Duration::from_millis(50)).await;

        let resp = reqwest::get(format!(
            "http://127.0.0.1:{port}/.well-known/acme-challenge/unknown"
        ))
        .await
        .unwrap();
        assert_eq!(resp.status(), 404);

        let _ = shutdown_tx.send(());
    }

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
}
