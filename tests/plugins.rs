mod common;
use common::{test_config, test_config_with_plugin, MockUpstream};

const PASS_ALL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pass_all.wat");
const BLOCK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/block_path.wat");

#[tokio::test]
async fn pass_all_plugin_does_not_block() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config_with_plugin(vec![("/", &addr)], PASS_ALL)).await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn block_path_plugin_returns_403_for_blocked_path() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config_with_plugin(vec![("/", &addr)], BLOCK_PATH)).await;

    let resp = reqwest::get(proxy.url("/blocked")).await.unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn block_path_plugin_passes_allowed_path() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config_with_plugin(vec![("/", &addr)], BLOCK_PATH)).await;

    let resp = reqwest::get(proxy.url("/allowed")).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn block_all_plugin_returns_403() {
    // Uses block_path.wat which blocks any path starting with "/block".
    // "/blockeverything" starts with "/block" so it will be blocked.
    // This test verifies that plugin invocation actually happens — if the
    // plugin wiring were removed, this test would fail (200 instead of 403).
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config_with_plugin(vec![("/", &addr)], BLOCK_PATH)).await;

    let resp = reqwest::get(proxy.url("/blockeverything")).await.unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn no_plugins_passes_all_requests() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
}
