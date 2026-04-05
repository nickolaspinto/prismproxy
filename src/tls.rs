use std::sync::Arc;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

use crate::config::TlsConfig;
use crate::error::ProxyError;

pub struct TlsState {
    pub acceptor: TlsAcceptor,
    pub domains: Vec<String>,
}

/// Build a TlsAcceptor from PEM-encoded certificate chain and private key.
/// Configures ALPN to negotiate HTTP/2 then HTTP/1.1.
pub fn build_tls_state(
    tls_config: &TlsConfig,
    cert_pem: &str,
    key_pem: &str,
) -> Result<TlsState, ProxyError> {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ProxyError::Tls(format!("parse cert chain: {e}")))?;

    if certs.is_empty() {
        return Err(ProxyError::Tls("no certificates found in PEM".to_string()));
    }

    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .map_err(|e| ProxyError::Tls(format!("parse private key: {e}")))?
        .ok_or_else(|| ProxyError::Tls("no private key found in PEM".to_string()))?;

    let mut server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ProxyError::Tls(format!("build server config: {e}")))?;

    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(TlsState {
        acceptor: TlsAcceptor::from(Arc::new(server_config)),
        domains: tls_config.domains.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TlsConfig;

    fn test_tls_config() -> TlsConfig {
        TlsConfig {
            acme_email: "test@example.com".to_string(),
            acme_directory: "https://acme-staging-v02.api.letsencrypt.org/directory".to_string(),
            cache_dir: "./certs".to_string(),
            domains: vec!["localhost".to_string()],
        }
    }

    fn generate_self_signed() -> (String, String) {
        let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        let key_pair = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        (cert.pem(), key_pair.serialize_pem())
    }

    #[test]
    fn build_tls_state_with_valid_cert_succeeds() {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let (cert_pem, key_pem) = generate_self_signed();
        let result = build_tls_state(&test_tls_config(), &cert_pem, &key_pem);
        assert!(result.is_ok());
        let state = result.unwrap();
        assert_eq!(state.domains, vec!["localhost"]);
    }

    #[test]
    fn build_tls_state_with_invalid_pem_fails() {
        let result = build_tls_state(&test_tls_config(), "not-a-cert", "not-a-key");
        assert!(result.is_err());
        let err = result.err().expect("expected error");
        assert!(err.to_string().contains("tls:"));
    }
}
