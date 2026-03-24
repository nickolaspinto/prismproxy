use prismproxy::config;
use prismproxy::server;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("prismproxy=info".parse()?))
        .json()
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/default.toml".to_string());

    let config = config::Config::from_file(&config_path)?;
    tracing::info!(listen = %config.server.listen, routes = config.routes.len(), "loaded config");

    server::run(config).await?;
    Ok(())
}
