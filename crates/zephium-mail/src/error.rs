//! The closed error set. No variant carries server text: a hostile or merely
//! chatty server can neither fill logs nor reach the UI through an error.

use serde::{Deserialize, Serialize};

/// Why a mail operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum MailError {
    /// An argument was malformed or out of bounds.
    #[error("invalid input")]
    InvalidInput,
    /// The account, folder or message does not exist (any more).
    #[error("not found")]
    NotFound,
    /// The server could not be reached or the connection dropped.
    #[error("offline")]
    Offline,
    /// The server did not answer within the bound.
    #[error("timeout")]
    Timeout,
    /// The TLS handshake or certificate check failed.
    #[error("tls")]
    Tls,
    /// The server refused the user name or password.
    #[error("authentication failed")]
    AuthFailed,
    /// The server answered NO to a valid request.
    #[error("rejected by server")]
    ServerRejected,
    /// The server sent something that is not valid IMAP/SMTP.
    #[error("protocol")]
    Protocol,
    /// A size bound was exceeded.
    #[error("too large")]
    TooLarge,
    /// The password is not in the Credential Manager or it refused access.
    #[error("credentials")]
    Credentials,
    /// The server lacks a needed capability.
    #[error("unsupported")]
    Unsupported,
    /// Too many operations are in flight.
    #[error("busy")]
    Busy,
    /// A local failure that is none of the above.
    #[error("internal")]
    Internal,
}

impl From<&async_imap::error::Error> for MailError {
    fn from(error: &async_imap::error::Error) -> Self {
        use async_imap::error::Error as E;
        match error {
            E::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => Self::Timeout,
            E::Io(io) if io.kind() == std::io::ErrorKind::InvalidData => Self::Protocol,
            E::Io(_) | E::ConnectionLost => Self::Offline,
            E::No(_) | E::Append => Self::ServerRejected,
            E::Validate(_) => Self::InvalidInput,
            _ => Self::Protocol,
        }
    }
}

impl From<async_imap::error::Error> for MailError {
    fn from(error: async_imap::error::Error) -> Self {
        Self::from(&error)
    }
}

impl From<zephium_credentials::VaultError> for MailError {
    fn from(_: zephium_credentials::VaultError) -> Self {
        Self::Credentials
    }
}

impl From<tokio::time::error::Elapsed> for MailError {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        Self::Timeout
    }
}

/// `Result` over [`MailError`].
pub type Result<T> = std::result::Result<T, MailError>;

#[cfg(test)]
mod tests {
    use super::*;
    use async_imap::error::Error as E;

    #[test]
    fn imap_errors_map_without_server_text() {
        let secret = "SECRET-server-text".to_owned();
        assert_eq!(
            MailError::from(E::No(secret.clone())),
            MailError::ServerRejected
        );
        assert_eq!(MailError::from(E::Bad(secret.clone())), MailError::Protocol);
        assert_eq!(MailError::from(E::ConnectionLost), MailError::Offline);
        assert_eq!(MailError::from(E::Append), MailError::ServerRejected);
        let timed_out = std::io::Error::new(std::io::ErrorKind::TimedOut, secret.clone());
        assert_eq!(MailError::from(E::Io(timed_out)), MailError::Timeout);
        let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, secret.clone());
        assert_eq!(MailError::from(E::Io(reset)), MailError::Offline);
        for error in [
            MailError::ServerRejected,
            MailError::Protocol,
            MailError::Offline,
        ] {
            assert!(!error.to_string().contains("SECRET"));
            assert!(!format!("{error:?}").contains("SECRET"));
        }
    }

    #[test]
    fn vault_errors_become_credentials() {
        use zephium_credentials::VaultError;
        for error in [
            VaultError::Missing,
            VaultError::Inaccessible,
            VaultError::AwaitingApproval,
        ] {
            assert_eq!(MailError::from(error), MailError::Credentials);
        }
    }
}
