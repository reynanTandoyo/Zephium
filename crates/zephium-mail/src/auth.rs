//! How an account proves itself (T-0034): a password from the OS vault, or
//! an OAuth 2.0 bearer token (XOAUTH2) for Microsoft 365 and Gmail. The
//! account only records *which*; the secret is fetched per connection and
//! wiped on drop.

use serde::{Deserialize, Serialize};
use zephium_oauth::provider::{ClientConfig, Provider};
use zephium_oauth::OAuthError;
use zeroize::Zeroizing;

use crate::error::{MailError, Result};

/// How an account signs in. Saved accounts written before OAuth existed have
/// no `auth` field and load as [`AuthKind::Password`].
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthKind {
    /// A password in the OS vault under `app.zephium.mail|<account id>`.
    #[default]
    Password,
    /// OAuth 2.0 with the person's own client ID; the refresh token lives in
    /// the OS vault under `app.zephium.oauth|<account id>`.
    Oauth {
        /// Microsoft or Google.
        provider: Provider,
        /// The client (application) ID the person registered.
        client_id: String,
        /// Microsoft tenant; `common` when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant: Option<String>,
    },
}

impl AuthKind {
    /// The OAuth client configuration, for an OAuth account.
    #[must_use]
    pub fn oauth_config(&self) -> Option<ClientConfig> {
        match self {
            Self::Password => None,
            Self::Oauth {
                provider,
                client_id,
                tenant,
            } => Some(ClientConfig {
                provider: *provider,
                client_id: client_id.clone(),
                tenant: tenant.clone(),
            }),
        }
    }

    /// Checks the OAuth fields against their bounds.
    pub fn validate(&self) -> Result<()> {
        match self.oauth_config() {
            Some(config) => config.validate().map_err(|_| MailError::InvalidInput),
            None => Ok(()),
        }
    }
}

/// The secret presented to a server for one connection.
pub enum Credential {
    /// A password: `LOGIN` for IMAP, `PLAIN`/`LOGIN` for SMTP.
    Password(Zeroizing<String>),
    /// An OAuth access token: `XOAUTH2` for both.
    Bearer(Zeroizing<String>),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Password(_) => "Credential::Password(..)",
            Self::Bearer(_) => "Credential::Bearer(..)",
        })
    }
}

impl Credential {
    /// A password credential.
    #[must_use]
    pub fn password(password: &str) -> Self {
        Self::Password(Zeroizing::new(password.to_owned()))
    }

    /// A bearer credential.
    #[must_use]
    pub fn bearer(token: &str) -> Self {
        Self::Bearer(Zeroizing::new(token.to_owned()))
    }

    /// The secret text.
    #[must_use]
    pub fn secret(&self) -> &str {
        match self {
            Self::Password(secret) | Self::Bearer(secret) => secret,
        }
    }
}

/// The XOAUTH2 initial client response:
/// `user=<user>^Aauth=Bearer <token>^A^A` (the SASL layer base64-encodes it).
/// Refuses a user or token that contains a control character, which could
/// splice extra fields into the response.
pub fn xoauth2(username: &str, token: &str) -> Result<Zeroizing<String>> {
    let clean =
        |text: &str| !text.is_empty() && !text.chars().any(|c| c.is_control() || c.is_whitespace());
    if !clean(username) || !clean(token) {
        return Err(MailError::InvalidInput);
    }
    Ok(Zeroizing::new(format!(
        "user={username}\u{1}auth=Bearer {token}\u{1}\u{1}"
    )))
}

/// The IMAP `AUTHENTICATE XOAUTH2` responder. The server's first challenge is
/// empty and gets the initial response; any later challenge is an error
/// document, answered with an empty line as RFC 7628 style servers expect,
/// after which the server ends the exchange with `NO`.
pub struct XOauth2 {
    initial: Option<Zeroizing<String>>,
}

