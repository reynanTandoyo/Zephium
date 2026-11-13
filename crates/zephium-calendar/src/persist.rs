//! Calendars in the profile database (`zephium-store`, META migration v30):
//! accounts, calendars, refresh state and cached events survive restarts.
//!
//! Accounts and calendars used to live in `calendar.json`. On the first start
//! with an empty database that file is imported once, verified by reading the
//! database back, and renamed to `calendar.json.migrated` (a backup that is
//! never read again and holds no secrets); if the rename fails the file is
//! deleted, and if that fails too it stays where it is but is ignored from then
//! on because the database is no longer empty.

use crate::api::{CalendarAccountKindV1, CalendarAccountV1, CalendarV1};
use crate::error::{CalendarError, Result};
use crate::store::{
    CacheMeta, CachedRecord, CalendarStorage, EventCache, JsonFileStorage, StoredState,
};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use zephium_store::calendar_cache::{
    CacheResult, CalendarAccountRecord, CalendarCache, CalendarEventRecord, CalendarRecord,
    CalendarSyncState,
};
use zephium_store::SqliteStore;

const STATE_VERSION: u32 = 1;
/// Name a migrated `calendar.json` is renamed to.
pub const MIGRATED_FILE: &str = "calendar.json.migrated";

/// [`CalendarStorage`] and [`EventCache`] over the application store.
pub struct StoreCalendar {
    store: Arc<SqliteStore>,
    legacy: Option<PathBuf>,
}

impl StoreCalendar {
    /// Over `store`; `legacy` is the old `calendar.json` to import once.
    pub fn new(store: Arc<SqliteStore>, legacy: Option<PathBuf>) -> Self {
        Self { store, legacy }
    }

    fn run<R: Send + 'static>(
        &self,
        job: impl FnOnce(&mut CalendarCache<'_>) -> CacheResult<R> + Send + 'static,
    ) -> Result<R> {
        self.store
            .with_calendar_cache(job)
            .ok_or(CalendarError::Storage)?
            .map_err(|_| CalendarError::Storage)
    }

    fn read_state(&self) -> Result<StoredState> {
        let (accounts, calendars) =
            self.run(|cache| Ok((cache.accounts()?, cache.calendars()?)))?;
        Ok(StoredState {
            version: STATE_VERSION,
            accounts: accounts
                .into_iter()
                .filter_map(|record| {
                    Some(CalendarAccountV1 {
                        id: record.id,
                        kind: match record.kind.as_str() {
                            "caldav" => CalendarAccountKindV1::Caldav,
                            "subscription" => CalendarAccountKindV1::Subscription,
                            _ => return None,
                        },
                        name: record.name,
                        server_url: record.server_url,
                        username: record.username,
                    })
                })
                .collect(),
            calendars: calendars
                .into_iter()
                .map(|record| CalendarV1 {
                    id: record.id,
                    account_id: record.account_id,
                    url: record.href,
                    name: record.display_name,
                    color: record.colour,
                    read_only: record.read_only,
                    visible: record.visible,
                })
                .collect(),
        })
    }

    fn write_state(&self, state: &StoredState) -> Result<()> {
        let accounts: Vec<CalendarAccountRecord> = state
            .accounts
            .iter()
            .map(|account| CalendarAccountRecord {
                id: account.id.clone(),
                kind: match account.kind {
                    CalendarAccountKindV1::Caldav => "caldav",
                    CalendarAccountKindV1::Subscription => "subscription",
                }
                .to_owned(),
                name: account.name.clone(),
                server_url: account.server_url.clone(),
                username: account.username.clone(),
            })
            .collect();
        let calendars: Vec<CalendarRecord> = state
            .calendars
            .iter()
            .map(|calendar| CalendarRecord {
                id: calendar.id.clone(),
                account_id: calendar.account_id.clone(),
                href: calendar.url.clone(),
                display_name: calendar.name.clone(),
                colour: calendar.color.clone(),
                read_only: calendar.read_only,
                visible: calendar.visible,
            })
            .collect();
        if self.run(move |cache| cache.replace_directory(&accounts, &calendars))? {
            Ok(())
        } else {
            Err(CalendarError::Limit)
        }
    }

