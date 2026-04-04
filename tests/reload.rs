mod common;
use common::{MockUpstream, TestProxyHot};
use std::time::Duration;
use tokio::time::sleep;

const PASS_ALL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pass_all.wat");
const BLOCK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/block_path.wat");

fn config_toml(upstream: &str) -> String {
    format!(
        r#"[server]
listen = "127.0.0.1:0"

[[routes]]
path_prefix = "/"
upstream = "{upstream}"
"#
    )
}

fn config_toml_with_plugin(upstream: &str, plugin: &str) -> String {
    format!(
        r#"[server]
listen = "127.0.0.1:0"

[[routes]]
path_prefix = "/"
upstream = "{upstream}"
plugins = ["{plugin}"]
"#
    )
}

fn config_toml_two_routes(upstream_a: &str, upstream_b: &str) -> String {
    format!(
        r#"[server]
listen = "127.0.0.1:0"

[[routes]]
path_prefix = "/a"
upstream = "{upstream_a}"

[[routes]]
path_prefix = "/b"
upstream = "{upstream_b}"
"#
    )
}

#[tokio::test]
async fn reload_adds_new_route() {
    let upstream_a = MockUpstream::start(200, "route-a").await;
    let upstream_b = MockUpstream::start(200, "route-b").await;
    let addr_a = format!("127.0.0.1:{}", upstream_a.addr.port());
    let addr_b = format!("127.0.0.1:{}", upstream_b.addr.port());

    // Start with only /a route (no /b)
    let initial_toml = format!(
        r#"[server]
listen = "127.0.0.1:0"

[[routes]]
path_prefix = "/a"
upstream = "{addr_a}"
"#
    );
    let proxy = TestProxyHot::start(&initial_toml).await;
    sleep(Duration::from_millis(100)).await;

    // /b returns 404 before reload
    let resp = reqwest::get(proxy.url("/b/anything")).await.unwrap();
    assert_eq!(resp.status(), 404, "/b should be 404 before reload");

    // Write new config with both /a and /b
    proxy.write_config(&config_toml_two_routes(&addr_a, &addr_b));
    sleep(Duration::from_millis(1500)).await;

    // /b now returns 200
    let resp = reqwest::get(proxy.url("/b/anything")).await.unwrap();
    assert_eq!(resp.status(), 200, "/b should be 200 after reload");
}

#[tokio::test]
async fn reload_invalid_config_keeps_previous_state() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());

    let proxy = TestProxyHot::start(&config_toml(&addr)).await;
    sleep(Duration::from_millis(100)).await;

    // Verify the proxy works before breaking the config
    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Write invalid TOML
    proxy.write_config("this is not valid toml ][[[");
    sleep(Duration::from_millis(1500)).await;

    // Proxy still serves with old config
    let resp = reqwest::get(proxy.url("/anything")).await.unwrap();
    assert_eq!(resp.status(), 200, "should keep serving after invalid config written");
}

#[tokio::test]
async fn reload_changes_plugin_chain() {
    let upstream = MockUpstream::start(200, "ok").await;
    let addr = format!("127.0.0.1:{}", upstream.addr.port());

    // Start with pass_all plugin — /blocked should pass (200)
    let proxy = TestProxyHot::start(&config_toml_with_plugin(&addr, PASS_ALL)).await;
    sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(proxy.url("/blocked")).await.unwrap();
    assert_eq!(resp.status(), 200, "/blocked should pass with pass_all plugin");

    // Reload with block_path plugin
    proxy.write_config(&config_toml_with_plugin(&addr, BLOCK_PATH));
    sleep(Duration::from_millis(1500)).await;

    // /blocked should now be 403
    let resp = reqwest::get(proxy.url("/blocked")).await.unwrap();
    assert_eq!(resp.status(), 403, "/blocked should be 403 after reload with block_path plugin");
}
