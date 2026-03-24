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
    let _config = Arc::new(config);
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result?;
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    if let Err(e) = http1::Builder::new()
                        .serve_connection(io, service_fn(handler::handle))
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
