use std::path::Path;

use crate::config::{Config, RouteConfig};
use crate::error::ProxyError;
use crate::plugin::PluginRuntime;
use crate::tls::{build_tls_state, TlsState};

pub struct RouteState {
    pub route: RouteConfig,
    pub runtime: PluginRuntime,
}

pub struct AppState {
    pub timeout_ms: u64,
    pub routes: Vec<RouteState>,
    pub tls: Option<TlsState>,
}

/// Build AppState from a Config.
///
/// If `config.tls` is Some and `{cache_dir}/cert.pem` + `{cache_dir}/key.pem` both exist,
/// builds TlsState from the cached cert files. If files are missing, returns `tls: None`
/// (the caller is responsible for provisioning via ACME before calling again).
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

    if routes.is_empty() {
        tracing::warn!("no routes configured — all requests will return 404");
    }

    let tls = if let Some(ref tls_config) = config.tls {
        let cert_path = Path::new(&tls_config.cache_dir).join("cert.pem");
        let key_path = Path::new(&tls_config.cache_dir).join("key.pem");
        if cert_path.exists() && key_path.exists() {
            let cert_pem = std::fs::read_to_string(&cert_path)
                .map_err(|e| ProxyError::Tls(format!("read cert.pem: {e}")))?;
            let key_pem = std::fs::read_to_string(&key_path)
                .map_err(|e| ProxyError::Tls(format!("read key.pem: {e}")))?;
            Some(build_tls_state(tls_config, &cert_pem, &key_pem)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(AppState {
        timeout_ms,
        routes,
        tls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ServerConfig, TlsConfig};

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
        assert!(state.tls.is_none());
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

    #[test]
    fn build_state_without_tls_config_has_no_tls() {
        let config = Config {
            server: server_config(),
            routes: vec![],
            tls: None,
        };
        let state = build_state(config).unwrap();
        assert!(state.tls.is_none());
    }

    #[test]
    fn build_state_with_tls_but_no_cert_files_returns_no_tls() {
        let config = Config {
            server: ServerConfig {
                http_challenge_listen: Some("127.0.0.1:8081".to_string()),
                ..server_config()
            },
            routes: vec![],
            tls: Some(TlsConfig {
                acme_email: "test@example.com".to_string(),
                acme_directory: "https://acme-staging-v02.api.letsencrypt.org/directory"
                    .to_string(),
                cache_dir: "/nonexistent/path/that/does/not/exist".to_string(),
                domains: vec!["example.com".to_string()],
            }),
        };
        let state = build_state(config).unwrap();
        // cert files don't exist → tls: None, no error
        assert!(state.tls.is_none());
    }
}
