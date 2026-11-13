//! The closed error set. No variant carries server or browser text.

use serde::{Deserialize, Serialize};

/// Why an OAuth operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum OAuthError {
    /// An argument was malformed or out of bounds.
    #[error("invalid input")]
    InvalidInput,
    /// The person cancelled the sign-in.
    #[error("cancelled")]
    Cancelled,
    /// Nobody finished the consent in time.
    #[error("timeout")]
    Timeout,
    /// The provider could not be reached.
    #[error("offline")]
    Offline,
    /// The person declined, or the provider refused the consent.
    #[error("denied")]
    Denied,
    /// The redirect carried the wrong `state`: the response was not for this sign-in.
    #[error("state mismatch")]
    StateMismatch,
    /// The provider or the browser sent something that is not valid OAuth.
    #[error("protocol")]
    Protocol,
    /// The token endpoint refused the request (wrong client ID, for example).
    #[error("rejected")]
    Rejected,
    /// The refresh token is revoked or expired: sign in again.
    #[error("sign-in required")]
    SignInRequired,
    /// The OS vault failed or refused access.
    #[error("credentials")]
    Credentials,
    /// Another sign-in is already waiting.
    #[error("busy")]
    Busy,
    /// The platform has no vault.
    #[error("unsupported")]
    Unsupported,
    /// A local failure that is none of the above.
    #[error("internal")]
    Internal,
}

impl From<zephium_credentials::VaultError> for OAuthError {
    fn from(_: zephium_credentials::VaultError) -> Self {
        Self::Credentials
    }
}

/// `Result` over [`OAuthError`].
pub type Result<T> = std::result::Result<T, OAuthError>;
