use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProxyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Hyper(#[from] hyper::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("no route for path: {0}")]
    NoRoute(String),

    #[error("upstream: {0}")]
    UpstreamConnect(String),

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("plugin: {0}")]
    Plugin(String),
}
