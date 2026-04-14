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

    for _ in 0..3 {
        reqwest::get(proxy.url("/anything")).await.unwrap();
    }

    let body = reqwest::get(proxy.url("/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(body.contains("prismproxy_requests_total"));
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

#[tokio::test]
async fn metrics_has_all_required_prometheus_fields() {
    let proxy = TestProxy::start(test_config(vec![])).await;
    let body = reqwest::get(proxy.url("/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    for field in &[
        "prismproxy_requests_total",
        "prismproxy_requests_2xx",
        "prismproxy_requests_4xx",
        "prismproxy_requests_5xx",
        "prismproxy_requests_blocked",
        "prismproxy_response_time_ms_mean",
    ] {
        assert!(body.contains(field), "missing field: {field}");
    }
}
