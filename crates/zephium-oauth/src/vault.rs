//! Refresh tokens live only in the OS vault (Windows Credential Manager,
//! macOS Keychain) under `app.zephium.oauth|<account id>`. They never reach
//! the store, a log, a diagnostic or IPC output.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{OAuthError, Result};

/// Prefix of every vault target this crate writes.
pub const SERVICE: &str = "app.zephium.oauth";
/// Longest vault record accepted.
pub const MAX_RECORD_BYTES: usize = 8 * 1024;

/// What is kept per account: the refresh token and, for Google desktop
/// clients that need one, the client secret the person pasted.
#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct StoredSecret {
    /// The refresh token.
    pub refresh_token: String,
    /// The client secret, when the provider requires one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl fmt::Debug for StoredSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredSecret(..)")
    }
}

impl Drop for StoredSecret {
    fn drop(&mut self) {
        self.refresh_token.zeroize();
        self.client_secret.zeroize();
    }
}

/// True for an account id: 1 to 40 characters of `[A-Za-z0-9-]`.
#[must_use]
pub fn id_ok(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The vault target for an account.
#[must_use]
pub fn target(account_id: &str) -> Option<String> {
    id_ok(account_id).then(|| format!("{SERVICE}|{account_id}"))
}

fn encode(secret: &StoredSecret) -> Result<Vec<u8>> {
    if secret.refresh_token.is_empty() {
        return Err(OAuthError::InvalidInput);
    }
    let bytes = serde_json::to_vec(secret).map_err(|_| OAuthError::Internal)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(OAuthError::InvalidInput);
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<StoredSecret> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(OAuthError::Credentials);
    }
    let secret: StoredSecret =
        serde_json::from_slice(bytes).map_err(|_| OAuthError::Credentials)?;
    if secret.refresh_token.is_empty() {
        return Err(OAuthError::Credentials);
    }
    Ok(secret)
}

/// Where refresh tokens are kept.
pub trait RefreshVault: Send + Sync {
    /// Stores or replaces the secret.
    fn set(&self, account_id: &str, secret: &StoredSecret) -> Result<()>;
    /// Reads the secret.
    fn get(&self, account_id: &str) -> Result<StoredSecret>;
    /// Removes the secret; a missing one is success.
    fn remove(&self, account_id: &str) -> Result<()>;
}

/// The real vault.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsRefreshVault;

#[cfg(any(windows, target_os = "macos"))]
impl RefreshVault for OsRefreshVault {
    fn set(&self, account_id: &str, secret: &StoredSecret) -> Result<()> {
        let target = target(account_id).ok_or(OAuthError::InvalidInput)?;
        let bytes = zeroize::Zeroizing::new(encode(secret)?);
        let _turn = zephium_credentials::turn()?;
        zephium_credentials::write(&target, &bytes)?;
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<StoredSecret> {
        let target = target(account_id).ok_or(OAuthError::InvalidInput)?;
        let _turn = zephium_credentials::turn()?;
        let bytes = zephium_credentials::read(&target)?;
        decode(&bytes)
    }

    fn remove(&self, account_id: &str) -> Result<()> {
        let target = target(account_id).ok_or(OAuthError::InvalidInput)?;
        let _turn = zephium_credentials::turn()?;
        match zephium_credentials::delete(&target) {
            Ok(()) | Err(zephium_credentials::VaultError::Missing) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl RefreshVault for OsRefreshVault {
    fn set(&self, _account_id: &str, _secret: &StoredSecret) -> Result<()> {
        Err(OAuthError::Unsupported)
    }

    fn get(&self, _account_id: &str) -> Result<StoredSecret> {
        Err(OAuthError::Unsupported)
    }

    fn remove(&self, _account_id: &str) -> Result<()> {
        Err(OAuthError::Unsupported)
    }
}

/// An in-memory vault for tests and fakes.
#[derive(Debug, Default)]
pub struct MemoryRefreshVault(Mutex<HashMap<String, Vec<u8>>>);

impl RefreshVault for MemoryRefreshVault {
    fn set(&self, account_id: &str, secret: &StoredSecret) -> Result<()> {
        target(account_id).ok_or(OAuthError::InvalidInput)?;
        let bytes = encode(secret)?;
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(account_id.to_owned(), bytes);
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<StoredSecret> {
        let guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        decode(guard.get(account_id).ok_or(OAuthError::Credentials)?)
    }

    fn remove(&self, account_id: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(account_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_names_the_service_and_rejects_bad_ids() {
        assert_eq!(target("abc-1").as_deref(), Some("app.zephium.oauth|abc-1"));
        assert_eq!(target("a|b"), None);
        assert_eq!(target(""), None);
    }

    #[test]
    fn memory_vault_round_trips_and_refuses_empty_tokens() {
        let vault = MemoryRefreshVault::default();
        assert!(vault.get("a1").is_err());
        let secret = StoredSecret {
            refresh_token: "rt".into(),
            client_secret: Some("cs".into()),
        };
        vault.set("a1", &secret).unwrap();
        assert_eq!(vault.get("a1").unwrap(), secret);
        let empty = StoredSecret {
            refresh_token: String::new(),
            client_secret: None,
        };
        assert_eq!(vault.set("a1", &empty), Err(OAuthError::InvalidInput));
        vault.remove("a1").unwrap();
        vault.remove("a1").unwrap();
        assert!(vault.get("a1").is_err());
        assert_eq!(format!("{secret:?}"), "StoredSecret(..)");
    }

    #[test]
    fn corrupt_records_are_credential_errors() {
        assert_eq!(decode(b"not json").unwrap_err(), OAuthError::Credentials);
        assert_eq!(
            decode(br#"{"refresh_token":""}"#).unwrap_err(),
            OAuthError::Credentials
        );
    }
}
