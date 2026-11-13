//! Durable calendar bookkeeping (accounts and calendars, never secrets or
//! events) behind an injectable trait, plus the OS-vault secret store.

use crate::api::{CalendarAccountV1, CalendarV1};
use crate::error::{CalendarError, Result};
use crate::http::Secret;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;
const STATE_VERSION: u32 = 1;

/// Everything persisted about calendars.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredState {
    /// Format version.
    pub version: u32,
    /// Accounts and subscriptions.
    pub accounts: Vec<CalendarAccountV1>,
    /// Calendars of all accounts.
    pub calendars: Vec<CalendarV1>,
}

/// Loads and saves [`StoredState`].
pub trait CalendarStorage: Send + Sync {
    /// The saved state, or the default when nothing was saved yet.
    fn load(&self) -> Result<StoredState>;
    /// Replaces the saved state.
    fn save(&self, state: &StoredState) -> Result<()>;
}

/// JSON file in the profile data directory, replaced atomically.
pub struct JsonFileStorage {
    path: PathBuf,
}

impl JsonFileStorage {
    /// Storage at `path` (the directory must exist).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl CalendarStorage for JsonFileStorage {
    fn load(&self) -> Result<StoredState> {
        match std::fs::metadata(&self.path) {
            Ok(meta) if meta.len() > MAX_STATE_BYTES => return Err(CalendarError::TooLarge),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoredState {
                    version: STATE_VERSION,
                    ..StoredState::default()
                });
            }
            Err(_) => return Err(CalendarError::Storage),
        }
        let bytes = std::fs::read(&self.path).map_err(|_| CalendarError::Storage)?;
        let state: StoredState =
            serde_json::from_slice(&bytes).map_err(|_| CalendarError::Storage)?;
        if state.version > STATE_VERSION {
            return Err(CalendarError::Storage);
        }
        Ok(state)
    }

    fn save(&self, state: &StoredState) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(state).map_err(|_| CalendarError::Storage)?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|_| CalendarError::Storage)?;
        std::fs::rename(&temp, &self.path).map_err(|_| {
            let _ = std::fs::remove_file(&temp);
            CalendarError::Storage
        })
    }
}

/// In-memory storage for tests and for hosts without a profile directory.
#[derive(Default)]
pub struct MemoryStorage(Mutex<StoredState>);

impl CalendarStorage for MemoryStorage {
    fn load(&self) -> Result<StoredState> {
        Ok(self.0.lock().map_err(|_| CalendarError::Storage)?.clone())
    }

    fn save(&self, state: &StoredState) -> Result<()> {
        *self.0.lock().map_err(|_| CalendarError::Storage)? = state.clone();
        Ok(())
    }
}

