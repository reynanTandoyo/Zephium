//! Byte streams to IMAP servers. Production connects over TCP and secures
//! the stream with rustls (system trust store, never OpenSSL, never plain
//! text); tests hand the engine in-memory streams through the same trait.

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};

use futures::future::BoxFuture;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::error::{MailError, Result};
use crate::limits::CONNECT_TIMEOUT;
use crate::model::{Endpoint, Security};

/// A duplex byte stream.
pub trait MailStream: AsyncRead + AsyncWrite + Unpin + Send + Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Debug> MailStream for T {}

/// A boxed [`MailStream`].
pub type BoxStream = Box<dyn MailStream>;

/// Opens a connection to an endpoint. For [`Security::StartTls`] the
/// implementation consumes the server greeting and upgrades before returning;
/// for [`Security::Tls`] the greeting is still unread.
pub trait Connector: Send + Sync {
    /// Connects and secures the stream.
    fn connect<'a>(&'a self, endpoint: &'a Endpoint) -> BoxFuture<'a, Result<BoxStream>>;
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<Option<Arc<rustls::ClientConfig>>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            let loaded = rustls_native_certs::load_native_certs();
            for cert in loaded.certs {
                let _ = roots.add(cert);
            }
            if roots.is_empty() {
                return None;
            }
            // Pin the provider: the build also links aws-lc-rs, so the
            // process-wide default is ambiguous.
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .ok()?
                .with_root_certificates(roots)
                .with_no_client_auth();
            Some(Arc::new(config))
        })
        .clone()
        .ok_or(MailError::Tls)
}

async fn read_line(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut line = Vec::with_capacity(128);
    loop {
        let byte = stream.read_u8().await.map_err(|_| MailError::Offline)?;
        line.push(byte);
        if byte == b'\n' {
            return Ok(line);
        }
        if line.len() > 4096 {
            return Err(MailError::Protocol);
        }
    }
}

/// The production connector.
#[derive(Clone, Copy, Debug, Default)]
pub struct TlsConnectorImpl;

impl TlsConnectorImpl {
    async fn open(endpoint: &Endpoint) -> Result<BoxStream> {
        endpoint.validate()?;
        let config = tls_config()?;
        let name =
            ServerName::try_from(endpoint.host.clone()).map_err(|_| MailError::InvalidInput)?;
        let mut tcp = timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((endpoint.host.as_str(), endpoint.port)),
        )
        .await?
        .map_err(|_| MailError::Offline)?;
        let _ = tcp.set_nodelay(true);
        if endpoint.security == Security::StartTls {
            let greeting = timeout(CONNECT_TIMEOUT, read_line(&mut tcp)).await??;
            if !greeting.starts_with(b"* OK") && !greeting.starts_with(b"* PREAUTH") {
                return Err(MailError::Protocol);
            }
            tcp.write_all(b"zs1 STARTTLS\r\n")
                .await
                .map_err(|_| MailError::Offline)?;
            loop {
                let line = timeout(CONNECT_TIMEOUT, read_line(&mut tcp)).await??;
                if line.starts_with(b"zs1 ") {
                    if !line.starts_with(b"zs1 OK") {
                        return Err(MailError::Unsupported);
                    }
                    break;
                }
            }
        }
        let tls = timeout(
            CONNECT_TIMEOUT,
            TlsConnector::from(config).connect(name, tcp),
        )
        .await?
        .map_err(|_| MailError::Tls)?;
        Ok(Box::new(tls))
    }
}

impl Connector for TlsConnectorImpl {
    fn connect<'a>(&'a self, endpoint: &'a Endpoint) -> BoxFuture<'a, Result<BoxStream>> {
        Box::pin(Self::open(endpoint))
    }
}
