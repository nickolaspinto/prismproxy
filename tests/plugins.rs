mod common;
use common::{test_config, test_config_with_routes, test_route_with_plugin, MockUpstream};

const PASS_ALL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pass_all.wat");
const BLOCK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/block_path.wat");

#[tokio::test]
async fn route_with_pass_all_plugin_passes() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config_with_routes(vec![test_route_with_plugin(
        "/", &addr, PASS_ALL,
    )]))
    .await;

    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn route_with_block_path_plugin_returns_403() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config_with_routes(vec![test_route_with_plugin(
        "/", &addr, BLOCK_PATH,
    )]))
    .await;

    let resp = reqwest::get(proxy.url("/blocked")).await.unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn route_without_plugin_always_passes() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());
    let proxy = common::TestProxy::start(test_config(vec![("/", &addr)])).await;

    let resp = reqwest::get(proxy.url("/blocked")).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn two_routes_independent_plugin_chains() {
    // Route order matters: /pub is checked before / (top-to-bottom first match)
    // /pub route has pass_all: /pub/blocked → pass_all plugin → 200
    // /  route has block_path: /blocked → block_path plugin → 403
    let pub_upstream = MockUpstream::start(200, "pub-ok").await;
    let root_upstream = MockUpstream::start(200, "root-ok").await;
    let pub_addr = format!("127.0.0.1:{}", pub_upstream.addr.port());
    let root_addr = format!("127.0.0.1:{}", root_upstream.addr.port());

    let proxy = common::TestProxy::start(test_config_with_routes(vec![
        test_route_with_plugin("/pub", &pub_addr, PASS_ALL),
        test_route_with_plugin("/", &root_addr, BLOCK_PATH),
    ]))
    .await;

    // /pub/blocked → /pub route → pass_all plugin → 200
    let resp = reqwest::get(proxy.url("/pub/blocked")).await.unwrap();
    assert_eq!(resp.status(), 200);

    // /blocked → / route → block_path plugin → 403
    let resp = reqwest::get(proxy.url("/blocked")).await.unwrap();
    assert_eq!(resp.status(), 403);
}
