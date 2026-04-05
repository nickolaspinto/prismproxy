// Stub — full implementation in Task 7
use crate::config::TlsConfig;
use crate::error::ProxyError;

pub async fn provision(
    _tls_config: &TlsConfig,
    _challenge_listen: &str,
) -> Result<(String, String), ProxyError> {
    unimplemented!("acme::provision not yet implemented")
}
