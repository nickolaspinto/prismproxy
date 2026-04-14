mod common;
use common::{test_config, MockUpstream, TestProxy};

#[tokio::test]
async fn health_returns_200_with_status_ok() {
    let proxy = TestProxy::start(test_config(vec![])).await;
    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn health_has_json_content_type() {
    let proxy = TestProxy::start(test_config(vec![])).await;
    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn health_includes_version_and_uptime() {
    let proxy = TestProxy::start(test_config(vec![])).await;
    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], "0.1.0");
    assert!(body["uptime_secs"].as_f64().unwrap() >= 0.0);
}

// ── Upstream health failover tests ────────────────────────────────────────────

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
    use prismproxy::config::RouteConfig;
    use prismproxy::plugin::PluginRuntime;
    use prismproxy::state::{AppState, RouteState};
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    let upstream = MockUpstream::start(200, "ok").await;
    let upstream_addr = format!("127.0.0.1:{}", upstream.addr.port());
    let healthy = Arc::new(AtomicBool::new(false)); // start unhealthy

    let app_state = Arc::new(ArcSwap::from_pointee(AppState {
        timeout_ms: 5000,
        routes: vec![RouteState {
            route: RouteConfig {
                path_prefix: "/".to_string(),
                upstream: upstream_addr.clone(),
                plugins: vec![],
            },
            runtime: PluginRuntime::new().unwrap(),
            healthy: healthy.clone(),
        }],
        tls: None,
    }));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        prismproxy::server::run_with_app_state(app_state, listener, async {
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
