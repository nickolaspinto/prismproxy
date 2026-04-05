#![allow(dead_code)]

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use prismproxy::config::{Config, RouteConfig, ServerConfig, TlsConfig};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::net::TcpListener;

pub struct TestProxy {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl TestProxy {
    pub async fn start(config: Config) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            prismproxy::server::run_with_listener(listener, config, async {
                rx.await.ok();
            })
            .await
            .unwrap();
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

/// Proxy that watches a config file on disk and hot-reloads on change.
pub struct TestProxyHot {
    pub addr: SocketAddr,
    pub config_path: PathBuf,
    _shutdown: tokio::sync::oneshot::Sender<()>,
    _dir: tempfile::TempDir,
}

impl TestProxyHot {
    pub async fn start(initial_toml: &str) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, initial_toml).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let path = config_path.clone();

        tokio::spawn(async move {
            prismproxy::server::run_with_listener_hot(listener, path, async {
                rx.await.ok();
            })
            .await
            .unwrap();
        });

        Self {
            addr,
            config_path,
            _shutdown: tx,
            _dir: dir,
        }
    }

    /// Overwrite the config file on disk. The proxy detects the change and reloads within ~1s.
    pub fn write_config(&self, toml: &str) {
        std::fs::write(&self.config_path, toml).unwrap();
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

/// Generate a self-signed cert + key PEM valid for 127.0.0.1 and localhost.
pub fn generate_self_signed_cert() -> (String, String) {
    let params =
        rcgen::CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()])
            .unwrap();
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key_pair).unwrap();
    (cert.pem(), key_pair.serialize_pem())
}

/// Proxy with TLS enabled, using a pre-generated self-signed certificate.
/// Uses `run_with_listener` (no hot reload) for test simplicity.
/// The upstream responds with 200 "tls-ok".
pub struct TestProxyTls {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
    _dir: tempfile::TempDir,
}

impl TestProxyTls {
    pub async fn start() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let cache_dir = dir.path().to_str().unwrap().to_string();

        let (cert_pem, key_pem) = generate_self_signed_cert();
        std::fs::write(dir.path().join("cert.pem"), &cert_pem).unwrap();
        std::fs::write(dir.path().join("key.pem"), &key_pem).unwrap();

        // Start a mock upstream
        let upstream = MockUpstream::start(200, "tls-ok").await;
        let upstream_addr = format!("127.0.0.1:{}", upstream.addr.port());

        let config = Config {
            server: ServerConfig {
                listen: "127.0.0.1:0".to_string(),
                max_idle_connections: 2,
                timeout_ms: 5000,
                http_challenge_listen: None,
            },
            routes: vec![RouteConfig {
                path_prefix: "/".to_string(),
                upstream: upstream_addr,
                plugins: vec![],
            }],
            tls: Some(TlsConfig {
                acme_email: "test@example.com".to_string(),
                acme_directory: "https://acme-staging-v02.api.letsencrypt.org/directory"
                    .to_string(),
                cache_dir,
                domains: vec!["localhost".to_string()],
            }),
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            prismproxy::server::run_with_listener(listener, config, async {
                rx.await.ok();
            })
            .await
            .unwrap();
            // Keep upstream alive for the test duration
            drop(upstream);
        });

        Self {
            addr,
            _shutdown: tx,
            _dir: dir,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.addr.port(), path)
    }
}

/// Build a Config with no plugins on any route.
pub fn test_config(routes: Vec<(&str, &str)>) -> Config {
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms: 5000,
            http_challenge_listen: None,
        },
        routes: routes
            .into_iter()
            .map(|(prefix, upstream)| RouteConfig {
                path_prefix: prefix.to_string(),
                upstream: upstream.to_string(),
                plugins: vec![],
            })
            .collect(),
        tls: None,
    }
}

