mod common;
use common::{test_config, MockUpstream, TestProxy};

#[tokio::test]
async fn proxies_to_upstream() {
    let upstream = MockUpstream::start(200, "hello from upstream").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "hello from upstream");
}

#[tokio::test]
async fn preserves_upstream_headers() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    assert_eq!(resp.headers().get("x-upstream").unwrap(), "mock");
}

#[tokio::test]
async fn returns_502_when_upstream_down() {
    let proxy = TestProxy::start(test_config(vec![("/", "127.0.0.1:1")])).await;
    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn handles_concurrent_requests() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let mut handles = vec![];
    for _ in 0..10 {
        let url = proxy.url("/test");
        handles.push(tokio::spawn(async move {
            reqwest::get(&url).await.unwrap()
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap().status(), 200);
    }
}
