mod common;
use common::{test_config, MockUpstream, TestProxy, TestProxyTls};
use prismproxy::acme::ChallengeStore;

fn install_crypto_provider() {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
}

fn tls_client() -> reqwest::Client {
    install_crypto_provider();
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .use_rustls_tls()
        .build()
        .unwrap()
}

fn tls_client_http1_only() -> reqwest::Client {
    install_crypto_provider();
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .use_rustls_tls()
        .http1_only()
        .build()
        .unwrap()
}

#[tokio::test]
async fn https_request_returns_200() {
    let proxy = TestProxyTls::start().await;
    let resp = tls_client()
        .get(proxy.url("/anything"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn http2_negotiated_via_alpn() {
    let proxy = TestProxyTls::start().await;
    let resp = tls_client()
        .get(proxy.url("/anything"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.version(), reqwest::Version::HTTP_2);
}

#[tokio::test]
async fn http1_also_works_over_tls() {
    let proxy = TestProxyTls::start().await;
    let resp = tls_client_http1_only()
        .get(proxy.url("/anything"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.version(), reqwest::Version::HTTP_11);
}

#[tokio::test]
async fn x_forwarded_proto_is_https_via_header_echo() {
    use prismproxy::config::{Config, RouteConfig, ServerConfig, TlsConfig};
    use tokio::net::TcpListener;

    let upstream = common::HeaderEchoUpstream::start().await;
    let upstream_addr = format!("127.0.0.1:{}", upstream.addr.port());

    let dir = tempfile::TempDir::new().unwrap();
    let (cert_pem, key_pem) = common::generate_self_signed_cert();
    std::fs::write(dir.path().join("cert.pem"), &cert_pem).unwrap();
    std::fs::write(dir.path().join("key.pem"), &key_pem).unwrap();

    let config = Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms: 5000,
            http_challenge_listen: None,
        },
        routes: vec![RouteConfig {
            path_prefix: "/".to_string(),
            upstream: upstream_addr,
            plugins: vec![],
        }],
        tls: Some(TlsConfig {
            acme_email: "test@example.com".to_string(),
            acme_directory: "https://acme-staging-v02.api.letsencrypt.org/directory".to_string(),
            cache_dir: dir.path().to_str().unwrap().to_string(),
            domains: vec!["localhost".to_string()],
        }),
    };

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

    let resp = tls_client()
        .get(format!("https://127.0.0.1:{}/", addr.port()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["x-forwarded-proto"], "https");

    let _ = tx.send(());
}

#[tokio::test]
async fn plain_http_mode_unaffected() {
    let upstream = MockUpstream::start(200, "plain-ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.version(), reqwest::Version::HTTP_11);
}

#[tokio::test]
async fn http_redirect_server_returns_301_to_https() {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let challenges: ChallengeStore = Arc::new(RwLock::new(HashMap::new()));

    tokio::spawn(prismproxy::server::run_http_redirect_server_with_listener(
        listener, 443, challenges,
    ));

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Disable redirect-following to inspect the 301
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let resp = client
        .get(format!("http://127.0.0.1:{port}/some/path"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 301);
    let loc = resp.headers()["location"].to_str().unwrap();
    assert!(loc.starts_with("https://"), "location was: {loc}");
    assert!(loc.contains("/some/path"), "location was: {loc}");
}

#[tokio::test]
async fn http_redirect_server_serves_acme_challenge() {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let challenges: ChallengeStore = Arc::new(RwLock::new(HashMap::new()));
    challenges
        .write()
        .unwrap()
        .insert("mytoken".to_string(), "mytoken.keyauth".to_string());

    tokio::spawn(prismproxy::server::run_http_redirect_server_with_listener(
        listener, 443, challenges,
    ));

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let resp = reqwest::get(format!(
        "http://127.0.0.1:{port}/.well-known/acme-challenge/mytoken"
    ))
    .await
    .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "mytoken.keyauth");
}
