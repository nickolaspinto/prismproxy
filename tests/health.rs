mod common;
use common::{test_config, MockUpstream, TestProxy};

#[tokio::test]
async fn health_returns_status_ok_with_version_and_uptime() {
    let upstream = MockUpstream::start(200, "up").await;
    let proxy = TestProxy::start(test_config(vec![(
        "/",
        &format!("127.0.0.1:{}", upstream.addr.port()),
    )]))
    .await;

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
    let proxy = TestProxy::start(test_config(vec![(
        "/",
        &format!("127.0.0.1:{}", upstream.addr.port()),
    )]))
    .await;

    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
}
