use serde::Deserialize;
use std::path::Path;

use crate::error::ProxyError;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    #[serde(default = "default_max_idle")]
    pub max_idle_connections: usize,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
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
        // Validate listen address
        self.server
            .listen
            .parse::<std::net::SocketAddr>()
            .map_err(|e| {
                ProxyError::Config(format!(
                    "invalid listen address '{}': {e}",
                    self.server.listen
                ))
            })?;

        // Validate routes
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
}
