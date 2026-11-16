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
use crate::limits::{CONNECT_TIMEOUT, MAX_STARTTLS_LINES};
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

/// Reads the greeting, asks for `STARTTLS` and waits for its tagged answer.
/// Bytes are read one at a time so nothing past the answer is consumed before
/// the TLS handshake. At most [`MAX_STARTTLS_LINES`] untagged lines are read;
/// the caller bounds the whole exchange in time.
async fn starttls_exchange<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> Result<()> {
    let greeting = read_line(stream).await?;
    if !greeting.starts_with(b"* OK") && !greeting.starts_with(b"* PREAUTH") {
        return Err(MailError::Protocol);
    }
    stream
        .write_all(b"zs1 STARTTLS\r\n")
        .await
        .map_err(|_| MailError::Offline)?;
    for _ in 0..MAX_STARTTLS_LINES {
        let line = read_line(stream).await?;
        if line.starts_with(b"zs1 ") {
            return if line.starts_with(b"zs1 OK") {
                Ok(())
            } else {
                Err(MailError::Unsupported)
            };
        }
    }
    Err(MailError::Protocol)
}

async fn read_line<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>> {
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
            timeout(CONNECT_TIMEOUT, starttls_exchange(&mut tcp)).await??;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    async fn run(script: Vec<u8>) -> (Result<()>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(1 << 20);
        server.write_all(&script).await.unwrap();
        let result = starttls_exchange(&mut client).await;
        drop(client);
        let mut sent = Vec::new();
        server.read_to_end(&mut sent).await.unwrap();
        (result, sent)
    }

    #[tokio::test]
    async fn an_answer_after_a_few_untagged_lines_upgrades() {
        let script = b"* OK hi\r\n* CAPABILITY X\r\n* NOTE y\r\nzs1 OK go\r\n".to_vec();
        let (result, sent) = run(script).await;
        assert_eq!(result, Ok(()));
        assert_eq!(sent, b"zs1 STARTTLS\r\n");
    }

    #[tokio::test]
    async fn a_refusal_and_a_bad_greeting_are_refused() {
        let (result, _) = run(b"* OK hi\r\nzs1 NO later\r\n".to_vec()).await;
        assert_eq!(result, Err(MailError::Unsupported));
        let (result, sent) = run(b"* BYE go away\r\n".to_vec()).await;
        assert_eq!(result, Err(MailError::Protocol));
        assert!(sent.is_empty());
    }

    #[tokio::test]
    async fn endless_untagged_lines_end_in_a_protocol_error() {
        let mut script = b"* OK hi\r\n".to_vec();
        for _ in 0..(MAX_STARTTLS_LINES * 4) {
            script.extend_from_slice(b"* NOISE\r\n");
        }
        // A tagged answer after the limit is never reached.
        script.extend_from_slice(b"zs1 OK late\r\n");
        let (result, _) = run(script).await;
        assert_eq!(result, Err(MailError::Protocol));
    }

    #[tokio::test]
    async fn an_overlong_line_is_a_protocol_error() {
        let mut script = b"* OK hi\r\n".to_vec();
        script.extend(std::iter::repeat_n(b'x', 10_000));
        let (result, _) = run(script).await;
        assert_eq!(result, Err(MailError::Protocol));
    }
}
