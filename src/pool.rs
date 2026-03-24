use bytes::Bytes;
use http_body_util::Full;
use hyper::client::conn::http1::SendRequest;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::error;

use crate::error::ProxyError;

pub struct ConnectionPool {
    idle: Mutex<HashMap<String, Vec<SendRequest<Full<Bytes>>>>>,
    max_idle_per_host: usize,
}

impl ConnectionPool {
    pub fn new(max_idle_per_host: usize) -> Self {
        Self {
            idle: Mutex::new(HashMap::new()),
            max_idle_per_host,
        }
    }

    pub async fn acquire(&self, addr: &str) -> Result<SendRequest<Full<Bytes>>, ProxyError> {
        {
            let mut idle = self.idle.lock().await;
            if let Some(conns) = idle.get_mut(addr) {
                if let Some(sender) = conns.pop() {
                    return Ok(sender);
                }
            }
        }
        self.connect(addr).await
    }

    pub async fn release(&self, addr: &str, sender: SendRequest<Full<Bytes>>) {
        let mut idle = self.idle.lock().await;
        let conns = idle.entry(addr.to_string()).or_default();
        if conns.len() < self.max_idle_per_host {
            conns.push(sender);
        }
    }

    async fn connect(&self, addr: &str) -> Result<SendRequest<Full<Bytes>>, ProxyError> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| ProxyError::UpstreamConnect(format!("{addr}: {e}")))?;
        let io = TokioIo::new(stream);
        let (sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(ProxyError::Hyper)?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                error!("pool connection error: {e}");
            }
        });
        Ok(sender)
    }
}