/// Refresh state of one calendar, kept so a restart can continue with a delta.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheMeta {
    /// `getctag` at the last refresh.
    pub ctag: Option<String>,
    /// `sync-token` at the last refresh.
    pub sync_token: Option<String>,
    /// When the calendar was last refreshed.
    pub synced_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One cached `.ics` resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedRecord {
    /// Resource address.
    pub href: String,
    /// ETag as sent by the server.
    pub etag: Option<String>,
    /// Raw iCalendar text (empty when the resource could not be read).
    pub raw: String,
    /// When a refresh last saw it.
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

/// Durable event cache behind the in-memory one. Holds iCalendar text and
/// refresh state, never secrets. A failing cache costs a re-sync, nothing
/// else, so the service ignores its write errors.
pub trait EventCache: Send + Sync {
    /// Refresh state and resources of a calendar; `None` when the calendar is
    /// unknown to the cache.
    fn load(&self, calendar_id: &str) -> Result<Option<(CacheMeta, Vec<CachedRecord>)>>;
    /// Adds or replaces resources and removes the given addresses.
    fn apply(&self, calendar_id: &str, upserts: &[CachedRecord], removed: &[String]) -> Result<()>;
    /// Replaces every resource of a calendar.
    fn replace(&self, calendar_id: &str, records: &[CachedRecord]) -> Result<()>;
    /// Records the refresh state.
    fn set_meta(&self, calendar_id: &str, meta: &CacheMeta) -> Result<()>;
}

type MemoryCalendar = (CacheMeta, HashMap<String, CachedRecord>);

/// In-memory cache for tests and for hosts without a profile database.
#[derive(Default)]
pub struct MemoryEventCache(Mutex<HashMap<String, MemoryCalendar>>);

impl EventCache for MemoryEventCache {
    fn load(&self, calendar_id: &str) -> Result<Option<(CacheMeta, Vec<CachedRecord>)>> {
        let map = self.0.lock().map_err(|_| CalendarError::Storage)?;
        Ok(map.get(calendar_id).map(|(meta, records)| {
            let mut records: Vec<_> = records.values().cloned().collect();
            records.sort_by(|a, b| a.href.cmp(&b.href));
            (meta.clone(), records)
        }))
    }

    fn apply(&self, calendar_id: &str, upserts: &[CachedRecord], removed: &[String]) -> Result<()> {
        let mut map = self.0.lock().map_err(|_| CalendarError::Storage)?;
        let entry = map.entry(calendar_id.to_owned()).or_default();
        for href in removed {
            entry.1.remove(href);
        }
        for record in upserts {
            entry.1.insert(record.href.clone(), record.clone());
        }
        Ok(())
    }

    fn replace(&self, calendar_id: &str, records: &[CachedRecord]) -> Result<()> {
        let mut map = self.0.lock().map_err(|_| CalendarError::Storage)?;
        map.entry(calendar_id.to_owned()).or_default().1 = records
            .iter()
            .map(|record| (record.href.clone(), record.clone()))
            .collect();
        Ok(())
    }

    fn set_meta(&self, calendar_id: &str, meta: &CacheMeta) -> Result<()> {
        let mut map = self.0.lock().map_err(|_| CalendarError::Storage)?;
        map.entry(calendar_id.to_owned()).or_default().0 = meta.clone();
        Ok(())
    }
}

/// Where account passwords live.
pub trait SecretStore: Send + Sync {
    /// The password of an account, if stored.
    fn get(&self, account: &str) -> Result<Option<Secret>>;
    /// Stores or replaces the password.
    fn set(&self, account: &str, secret: &Secret) -> Result<()>;
    /// Removes the password; missing is success.
    fn delete(&self, account: &str) -> Result<()>;
}

/// Vault target for an account: `app.zephium.calendar|<account id>`.
pub fn vault_target(account: &str) -> Result<String> {
    if account.is_empty()
        || account.len() > 64
        || !account.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(CalendarError::InvalidInput);
    }
    Ok(format!("app.zephium.calendar|{account}"))
}

/// Windows Credential Manager (and the macOS Keychain) through
/// `zephium-credentials`.
pub struct VaultSecrets;

#[cfg(any(windows, target_os = "macos"))]
impl SecretStore for VaultSecrets {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        use zephium_credentials::VaultError;
        let target = vault_target(account)?;
        let _turn = zephium_credentials::turn().map_err(|_| CalendarError::Credentials)?;
        match zephium_credentials::read(&target) {
            Ok(bytes) => String::from_utf8(bytes.to_vec())
                .map(|text| Some(Secret::new(text)))
                .map_err(|_| CalendarError::Credentials),
            Err(VaultError::Missing) => Ok(None),
            Err(_) => Err(CalendarError::Credentials),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        let target = vault_target(account)?;
        let _turn = zephium_credentials::turn().map_err(|_| CalendarError::Credentials)?;
        zephium_credentials::write(&target, secret.expose().as_bytes())
            .map_err(|_| CalendarError::Credentials)
    }

    fn delete(&self, account: &str) -> Result<()> {
        use zephium_credentials::VaultError;
        let target = vault_target(account)?;
        let _turn = zephium_credentials::turn().map_err(|_| CalendarError::Credentials)?;
        match zephium_credentials::delete(&target) {
            Ok(()) | Err(VaultError::Missing) => Ok(()),
            Err(_) => Err(CalendarError::Credentials),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl SecretStore for VaultSecrets {
    fn get(&self, _account: &str) -> Result<Option<Secret>> {
        Err(CalendarError::Credentials)
    }
    fn set(&self, _account: &str, _secret: &Secret) -> Result<()> {
        Err(CalendarError::Credentials)
    }
    fn delete(&self, _account: &str) -> Result<()> {
        Err(CalendarError::Credentials)
    }
}

/// In-memory secrets for tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        let map = self.0.lock().map_err(|_| CalendarError::Credentials)?;
        Ok(map.get(account).map(|value| Secret::new(value.clone())))
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| CalendarError::Credentials)?
            .insert(account.to_owned(), secret.expose().to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|_| CalendarError::Credentials)?
            .remove(account);
        Ok(())
    }
}
