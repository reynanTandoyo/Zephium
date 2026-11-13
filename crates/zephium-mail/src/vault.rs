//! Passwords live only in the OS vault (Windows Credential Manager, macOS
//! Keychain) under `app.zephium.mail|<account id>`. They never reach the
//! store, a log, a diagnostic or IPC output.

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::error::{MailError, Result};
use crate::model::id_ok;

/// Prefix of every vault target this crate writes.
pub const SERVICE: &str = "app.zephium.mail";
/// Longest password accepted.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// The vault target for an account.
#[must_use]
pub fn target(account_id: &str) -> Option<String> {
    id_ok(account_id).then(|| format!("{SERVICE}|{account_id}"))
}

/// Where passwords are kept.
pub trait PasswordVault: Send + Sync {
    /// Stores or replaces the password.
    fn set(&self, account_id: &str, password: &str) -> Result<()>;
    /// Reads the password.
    fn get(&self, account_id: &str) -> Result<Zeroizing<String>>;
    /// Removes the password; a missing one is success.
    fn remove(&self, account_id: &str) -> Result<()>;
}

fn checked_password(password: &str) -> Result<()> {
    if password.is_empty()
        || password.len() > MAX_PASSWORD_BYTES
        || password
            .chars()
            .any(|c| c == '\0' || c == '\r' || c == '\n')
    {
        return Err(MailError::InvalidInput);
    }
    Ok(())
}

/// The real vault.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsVault;

impl PasswordVault for OsVault {
    fn set(&self, account_id: &str, password: &str) -> Result<()> {
        let target = target(account_id).ok_or(MailError::InvalidInput)?;
        checked_password(password)?;
        let _turn = zephium_credentials::turn()?;
        zephium_credentials::write(&target, password.as_bytes())?;
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<Zeroizing<String>> {
        let target = target(account_id).ok_or(MailError::InvalidInput)?;
        let _turn = zephium_credentials::turn()?;
        let bytes = zephium_credentials::read(&target)?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| MailError::Credentials)?;
        Ok(Zeroizing::new(text))
    }

    fn remove(&self, account_id: &str) -> Result<()> {
        let target = target(account_id).ok_or(MailError::InvalidInput)?;
        let _turn = zephium_credentials::turn()?;
        match zephium_credentials::delete(&target) {
            Ok(()) | Err(zephium_credentials::VaultError::Missing) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// An in-memory vault for tests and fakes.
#[derive(Debug, Default)]
pub struct MemoryVault(Mutex<HashMap<String, String>>);

impl PasswordVault for MemoryVault {
    fn set(&self, account_id: &str, password: &str) -> Result<()> {
        target(account_id).ok_or(MailError::InvalidInput)?;
        checked_password(password)?;
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(account_id.to_owned(), password.to_owned());
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<Zeroizing<String>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(account_id)
            .map(|password| Zeroizing::new(password.clone()))
            .ok_or(MailError::Credentials)
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
        assert_eq!(target("abc-1").as_deref(), Some("app.zephium.mail|abc-1"));
        assert_eq!(target("a|b"), None);
        assert_eq!(target(""), None);
    }

    #[test]
    fn memory_vault_round_trips_and_bounds_passwords() {
        let vault = MemoryVault::default();
        assert!(vault.get("a1").is_err());
        vault.set("a1", "hunter2").unwrap();
        assert_eq!(&*vault.get("a1").unwrap(), "hunter2");
        assert_eq!(vault.set("a1", ""), Err(MailError::InvalidInput));
        assert_eq!(vault.set("a1", "x\ny"), Err(MailError::InvalidInput));
        assert_eq!(
            vault.set("a1", &"x".repeat(MAX_PASSWORD_BYTES + 1)),
            Err(MailError::InvalidInput)
        );
        vault.remove("a1").unwrap();
        vault.remove("a1").unwrap();
        assert!(vault.get("a1").is_err());
    }
}
