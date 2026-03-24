use prismproxy::config::Config;

#[test]
fn rejects_invalid_listen_address() {
    let toml = r#"
[server]
listen = "not-an-address"

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("listen"));
}

#[test]
fn rejects_invalid_upstream_address() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/api"
upstream = "not-valid"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("upstream"));
}

#[test]
fn rejects_empty_path_prefix() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = ""
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("path_prefix"));
}

#[test]
fn rejects_prefix_without_leading_slash() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "api"
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    let result = cfg.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("path_prefix"));
}

#[test]
fn accepts_valid_config() {
    let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert!(cfg.validate().is_ok());
}

#[test]
fn accepts_config_with_no_routes() {
    let toml = r#"
[server]
listen = "0.0.0.0:80"
"#;
    let cfg = Config::parse(toml).unwrap();
    assert!(cfg.validate().is_ok());
}
