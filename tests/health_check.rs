mod common;
use common::{test_config, TestProxy};

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
