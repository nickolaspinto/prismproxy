use crate::config::{Config, RouteConfig};
use crate::error::ProxyError;
use crate::plugin::PluginRuntime;

pub struct RouteState {
    pub route: RouteConfig,
    pub runtime: PluginRuntime,
}

pub struct AppState {
    pub timeout_ms: u64,
    pub routes: Vec<RouteState>,
}

pub fn build_state(config: Config) -> Result<AppState, ProxyError> {
    let timeout_ms = config.server.timeout_ms;
    let mut routes = Vec::with_capacity(config.routes.len());
    for route in config.routes {
        let mut runtime = PluginRuntime::new()?;
        for path in &route.plugins {
            runtime.load(path)?;
        }
        routes.push(RouteState { route, runtime });
    }
    Ok(AppState { timeout_ms, routes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    fn server_config() -> ServerConfig {
        ServerConfig {
            listen: "127.0.0.1:8080".to_string(),
            max_idle_connections: 10,
            timeout_ms: 5000,
            http_challenge_listen: None,
        }
    }

    #[test]
    fn build_state_with_no_plugins_succeeds() {
        let config = Config {
            server: server_config(),
            routes: vec![
                RouteConfig {
                    path_prefix: "/api".to_string(),
                    upstream: "127.0.0.1:3000".to_string(),
                    plugins: vec![],
                },
                RouteConfig {
                    path_prefix: "/".to_string(),
                    upstream: "127.0.0.1:8000".to_string(),
                    plugins: vec![],
                },
            ],
            tls: None,
        };
        let state = build_state(config).unwrap();
        assert_eq!(state.routes.len(), 2);
        assert_eq!(state.routes[0].route.path_prefix, "/api");
        assert_eq!(state.timeout_ms, 5000);
    }

    #[test]
    fn build_state_with_missing_plugin_fails() {
        let config = Config {
            server: server_config(),
            routes: vec![RouteConfig {
                path_prefix: "/api".to_string(),
                upstream: "127.0.0.1:3000".to_string(),
                plugins: vec!["nonexistent.wasm".to_string()],
            }],
            tls: None,
        };
        let result = build_state(config);
        assert!(result.is_err());
    }
}
