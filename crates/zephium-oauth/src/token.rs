//! Token endpoint responses. Parsed defensively: bounded, typed, and mapped
//! to [`OAuthError`] without the server's text.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::error::{OAuthError, Result};
use crate::http::{HttpResponse, MAX_BODY_BYTES};

/// Lifetime assumed when the server names none.
pub const DEFAULT_LIFETIME: Duration = Duration::from_secs(3600);
const MIN_LIFETIME: Duration = Duration::from_secs(1);
const MAX_LIFETIME: Duration = Duration::from_secs(24 * 3600);
const MAX_TOKEN_BYTES: usize = 16 * 1024;

/// What the token endpoint granted.
pub struct TokenSet {
    /// The bearer token. Memory only.
    pub access_token: Zeroizing<String>,
    /// A new refresh token, when the server issued one.
    pub refresh_token: Option<Zeroizing<String>>,
    /// How long the access token lives.
    pub lifetime: Duration,
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenSet")
            .field("lifetime", &self.lifetime)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Raw {
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<serde_json::Value>,
    error: Option<String>,
}

fn token_ok(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_BYTES
        && token.bytes().all(|b| b.is_ascii_graphic())
}

fn lifetime(value: Option<&serde_json::Value>) -> Duration {
    let seconds = match value {
        Some(serde_json::Value::Number(number)) => number.as_u64(),
        Some(serde_json::Value::String(text)) => text.parse().ok(),
        _ => None,
    };
    seconds
        .map_or(DEFAULT_LIFETIME, Duration::from_secs)
        .clamp(MIN_LIFETIME, MAX_LIFETIME)
}

/// Parses a token endpoint response.
///
/// `invalid_grant` means the grant or refresh token is dead
/// ([`OAuthError::SignInRequired`]); other 4xx answers are
/// [`OAuthError::Rejected`]; 5xx and 429 are [`OAuthError::Offline`].
pub fn parse_response(response: &HttpResponse) -> Result<TokenSet> {
    if response.body.len() > MAX_BODY_BYTES {
        return Err(OAuthError::Protocol);
    }
    if response.status == 429 || response.status >= 500 {
        return Err(OAuthError::Offline);
    }
    let raw: Raw = serde_json::from_slice(&response.body).map_err(|_| {
        if (400..500).contains(&response.status) {
            OAuthError::Rejected
        } else {
            OAuthError::Protocol
        }
    })?;
    if response.status != 200 || raw.error.is_some() {
        return Err(match raw.error.as_deref() {
            Some("invalid_grant") => OAuthError::SignInRequired,
            _ => OAuthError::Rejected,
        });
    }
    if !raw
        .token_type
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer"))
    {
        return Err(OAuthError::Protocol);
    }
    let access = raw
        .access_token
        .filter(|token| token_ok(token))
        .ok_or(OAuthError::Protocol)?;
    let refresh = match raw.refresh_token {
        None => None,
        Some(token) if token_ok(&token) => Some(Zeroizing::new(token)),
        Some(_) => return Err(OAuthError::Protocol),
    };
    Ok(TokenSet {
        access_token: Zeroizing::new(access),
        refresh_token: refresh,
        lifetime: lifetime(raw.expires_in.as_ref()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn parses_a_good_response() {
        let set = parse_response(&response(
            200,
            r#"{"access_token":"at","refresh_token":"rt","token_type":"Bearer","expires_in":3599,"scope":"x"}"#,
        ))
        .unwrap();
        assert_eq!(&*set.access_token, "at");
        assert_eq!(set.refresh_token.as_deref().map(String::as_str), Some("rt"));
        assert_eq!(set.lifetime, Duration::from_secs(3599));
        assert!(!format!("{set:?}").contains("\"at\""));
    }

    #[test]
    fn expires_in_may_be_text_missing_or_absurd() {
        let lifetime_of = |extra: &str| {
            let body = format!(r#"{{"access_token":"at","token_type":"bearer"{extra}}}"#);
            parse_response(&response(200, &body)).unwrap().lifetime
        };
        assert_eq!(
            lifetime_of(r#","expires_in":"120""#),
            Duration::from_secs(120)
        );
        assert_eq!(lifetime_of(""), DEFAULT_LIFETIME);
        assert_eq!(lifetime_of(r#","expires_in":99999999999"#), MAX_LIFETIME);
        assert_eq!(lifetime_of(r#","expires_in":-5"#), DEFAULT_LIFETIME);
    }

    #[test]
    fn errors_map_without_server_text() {
        let dead = response(
            400,
            r#"{"error":"invalid_grant","error_description":"SECRET"}"#,
        );
        assert_eq!(
            parse_response(&dead).unwrap_err(),
            OAuthError::SignInRequired
        );
        let bad = response(
            401,
            r#"{"error":"invalid_client","error_description":"SECRET"}"#,
        );
        assert_eq!(parse_response(&bad).unwrap_err(), OAuthError::Rejected);
        assert_eq!(
            parse_response(&response(503, "<html>SECRET</html>")).unwrap_err(),
            OAuthError::Offline
        );
        assert_eq!(
            parse_response(&response(400, "<html>SECRET</html>")).unwrap_err(),
            OAuthError::Rejected
        );
        assert_eq!(
            parse_response(&response(200, "not json")).unwrap_err(),
            OAuthError::Protocol
        );
        for error in [OAuthError::SignInRequired, OAuthError::Rejected] {
            assert!(!error.to_string().contains("SECRET"));
        }
    }

    #[test]
    fn refuses_malformed_success_bodies() {
        for body in [
            r#"{"token_type":"bearer"}"#,
            r#"{"access_token":"","token_type":"bearer"}"#,
            r#"{"access_token":"a b","token_type":"bearer"}"#,
            r#"{"access_token":"at","token_type":"mac"}"#,
            r#"{"access_token":"at"}"#,
            r#"{"access_token":"at","token_type":"bearer","refresh_token":"x y"}"#,
            r#"{"access_token":"at","token_type":"bearer","error":"x"}"#,
        ] {
            assert!(parse_response(&response(200, body)).is_err(), "{body}");
        }
        let huge = HttpResponse {
            status: 200,
            body: vec![b' '; MAX_BODY_BYTES + 1],
        };
        assert_eq!(parse_response(&huge).unwrap_err(), OAuthError::Protocol);
    }
}