    /// Imports `calendar.json` when the database holds no accounts yet.
    fn import_legacy(&self) {
        let Some(path) = &self.legacy else {
            return;
        };
        if !path.exists() {
            return;
        }
        let Ok(old) = JsonFileStorage::new(path.clone()).load() else {
            return; // unreadable or too large: leave it for the person to see
        };
        if !old.accounts.is_empty() || !old.calendars.is_empty() {
            if self.write_state(&old).is_err() {
                return;
            }
            // Verify before touching the file.
            match self.read_state() {
                Ok(read) if read.accounts == old.accounts && read.calendars == old.calendars => {}
                _ => return,
            }
        }
        let backup = path.with_file_name(MIGRATED_FILE);
        if std::fs::rename(path, &backup).is_err() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl CalendarStorage for StoreCalendar {
    fn load(&self) -> Result<StoredState> {
        let state = self.read_state()?;
        if state.accounts.is_empty() && self.legacy.as_ref().is_some_and(|path| path.exists()) {
            self.import_legacy();
            return self.read_state();
        }
        Ok(state)
    }

    fn save(&self, state: &StoredState) -> Result<()> {
        self.write_state(state)
    }
}

fn instant(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap_or_default()
}

impl EventCache for StoreCalendar {
    fn load(&self, calendar_id: &str) -> Result<Option<(CacheMeta, Vec<CachedRecord>)>> {
        let id = calendar_id.to_owned();
        let loaded = self.run(move |cache| {
            let Some(state) = cache.sync_state(&id)? else {
                return Ok(None);
            };
            Ok(Some((state, cache.events(&id)?)))
        })?;
        Ok(loaded.map(|(state, events)| {
            (
                CacheMeta {
                    ctag: state.ctag,
                    sync_token: state.sync_token,
                    synced_at: state.synced_at.map(instant),
                },
                events
                    .into_iter()
                    .map(|record| CachedRecord {
                        href: record.href,
                        etag: record.etag,
                        raw: record.ical,
                        last_seen: instant(record.last_seen),
                    })
                    .collect(),
            )
        }))
    }

    fn apply(&self, calendar_id: &str, upserts: &[CachedRecord], removed: &[String]) -> Result<()> {
        let id = calendar_id.to_owned();
        let rows: Vec<CalendarEventRecord> = upserts.iter().map(to_row).collect();
        let removed = removed.to_vec();
        if self.run(move |cache| cache.apply_events(&id, &rows, &removed))? {
            Ok(())
        } else {
            Err(CalendarError::Limit)
        }
    }

    fn replace(&self, calendar_id: &str, records: &[CachedRecord]) -> Result<()> {
        let id = calendar_id.to_owned();
        let rows: Vec<CalendarEventRecord> = records.iter().map(to_row).collect();
        if self.run(move |cache| cache.replace_events(&id, &rows))? {
            Ok(())
        } else {
            Err(CalendarError::Limit)
        }
    }

    fn set_meta(&self, calendar_id: &str, meta: &CacheMeta) -> Result<()> {
        let id = calendar_id.to_owned();
        let state = CalendarSyncState {
            ctag: meta.ctag.clone(),
            sync_token: meta.sync_token.clone(),
            synced_at: meta.synced_at.map(|at| at.timestamp()),
        };
        if self.run(move |cache| cache.set_sync_state(&id, &state))? {
            Ok(())
        } else {
            Err(CalendarError::NotFound)
        }
    }
}

/// Parsed bounds for the range index; unreadable text gets an empty range.
fn to_row(record: &CachedRecord) -> CalendarEventRecord {
    let events = crate::ical::parse_calendar(&record.raw).unwrap_or_default();
    let (start, end) = match crate::ical::bounds(&events) {
        Some((start, end)) => (start.timestamp(), end.map(|end| end.timestamp())),
        None => (0, Some(0)),
    };
    CalendarEventRecord {
        href: record.href.clone(),
        etag: record.etag.clone(),
        uid: events
            .first()
            .map(|event| event.uid.chars().take(1_000).collect())
            .unwrap_or_default(),
        ical: record.raw.clone(),
        start_utc: start,
        end_utc: end,
        last_seen: record.last_seen.timestamp(),
    }
}
