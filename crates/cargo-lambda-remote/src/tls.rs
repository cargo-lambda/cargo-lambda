use std::path::{Path, PathBuf};

use clap::Args;
use miette::{Diagnostic, Result};
use rustls::{crypto::aws_lc_rs, ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use thiserror::Error;

#[derive(Debug, Diagnostic, Error)]
pub enum TlsError {
    #[error("missing TLS certificate")]
    #[diagnostic()]
    MissingTlsCert,

    #[error("missing TLS key")]
    #[diagnostic()]
    MissingTlsKey,

    #[error("invalid TLS file: {0}, {1}")]
    #[diagnostic()]
    InvalidTlsFile(PathBuf, rustls_pki_types::pem::Error),

    #[error("failed to parse TLS key: {0}")]
    #[diagnostic()]
    FailedToParseTlsKey(String),

    #[error("failed to parse config: {0}")]
    #[diagnostic()]
    FailedToParseConfig(#[from] rustls::Error),
}

#[derive(Args, Clone, Debug, Default)]
pub struct TlsOptions {
    /// Path to a TLS certificate file
    #[arg(long, conflicts_with = "remote")]
    pub tls_cert: Option<PathBuf>,
    /// Path to a TLS key file
    #[arg(long, conflicts_with = "remote")]
    pub tls_key: Option<PathBuf>,
    /// Path to a TLS CA file
    #[arg(long, conflicts_with = "remote")]
    pub tls_ca: Option<PathBuf>,
}

impl TlsOptions {
    pub fn is_secure(&self) -> bool {
        self.tls_cert.is_some() && self.tls_key.is_some()
    }

    pub async fn server_config(&self) -> Result<Option<ServerConfig>> {
        if !self.is_secure() {
            return Ok(None);
        }

        aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install the default TLS provider");

        let (mut cert_chain, key) =
            parse_cert_and_key(self.tls_cert.as_ref(), self.tls_key.as_ref())?;

        if let Some(path) = &self.tls_ca {
            let certs = parse_certificates(path)?;
            if !certs.is_empty() {
                cert_chain.extend(certs);
            }
        }

        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, key)
            .map_err(TlsError::FailedToParseConfig)?;

        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        Ok(Some(config))
    }

    pub async fn client_config(&self) -> Result<ClientConfig> {
        aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install the default TLS provider");

        let builder = if let Some(path) = &self.tls_ca {
            let mut root_store = RootCertStore::empty();
            root_store.add_parsable_certificates(parse_certificates(path)?);
            ClientConfig::builder().with_root_certificates(root_store)
        } else {
            use rustls_platform_verifier::BuilderVerifierExt;
            ClientConfig::builder().with_platform_verifier()
        };

        let (cert, key) = parse_cert_and_key(self.tls_cert.as_ref(), self.tls_key.as_ref())?;

        let config = builder
            .with_client_auth_cert(cert, key)
            .map_err(TlsError::FailedToParseConfig)?;

        Ok(config)
    }
}

fn parse_certificates<P: AsRef<Path>>(path: P) -> Result<Vec<CertificateDer<'static>>> {
    let path = path.as_ref();
    let parser = CertificateDer::pem_file_iter(path)
        .map_err(|e| TlsError::InvalidTlsFile(path.to_path_buf(), e))?
        .collect::<Vec<_>>();

    let mut certs = Vec::with_capacity(parser.len());
    for cert in parser {
        certs.push(cert.map_err(|e| TlsError::InvalidTlsFile(path.to_path_buf(), e))?);
    }

    Ok(certs)
}

fn parse_cert_and_key(
    cert: Option<&PathBuf>,
    key: Option<&PathBuf>,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let path = cert.ok_or(TlsError::MissingTlsCert)?;
    let cert = parse_certificates(path)?;

    let path = key.ok_or(TlsError::MissingTlsKey)?;
    let key = PrivateKeyDer::from_pem_file(path)
        .map_err(|e| TlsError::FailedToParseTlsKey(e.to_string()))?;

    Ok((cert, key))
}