pub fn test_config_with_timeout(routes: Vec<(&str, &str)>, timeout_ms: u64) -> Config {
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms,
            http_challenge_listen: None,
        },
        routes: routes
            .into_iter()
            .map(|(prefix, upstream)| RouteConfig {
                path_prefix: prefix.to_string(),
                upstream: upstream.to_string(),
                plugins: vec![],
            })
            .collect(),
        tls: None,
    }
}

/// Build a RouteConfig with a specific plugin (for per-route plugin tests).
pub fn test_route_with_plugin(prefix: &str, upstream: &str, plugin_path: &str) -> RouteConfig {
    RouteConfig {
        path_prefix: prefix.to_string(),
        upstream: upstream.to_string(),
        plugins: vec![plugin_path.to_string()],
    }
}

/// Build a Config from specific RouteConfigs (allows mixing routes with/without plugins).
pub fn test_config_with_routes(routes: Vec<RouteConfig>) -> Config {
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
            timeout_ms: 5000,
            http_challenge_listen: None,
        },
        routes,
        tls: None,
    }
}

pub struct MockUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl MockUpstream {
    pub async fn start(status: u16, body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let status = StatusCode::from_u16(status).unwrap();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            http1::Builder::new()
                                .serve_connection(io, service_fn(move |_req| async move {
                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(status)
                                            .header("x-upstream", "mock")
                                            .body(Full::new(Bytes::from(body)))
                                            .unwrap(),
                                    )
                                }))
                                .await
                                .ok();
                        });
                    }
                    _ = &mut rx => break,
                }
            }
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }
}

pub struct SlowUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl SlowUpstream {
    pub async fn start(delay_ms: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            http1::Builder::new()
                                .serve_connection(io, service_fn(move |_req| async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .body(Full::new(Bytes::from("slow")))
                                            .unwrap(),
                                    )
                                }))
                                .await
                                .ok();
                        });
                    }
                    _ = &mut rx => break,
                }
            }
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }
}

pub struct EchoUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl EchoUpstream {
    pub async fn start() -> Self {
        use http_body_util::BodyExt;
        use hyper::body::Incoming;
        use hyper::Request;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            http1::Builder::new()
                                .serve_connection(io, service_fn(|req: Request<Incoming>| async move {
                                    let method = req.method().to_string();
                                    let path = req.uri().path().to_string();
                                    let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
                                    let body_str = String::from_utf8_lossy(&body_bytes).to_string();
                                    let escaped_body = body_str.replace('\\', "\\\\").replace('"', "\\\"");

                                    let json = format!(
                                        r#"{{"method":"{}","path":"{}","body":"{}"}}"#,
                                        method, path, escaped_body
                                    );

                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .header("content-type", "application/json")
                                            .body(Full::new(Bytes::from(json)))
                                            .unwrap(),
                                    )
                                }))
                                .await
                                .ok();
                        });
                    }
                    _ = &mut rx => break,
                }
            }
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }
}

pub struct HeaderEchoUpstream {
    pub addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl HeaderEchoUpstream {
    pub async fn start() -> Self {
        use hyper::body::Incoming;
        use hyper::Request;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            http1::Builder::new()
                                .serve_connection(io, service_fn(|req: Request<Incoming>| async move {
                                    let mut headers_json = String::from("{");
                                    for (name, value) in req.headers() {
                                        if headers_json.len() > 1 {
                                            headers_json.push(',');
                                        }
                                        headers_json.push_str(&format!(
                                            "\"{}\":\"{}\"",
                                            name.as_str(),
                                            value.to_str().unwrap_or("")
                                        ));
                                    }
                                    headers_json.push('}');

                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .header("content-type", "application/json")
                                            .body(Full::new(Bytes::from(headers_json)))
                                            .unwrap(),
                                    )
                                }))
                                .await
                                .ok();
                        });
                    }
                    _ = &mut rx => break,
                }
            }
        });

        Self {
            addr,
            _shutdown: tx,
        }
    }
}
