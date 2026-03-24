mod common;
use common::{test_config, MockUpstream, TestProxy};

#[tokio::test]
async fn routes_by_prefix() {
    let api = MockUpstream::start(200, "api").await;
    let web = MockUpstream::start(200, "web").await;
    let api_addr = format!("127.0.0.1:{}", api.addr.port());
    let web_addr = format!("127.0.0.1:{}", web.addr.port());

    let proxy = TestProxy::start(test_config(vec![
        ("/api", &api_addr),
        ("/", &web_addr),
    ])).await;

    let resp = reqwest::get(proxy.url("/api/users")).await.unwrap();
    assert_eq!(resp.text().await.unwrap(), "api");

    let resp = reqwest::get(proxy.url("/index.html")).await.unwrap();
    assert_eq!(resp.text().await.unwrap(), "web");
}

#[tokio::test]
async fn first_match_wins() {
    let specific = MockUpstream::start(200, "specific").await;
    let general = MockUpstream::start(200, "general").await;
    let s_addr = format!("127.0.0.1:{}", specific.addr.port());
    let g_addr = format!("127.0.0.1:{}", general.addr.port());

    let proxy = TestProxy::start(test_config(vec![
        ("/api/v2", &s_addr),
        ("/api", &g_addr),
    ])).await;

    let r = reqwest::get(proxy.url("/api/v2/x")).await.unwrap();
    assert_eq!(r.text().await.unwrap(), "specific");

    let r = reqwest::get(proxy.url("/api/v1/x")).await.unwrap();
    assert_eq!(r.text().await.unwrap(), "general");
}

#[tokio::test]
async fn no_match_returns_404() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/api", &addr)])).await;

    let resp = reqwest::get(proxy.url("/other")).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn health_bypasses_routing() {
    // Even with no routes, /health works
    let proxy = TestProxy::start(test_config(vec![])).await;
    let resp = reqwest::get(proxy.url("/health")).await.unwrap();
    assert_eq!(resp.status(), 200);
}