impl XOauth2 {
    /// Builds the responder for one authentication.
    pub fn new(username: &str, token: &str) -> Result<Self> {
        Ok(Self {
            initial: Some(xoauth2(username, token)?),
        })
    }
}

impl async_imap::Authenticator for XOauth2 {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        self.initial
            .take()
            .map(|initial| initial.as_bytes().to_vec())
            .unwrap_or_default()
    }
}

impl From<OAuthError> for MailError {
    fn from(error: OAuthError) -> Self {
        match error {
            OAuthError::InvalidInput => Self::InvalidInput,
            OAuthError::Offline => Self::Offline,
            OAuthError::Timeout => Self::Timeout,
            OAuthError::Protocol => Self::Protocol,
            OAuthError::Credentials => Self::Credentials,
            OAuthError::Busy => Self::Busy,
            OAuthError::Unsupported => Self::Unsupported,
            // The person must sign in again: the same fix as a wrong password.
            OAuthError::Denied
            | OAuthError::StateMismatch
            | OAuthError::Rejected
            | OAuthError::SignInRequired => Self::AuthFailed,
            OAuthError::Cancelled | OAuthError::Internal => Self::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_imap::Authenticator;

    #[test]
    fn xoauth2_string_is_exact() {
        let text = xoauth2("ann@example.com", "ya29.abc").unwrap();
        assert_eq!(
            text.as_bytes(),
            b"user=ann@example.com\x01auth=Bearer ya29.abc\x01\x01"
        );
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(decoded, text.as_bytes());
    }

    #[test]
    fn xoauth2_refuses_injection() {
        for (user, token) in [
            ("a\u{1}b", "t"),
            ("a", "t\u{1}auth=Bearer x"),
            ("a", "t\r\nx"),
            ("a b", "t"),
            ("a", "t t"),
            ("", "t"),
            ("a", ""),
        ] {
            assert_eq!(xoauth2(user, token).unwrap_err(), MailError::InvalidInput);
        }
    }

    #[test]
    fn responder_sends_the_token_once() {
        let mut auth = XOauth2::new("u", "t").unwrap();
        assert_eq!(auth.process(b""), b"user=u\x01auth=Bearer t\x01\x01");
        // The server's error document is answered with an empty line.
        assert!(auth.process(b"{\"status\":\"401\"}").is_empty());
    }

    #[test]
    fn credentials_never_print() {
        let secret = Credential::bearer("SECRET");
        assert!(!format!("{secret:?}").contains("SECRET"));
        assert_eq!(secret.secret(), "SECRET");
    }

    #[test]
    fn oauth_errors_map_without_text() {
        assert_eq!(
            MailError::from(OAuthError::SignInRequired),
            MailError::AuthFailed
        );
        assert_eq!(MailError::from(OAuthError::Offline), MailError::Offline);
        assert_eq!(
            MailError::from(OAuthError::Credentials),
            MailError::Credentials
        );
    }

    #[test]
    fn auth_kind_serde_and_defaults() {
        assert_eq!(AuthKind::default(), AuthKind::Password);
        let oauth = AuthKind::Oauth {
            provider: Provider::Microsoft,
            client_id: "abc".into(),
            tenant: None,
        };
        let json = serde_json::to_string(&oauth).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"oauth","provider":"microsoft","client_id":"abc"}"#
        );
        assert_eq!(serde_json::from_str::<AuthKind>(&json).unwrap(), oauth);
        assert_eq!(
            serde_json::to_string(&AuthKind::Password).unwrap(),
            r#"{"kind":"password"}"#
        );
        assert!(serde_json::from_str::<AuthKind>(r#"{"kind":"magic"}"#).is_err());
        let bad = AuthKind::Oauth {
            provider: Provider::Google,
            client_id: "a b".into(),
            tenant: None,
        };
        assert_eq!(bad.validate(), Err(MailError::InvalidInput));
        assert!(oauth.validate().is_ok());
        assert_eq!(oauth.oauth_config().unwrap().provider, Provider::Microsoft);
    }
}
