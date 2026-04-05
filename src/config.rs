use serde::Deserialize;
use std::path::Path;

use crate::error::ProxyError;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    #[serde(default = "default_max_idle")]
    pub max_idle_connections: usize,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub http_challenge_listen: Option<String>,
}

fn default_max_idle() -> usize {
    10
}

fn default_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub path_prefix: String,
    pub upstream: String,
    #[serde(default)]
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub acme_email: String,
    #[serde(default = "default_acme_directory")]
    pub acme_directory: String,
    pub cache_dir: String,
    pub domains: Vec<String>,
}

fn default_acme_directory() -> String {
    "https://acme-v02.api.letsencrypt.org/directory".to_string()
}

impl Config {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ProxyError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ProxyError::Config(format!("read: {e}")))?;
        Self::parse(&content)
    }

    pub fn parse(s: &str) -> Result<Self, ProxyError> {
        toml::from_str(s).map_err(|e| ProxyError::Config(format!("parse: {e}")))
    }

    pub fn validate(&self) -> Result<(), ProxyError> {
        self.server
            .listen
            .parse::<std::net::SocketAddr>()
            .map_err(|e| {
                ProxyError::Config(format!(
                    "invalid listen address '{}': {e}",
                    self.server.listen
                ))
            })?;

        if self.tls.is_some() && self.server.http_challenge_listen.is_none() {
            return Err(ProxyError::Config(
                "http_challenge_listen is required when [tls] is configured".to_string(),
            ));
        }

        for (i, route) in self.routes.iter().enumerate() {
            if route.path_prefix.is_empty() || !route.path_prefix.starts_with('/') {
                return Err(ProxyError::Config(format!(
                    "route[{i}]: path_prefix must start with '/', got '{}'",
                    route.path_prefix
                )));
            }
            route
                .upstream
                .parse::<std::net::SocketAddr>()
                .map_err(|e| {
                    ProxyError::Config(format!(
                        "route[{i}]: invalid upstream address '{}': {e}",
                        route.upstream
                    ))
                })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_config() {
        let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:8000"
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.server.listen, "127.0.0.1:8080");
        assert_eq!(cfg.server.max_idle_connections, 10);
        assert_eq!(cfg.routes.len(), 2);
        assert_eq!(cfg.routes[0].path_prefix, "/api");
    }

    #[test]
    fn missing_server_fails() {
        let toml = "[[routes]]\npath_prefix = \"/\"\nupstream = \"x\"";
        assert!(Config::parse(toml).is_err());
    }

    #[test]
    fn empty_routes_ok() {
        let toml = "[server]\nlisten = \"0.0.0.0:80\"";
        let cfg = Config::parse(toml).unwrap();
        assert!(cfg.routes.is_empty());
    }

    #[test]
    fn default_max_idle() {
        let toml = "[server]\nlisten = \"0.0.0.0:80\"";
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.server.max_idle_connections, 10);
    }

    #[test]
    fn route_with_plugins_parses() {
        let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/api"
upstream = "127.0.0.1:3000"
plugins = ["./plugins/auth.wasm", "./plugins/rate.wasm"]
"#;
        let cfg = Config::parse(toml).unwrap();
        assert_eq!(cfg.routes[0].plugins.len(), 2);
        assert_eq!(cfg.routes[0].plugins[0], "./plugins/auth.wasm");
    }

    #[test]
    fn route_without_plugins_defaults_empty() {
        let toml = r#"
[server]
listen = "127.0.0.1:8080"

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:8000"
"#;
        let cfg = Config::parse(toml).unwrap();
        assert!(cfg.routes[0].plugins.is_empty());
    }

    #[test]
    fn tls_config_parses() {
        let toml = r#"
[server]
listen = "0.0.0.0:443"
http_challenge_listen = "0.0.0.0:80"

[tls]
acme_email = "admin@example.com"
cache_dir = "./certs"
domains = ["example.com", "www.example.com"]

[[routes]]
path_prefix = "/"
upstream = "127.0.0.1:3000"
"#;
        let cfg = Config::parse(toml).unwrap();
        let tls = cfg.tls.unwrap();
        assert_eq!(tls.acme_email, "admin@example.com");
        assert_eq!(tls.domains.len(), 2);
        assert_eq!(
            tls.acme_directory,
            "https://acme-v02.api.letsencrypt.org/directory"
        );
        assert_eq!(cfg.server.http_challenge_listen.unwrap(), "0.0.0.0:80");
    }

    #[test]
    fn missing_tls_defaults_none() {
        let toml = "[server]\nlisten = \"0.0.0.0:80\"";
        let cfg = Config::parse(toml).unwrap();
        assert!(cfg.tls.is_none());
        assert!(cfg.server.http_challenge_listen.is_none());
    }
}
