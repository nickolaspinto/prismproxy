use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::config::Config;
use crate::error::ProxyError;
use crate::handler;
use crate::pool::ConnectionPool;

pub async fn run(config: Config) -> Result<(), ProxyError> {
    let listener = TcpListener::bind(&config.server.listen).await?;
    info!("listening on {}", config.server.listen);
    run_with_listener(listener, config, std::future::pending::<()>()).await
}

pub async fn run_with_listener(
    listener: TcpListener,
    config: Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProxyError> {
    let pool = Arc::new(ConnectionPool::new(config.server.max_idle_connections));
    let config = Arc::new(config);
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result?;
                let config = config.clone();
                let pool = pool.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc = service_fn(move |req| {
                        let config = config.clone();
                        let pool = pool.clone();
                        async move { handler::handle(config, pool, req).await }
                    });
                    if let Err(e) = http1::Builder::new()
                        .serve_connection(io, svc)
                        .await
                    {
                        error!(%addr, "connection error: {e}");
                    }
                });
            }
            _ = &mut shutdown => {
                info!("shutting down");
                return Ok(());
            }
        }
    }
}
