use prismproxy::config::{Config, RouteConfig, ServerConfig};
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
