//! The two providers, their endpoints and scopes. Client IDs are supplied by
//! the person at sign-in; none is committed.

use serde::{Deserialize, Serialize};

use crate::error::{OAuthError, Result};

/// Mail scopes for Microsoft 365 (IMAP, SMTP submission, refresh token).
pub const MICROSOFT_MAIL_SCOPES: &[&str] = &[
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "https://outlook.office.com/SMTP.Send",
    "offline_access",
];
/// Mail scope for Gmail.
pub const GOOGLE_MAIL_SCOPES: &[&str] = &["https://mail.google.com/"];
/// Calendar scopes for Microsoft Graph (T-0040).
pub const MICROSOFT_CALENDAR_SCOPES: &[&str] = &[
    "https://graph.microsoft.com/Calendars.ReadWrite",
    "offline_access",
];
/// Calendar scope for Google Calendar (T-0040).
pub const GOOGLE_CALENDAR_SCOPES: &[&str] = &["https://www.googleapis.com/auth/calendar"];

/// Longest client ID accepted.
pub const MAX_CLIENT_ID_BYTES: usize = 200;
/// Longest client secret accepted.
pub const MAX_CLIENT_SECRET_BYTES: usize = 200;
/// Longest tenant accepted.
pub const MAX_TENANT_BYTES: usize = 128;
/// The default Microsoft tenant: work, school and personal accounts.
pub const DEFAULT_TENANT: &str = "common";

/// An identity provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Microsoft identity platform (Microsoft 365, Outlook.com).
    Microsoft,
    /// Google (Gmail).
    Google,
}

/// What a sign-in asks permission for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// IMAP and SMTP access.
    Mail,
    /// Calendar access: Google Calendar API, Microsoft Graph.
    Calendar,
}

/// Who is signing in: the provider, the person's own client ID and, for
/// Microsoft, the tenant. Holds no secret.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ClientConfig {
    /// The provider.
    pub provider: Provider,
    /// The client (application) ID the person registered.
    pub client_id: String,
    /// Microsoft tenant: `common`, `organizations`, `consumers`, a tenant ID
    /// or a domain. Ignored for Google.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

fn token_chars_ok(text: &str, allow_underscore: bool) -> bool {
    text.bytes().all(|b| {
        b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || (allow_underscore && b == b'_')
    })
}

/// True for a plausible client ID (GUID, or `<digits>-<token>.apps.googleusercontent.com`).
#[must_use]
pub fn client_id_ok(client_id: &str) -> bool {
    (1..=MAX_CLIENT_ID_BYTES).contains(&client_id.len()) && token_chars_ok(client_id, true)
}

/// True for a plausible client secret (printable ASCII without spaces).
#[must_use]
pub fn client_secret_ok(secret: &str) -> bool {
    (1..=MAX_CLIENT_SECRET_BYTES).contains(&secret.len())
        && secret.bytes().all(|b| b.is_ascii_graphic())
}

impl ClientConfig {
    /// Checks the client ID and tenant against their bounds.
    pub fn validate(&self) -> Result<()> {
        if !client_id_ok(&self.client_id) {
            return Err(OAuthError::InvalidInput);
        }
        if let Some(tenant) = &self.tenant {
            if tenant.is_empty()
                || tenant.len() > MAX_TENANT_BYTES
                || !token_chars_ok(tenant, false)
                || tenant.starts_with(['.', '-'])
            {
                return Err(OAuthError::InvalidInput);
            }
        }
        Ok(())
    }

    fn tenant(&self) -> &str {
        self.tenant.as_deref().unwrap_or(DEFAULT_TENANT)
    }

    /// The authorization endpoint.
    #[must_use]
    pub fn authorize_endpoint(&self) -> String {
        match self.provider {
            Provider::Microsoft => format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
                self.tenant()
            ),
            Provider::Google => "https://accounts.google.com/o/oauth2/v2/auth".to_owned(),
        }
    }

    /// The token endpoint.
    #[must_use]
    pub fn token_endpoint(&self) -> String {
        match self.provider {
            Provider::Microsoft => format!(
                "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                self.tenant()
            ),
            Provider::Google => "https://oauth2.googleapis.com/token".to_owned(),
        }
    }

    /// Space-separated scopes for the requested purposes, without repeats.
    /// Microsoft issues a token per resource: asking for mail and calendar
    /// together grants both, but each access token serves one resource.
    #[must_use]
    pub fn scopes(&self, purposes: &[Purpose]) -> String {
        let mut scopes: Vec<&str> = Vec::new();
        for purpose in purposes {
            let set = match (self.provider, purpose) {
                (Provider::Microsoft, Purpose::Mail) => MICROSOFT_MAIL_SCOPES,
                (Provider::Google, Purpose::Mail) => GOOGLE_MAIL_SCOPES,
                (Provider::Microsoft, Purpose::Calendar) => MICROSOFT_CALENDAR_SCOPES,
                (Provider::Google, Purpose::Calendar) => GOOGLE_CALENDAR_SCOPES,
            };
            for scope in set {
                if !scopes.contains(scope) {
                    scopes.push(scope);
                }
            }
        }
        scopes.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(provider: Provider, tenant: Option<&str>) -> ClientConfig {
        ClientConfig {
            provider,
            client_id: "00000000-0000-0000-0000-000000000000".into(),
            tenant: tenant.map(str::to_owned),
        }
    }

    #[test]
    fn microsoft_defaults_to_the_common_tenant() {
        let c = config(Provider::Microsoft, None);
        assert_eq!(
            c.token_endpoint(),
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        let c = config(Provider::Microsoft, Some("contoso.onmicrosoft.com"));
        assert!(c.authorize_endpoint().contains("/contoso.onmicrosoft.com/"));
    }

    #[test]
    fn scopes_are_the_documented_sets() {
        let m = config(Provider::Microsoft, None);
        assert_eq!(
            m.scopes(&[Purpose::Mail]),
            "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access"
        );
        assert_eq!(
            m.scopes(&[Purpose::Mail, Purpose::Calendar])
                .matches("offline_access")
                .count(),
            1
        );
        let g = config(Provider::Google, None);
        assert_eq!(g.scopes(&[Purpose::Mail]), "https://mail.google.com/");
        assert_eq!(
            g.scopes(&[Purpose::Calendar]),
            "https://www.googleapis.com/auth/calendar"
        );
    }

    #[test]
    fn validation_refuses_hostile_ids_and_tenants() {
        assert!(config(Provider::Microsoft, None).validate().is_ok());
        for bad in ["", "a b", "a&scope=x", "a/../b", "\u{e9}"] {
            let mut c = config(Provider::Google, None);
            c.client_id = bad.into();
            assert_eq!(c.validate(), Err(OAuthError::InvalidInput), "{bad}");
        }
        for bad in ["", "evil.com/x", "a?b", "-lead", "a_b"] {
            assert_eq!(
                config(Provider::Microsoft, Some(bad)).validate(),
                Err(OAuthError::InvalidInput),
                "{bad}"
            );
        }
        let mut long = config(Provider::Google, None);
        long.client_id = "a".repeat(MAX_CLIENT_ID_BYTES + 1);
        assert!(long.validate().is_err());
    }

    #[test]
    fn client_secret_bounds() {
        assert!(client_secret_ok("GOCSPX-abc_123"));
        assert!(!client_secret_ok(""));
        assert!(!client_secret_ok("a b"));
        assert!(!client_secret_ok("a\nb"));
    }
}
