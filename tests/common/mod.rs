use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use prismproxy::config::{Config, RouteConfig, ServerConfig};
use std::convert::Infallible;
use std::net::SocketAddr;
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
            prismproxy::server::run_with_listener(
                listener,
                config,
                async { rx.await.ok(); },
            )
            .await
            .unwrap();
        });

        Self { addr, _shutdown: tx }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }
}

pub fn test_config(routes: Vec<(&str, &str)>) -> Config {
    Config {
        server: ServerConfig {
            listen: "127.0.0.1:0".to_string(),
            max_idle_connections: 2,
        },
        routes: routes
            .into_iter()
            .map(|(prefix, upstream)| RouteConfig {
                path_prefix: prefix.to_string(),
                upstream: upstream.to_string(),
            })
            .collect(),
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

        Self { addr, _shutdown: tx }
    }
}
