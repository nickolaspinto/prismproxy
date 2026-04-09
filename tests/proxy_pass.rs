mod common;
use common::{
    test_config, test_config_with_timeout, EchoUpstream, HeaderEchoUpstream, MockUpstream,
    SlowUpstream, TestProxy,
};

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
        handles.push(tokio::spawn(
            async move { reqwest::get(&url).await.unwrap() },
        ));
    }
    for h in handles {
        assert_eq!(h.await.unwrap().status(), 200);
    }
}

#[tokio::test]
async fn returns_504_when_upstream_times_out() {
    let upstream = SlowUpstream::start(5000).await; // 5s delay
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    // Proxy with 200ms timeout — upstream will never respond in time
    let proxy = TestProxy::start(test_config_with_timeout(vec![("/", &addr)], 200)).await;

    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    assert_eq!(resp.status(), 504);
}

#[tokio::test]
async fn forwards_post_with_body() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(proxy.url("/submit"))
        .body("hello world")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "POST");
    assert_eq!(body["path"], "/submit");
    assert_eq!(body["body"], "hello world");
}

#[tokio::test]
async fn forwards_put_with_json_body() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(proxy.url("/resource/1"))
        .header("content-type", "application/json")
        .body(r#"{"name":"test"}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "PUT");
    assert_eq!(body["body"], r#"{"name":"test"}"#);
}

#[tokio::test]
async fn forwards_delete_request() {
    let upstream = EchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let client = reqwest::Client::new();
    let resp = client
        .delete(proxy.url("/resource/1"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "DELETE");
    assert_eq!(body["path"], "/resource/1");
}

#[tokio::test]
async fn adds_x_forwarded_for_header() {
    let upstream = HeaderEchoUpstream::start().await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/test")).await.unwrap();
    let headers: serde_json::Value = resp.json().await.unwrap();
    assert!(headers["x-forwarded-for"].as_str().is_some());
    assert!(headers["x-forwarded-proto"]
        .as_str()
        .unwrap()
        .contains("http"));
}

#[tokio::test]
async fn response_includes_x_response_time_header() {
    let upstream = MockUpstream::start(200, "ok").await;
    let proxy = TestProxy::start(test_config(vec![(
        "/",
        &format!("127.0.0.1:{}", upstream.addr.port()),
    )]))
    .await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert!(
        resp.headers().contains_key("x-response-time"),
        "x-response-time header should be present"
    );
    let val = resp.headers()["x-response-time"].to_str().unwrap();
    assert!(
        val.ends_with("ms"),
        "expected format like '5ms', got: {val}"
    );
}
