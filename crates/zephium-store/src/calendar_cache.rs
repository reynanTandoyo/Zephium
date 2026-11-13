//! Fork-owned (T-0041): the local calendar cache in the meta database.
//! Accounts and calendars (no secrets), per-calendar sync state (ctag and
//! sync-token) and one row per `.ics` resource with its raw iCalendar text and
//! parsed start/end for range queries.
//!
//! The store keeps plain rows; `zephium-calendar` types are mapped by the caller.

use rusqlite::{params, Connection, OptionalExtension};

/// The result of a cache operation, so callers need no SQLite dependency.
pub type CacheResult<T> = rusqlite::Result<T>;

/// Accounts and subscriptions kept.
pub const MAX_CALENDAR_ACCOUNTS: usize = 20;
/// Calendars kept in total.
pub const MAX_CALENDARS: usize = 500;
/// Resources kept per calendar.
pub const MAX_EVENTS_PER_CALENDAR: usize = 20_000;
/// Largest raw iCalendar text kept for one resource.
pub const MAX_ICAL_BYTES: usize = 8 * 1024 * 1024;

/// A stored account or subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarAccountRecord {
    /// Account id.
    pub id: String,
    /// `caldav` or `subscription`.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Server or feed address.
    pub server_url: String,
    /// CalDAV user name.
    pub username: Option<String>,
}

/// A stored calendar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarRecord {
    /// Calendar id.
    pub id: String,
    /// Owning account.
    pub account_id: String,
    /// Collection address.
    pub href: String,
    /// Display name.
    pub display_name: String,
    /// `#RRGGBB`.
    pub colour: Option<String>,
    /// Events cannot be changed.
    pub read_only: bool,
    /// Shown in queries.
    pub visible: bool,
}

/// Collection fingerprint of the last refresh.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CalendarSyncState {
    /// `getctag`.
    pub ctag: Option<String>,
    /// `sync-token`.
    pub sync_token: Option<String>,
    /// Unix seconds of the last refresh.
    pub synced_at: Option<i64>,
}

/// One cached resource (an `.ics` file with a series and its overrides).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarEventRecord {
    /// Resource address.
    pub href: String,
    /// ETag as sent by the server.
    pub etag: Option<String>,
    /// UID of the first event.
    pub uid: String,
    /// Raw iCalendar text.
    pub ical: String,
    /// Earliest start, Unix seconds.
    pub start_utc: i64,
    /// Latest end, Unix seconds; `None` for a series without an end.
    pub end_utc: Option<i64>,
    /// Unix seconds of the refresh that last saw it.
    pub last_seen: i64,
}

/// The cache over one connection.
pub struct CalendarCache<'a> {
    conn: &'a mut Connection,
}

fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CalendarEventRecord> {
    Ok(CalendarEventRecord {
        href: row.get(0)?,
        etag: row.get(1)?,
        uid: row.get(2)?,
        ical: row.get(3)?,
        start_utc: row.get(4)?,
        end_utc: row.get(5)?,
        last_seen: row.get(6)?,
    })
}

const EVENT_COLUMNS: &str = "href, etag, uid, ical, start_utc, end_utc, last_seen";

fn insert_event(
    tx: &rusqlite::Transaction<'_>,
    calendar_id: &str,
    record: &CalendarEventRecord,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO calendar_events(calendar_id, href, etag, uid, ical, start_utc, end_utc, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(calendar_id, href) DO UPDATE SET
             etag = excluded.etag, uid = excluded.uid, ical = excluded.ical,
             start_utc = excluded.start_utc, end_utc = excluded.end_utc,
             last_seen = excluded.last_seen",
        params![
            calendar_id,
            record.href,
            record.etag,
            record.uid,
            record.ical,
            record.start_utc,
            record.end_utc,
            record.last_seen
        ],
    )?;
    Ok(())
}

fn calendar_exists(tx: &rusqlite::Transaction<'_>, calendar_id: &str) -> rusqlite::Result<bool> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM calendars WHERE id = ?1",
            [calendar_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

impl<'a> CalendarCache<'a> {
    /// Wraps a meta-database connection.
    pub fn new(conn: &'a mut Connection) -> Self {
        Self { conn }
    }

    /// Accounts in the order they were added.
    pub fn accounts(&self) -> CacheResult<Vec<CalendarAccountRecord>> {
        self.conn
            .prepare_cached(
                "SELECT id, kind, name, server_url, username FROM calendar_accounts
                 ORDER BY position, id",
            )?
            .query_map([], |row| {
                Ok(CalendarAccountRecord {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    server_url: row.get(3)?,
                    username: row.get(4)?,
                })
            })?
            .collect()
    }

    /// Calendars in the order they were added.
    pub fn calendars(&self) -> CacheResult<Vec<CalendarRecord>> {
        self.conn
            .prepare_cached(
                "SELECT id, account_id, href, display_name, colour, read_only, visible
                 FROM calendars ORDER BY position, id",
            )?
            .query_map([], |row| {
                Ok(CalendarRecord {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    href: row.get(2)?,
                    display_name: row.get(3)?,
                    colour: row.get(4)?,
                    read_only: row.get(5)?,
                    visible: row.get(6)?,
                })
            })?
            .collect()
    }

    /// Makes the stored accounts and calendars equal to the given lists, in
    /// that order. Calendars that stay keep their sync state and events; the
    /// rest, with their events, are removed. False (and nothing written) when
    /// a limit would be exceeded or a calendar names an unknown account.
    pub fn replace_directory(
        &mut self,
        accounts: &[CalendarAccountRecord],
        calendars: &[CalendarRecord],
    ) -> CacheResult<bool> {
        if accounts.len() > MAX_CALENDAR_ACCOUNTS
            || calendars.len() > MAX_CALENDARS
            || calendars
                .iter()
                .any(|calendar| !accounts.iter().any(|a| a.id == calendar.account_id))
        {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        for (position, account) in accounts.iter().enumerate() {
            tx.execute(
                "INSERT INTO calendar_accounts(id, position, kind, name, server_url, username)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET position = excluded.position, kind = excluded.kind,
                     name = excluded.name, server_url = excluded.server_url,
                     username = excluded.username",
                params![
                    account.id,
                    position as i64,
                    account.kind,
                    account.name,
                    account.server_url,
                    account.username
                ],
            )?;
        }
        for (position, calendar) in calendars.iter().enumerate() {
            tx.execute(
                "INSERT INTO calendars(id, account_id, position, href, display_name, colour, read_only, visible)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET account_id = excluded.account_id,
                     position = excluded.position, href = excluded.href,
                     display_name = excluded.display_name, colour = excluded.colour,
                     read_only = excluded.read_only, visible = excluded.visible",
                params![
                    calendar.id,
                    calendar.account_id,
                    position as i64,
                    calendar.href,
                    calendar.display_name,
                    calendar.colour,
                    calendar.read_only,
                    calendar.visible
                ],
            )?;
        }
        // Foreign keys may be off on this connection: delete children first.
        let keep_calendars: Vec<&str> = calendars.iter().map(|c| c.id.as_str()).collect();
        let keep_accounts: Vec<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
        let stale_calendars: Vec<String> = {
            let mut statement = tx.prepare("SELECT id FROM calendars")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|id| !keep_calendars.contains(&id.as_str()))
                .collect()
        };
        for id in &stale_calendars {
            tx.execute("DELETE FROM calendar_events WHERE calendar_id = ?1", [id])?;
            tx.execute("DELETE FROM calendars WHERE id = ?1", [id])?;
        }
        let stale_accounts: Vec<String> = {
            let mut statement = tx.prepare("SELECT id FROM calendar_accounts")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|id| !keep_accounts.contains(&id.as_str()))
                .collect()
        };
        for id in &stale_accounts {
            tx.execute("DELETE FROM calendar_accounts WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// The last refresh state of a calendar; `None` for an unknown calendar.
    pub fn sync_state(&self, calendar_id: &str) -> CacheResult<Option<CalendarSyncState>> {
        self.conn
            .query_row(
                "SELECT ctag, sync_token, synced_at FROM calendars WHERE id = ?1",
                [calendar_id],
                |row| {
                    Ok(CalendarSyncState {
                        ctag: row.get(0)?,
                        sync_token: row.get(1)?,
                        synced_at: row.get(2)?,
                    })
                },
            )
            .optional()
    }

    /// Records the refresh state of a calendar. False for an unknown calendar.
    pub fn set_sync_state(
        &mut self,
        calendar_id: &str,
        state: &CalendarSyncState,
    ) -> CacheResult<bool> {
        Ok(self.conn.execute(
            "UPDATE calendars SET ctag = ?2, sync_token = ?3, synced_at = ?4 WHERE id = ?1",
            params![calendar_id, state.ctag, state.sync_token, state.synced_at],
        )? > 0)
    }

    /// Every cached resource of a calendar, ordered by address.
    pub fn events(&self, calendar_id: &str) -> CacheResult<Vec<CalendarEventRecord>> {
        self.conn
            .prepare_cached(&format!(
                "SELECT {EVENT_COLUMNS} FROM calendar_events WHERE calendar_id = ?1 ORDER BY href"
            ))?
            .query_map([calendar_id], event_row)?
            .collect()
    }

    /// Resources of a calendar that may overlap `[from, to)` (Unix seconds):
    /// those starting before `to` that end after `from` or never end.
    pub fn events_in_range(
        &self,
        calendar_id: &str,
        from: i64,
        to: i64,
    ) -> CacheResult<Vec<CalendarEventRecord>> {
        self.conn
            .prepare_cached(&format!(
                "SELECT {EVENT_COLUMNS} FROM calendar_events
                 WHERE calendar_id = ?1 AND start_utc < ?3 AND (end_utc IS NULL OR end_utc > ?2)
                 ORDER BY start_utc, href"
            ))?
            .query_map(params![calendar_id, from, to], event_row)?
            .collect()
    }

    /// Adds or replaces resources and removes the given addresses, in one
    /// transaction. False (and nothing written) for an unknown calendar, an
    /// oversized text or when the per-calendar limit would be exceeded.
    pub fn apply_events(
        &mut self,
        calendar_id: &str,
        upserts: &[CalendarEventRecord],
        removed: &[String],
    ) -> CacheResult<bool> {
        if upserts
            .iter()
            .any(|record| record.ical.len() > MAX_ICAL_BYTES)
        {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        if !calendar_exists(&tx, calendar_id)? {
            return Ok(false);
        }
        for href in removed {
            tx.execute(
                "DELETE FROM calendar_events WHERE calendar_id = ?1 AND href = ?2",
                params![calendar_id, href],
            )?;
        }
        for record in upserts {
            insert_event(&tx, calendar_id, record)?;
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM calendar_events WHERE calendar_id = ?1",
            [calendar_id],
            |r| r.get(0),
        )?;
        if count > MAX_EVENTS_PER_CALENDAR as i64 {
            return Ok(false); // dropping the transaction rolls it back
        }
        tx.commit()?;
        Ok(true)
    }

    /// Replaces every resource of a calendar. Same refusals as
    /// [`Self::apply_events`].
    pub fn replace_events(
        &mut self,
        calendar_id: &str,
        records: &[CalendarEventRecord],
    ) -> CacheResult<bool> {
        if records.len() > MAX_EVENTS_PER_CALENDAR
            || records
                .iter()
                .any(|record| record.ical.len() > MAX_ICAL_BYTES)
        {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        if !calendar_exists(&tx, calendar_id)? {
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM calendar_events WHERE calendar_id = ?1",
            [calendar_id],
        )?;
        for record in records {
            insert_event(&tx, calendar_id, record)?;
        }
        tx.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::{apply, META};

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, META).unwrap();
        conn
    }

    fn account(id: &str) -> CalendarAccountRecord {
        CalendarAccountRecord {
            id: id.into(),
            kind: "caldav".into(),
            name: "Work".into(),
            server_url: "https://dav.example.com/".into(),
            username: Some("ann".into()),
        }
    }

    fn calendar(id: &str, account_id: &str) -> CalendarRecord {
        CalendarRecord {
            id: id.into(),
            account_id: account_id.into(),
            href: format!("https://dav.example.com/{id}/"),
            display_name: id.into(),
            colour: Some("#4285F4".into()),
            read_only: false,
            visible: true,
        }
    }

    fn event(href: &str, start: i64, end: Option<i64>) -> CalendarEventRecord {
        CalendarEventRecord {
            href: href.into(),
            etag: Some("\"e1\"".into()),
            uid: format!("{href}@x"),
            ical: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into(),
            start_utc: start,
            end_utc: end,
            last_seen: 1,
        }
    }

    fn hrefs(found: Vec<CalendarEventRecord>) -> Vec<String> {
        found.into_iter().map(|e| e.href).collect()
    }

    #[test]
    fn directory_round_trips_and_removal_cascades() {
        let mut conn = conn();
        let accounts = vec![account("A1"), account("A2")];
        let calendars = vec![calendar("C1", "A1"), calendar("C2", "A2")];
        {
            let mut cache = CalendarCache::new(&mut conn);
            assert!(cache.replace_directory(&accounts, &calendars).unwrap());
            assert_eq!(cache.accounts().unwrap(), accounts);
            assert_eq!(cache.calendars().unwrap(), calendars);
            assert!(cache
                .apply_events("C2", &[event("a", 10, Some(20))], &[])
                .unwrap());

            let mut changed = calendars[..1].to_vec();
            changed[0].visible = false;
            assert!(cache.replace_directory(&accounts[..1], &changed).unwrap());
            assert_eq!(cache.accounts().unwrap(), accounts[..1].to_vec());
            assert_eq!(cache.calendars().unwrap(), changed);
            assert!(cache.events("C2").unwrap().is_empty());
        }
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM calendar_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn replacing_the_directory_keeps_sync_state_and_events() {
        let mut conn = conn();
        let mut cache = CalendarCache::new(&mut conn);
        let accounts = vec![account("A1")];
        let calendars = vec![calendar("C1", "A1")];
        cache.replace_directory(&accounts, &calendars).unwrap();
        let state = CalendarSyncState {
            ctag: Some("c9".into()),
            sync_token: Some("tok".into()),
            synced_at: Some(500),
        };
        assert!(cache.set_sync_state("C1", &state).unwrap());
        cache
            .apply_events("C1", &[event("a", 10, None)], &[])
            .unwrap();
        let mut renamed = calendars.clone();
        renamed[0].display_name = "Renamed".into();
        cache.replace_directory(&accounts, &renamed).unwrap();
        assert_eq!(cache.sync_state("C1").unwrap(), Some(state));
        assert_eq!(cache.events("C1").unwrap().len(), 1);
        assert_eq!(cache.sync_state("nope").unwrap(), None);
        assert!(!cache
            .set_sync_state("nope", &CalendarSyncState::default())
            .unwrap());
    }

    #[test]
    fn events_upsert_remove_and_range_query() {
        let mut conn = conn();
        let mut cache = CalendarCache::new(&mut conn);
        cache
            .replace_directory(&[account("A1")], &[calendar("C1", "A1")])
            .unwrap();
        let records = vec![
            event("early", 100, Some(200)),
            event("inside", 1_000, Some(2_000)),
            event("series", 50, None),
            event("late", 10_000, Some(11_000)),
        ];
        assert!(cache.apply_events("C1", &records, &[]).unwrap());
        assert_eq!(
            hrefs(cache.events_in_range("C1", 900, 3_000).unwrap()),
            ["series", "inside"]
        );
        // End is exclusive: touching ranges do not overlap.
        assert_eq!(
            hrefs(cache.events_in_range("C1", 200, 1_000).unwrap()),
            ["series"]
        );

        let mut changed = event("inside", 1_000, Some(2_500));
        changed.etag = Some("\"e2\"".into());
        assert!(cache
            .apply_events("C1", &[changed.clone()], &["late".to_owned()])
            .unwrap());
        let all = cache.events("C1").unwrap();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&changed));
        assert!(!all.iter().any(|e| e.href == "late"));

        assert!(cache
            .replace_events("C1", &[event("only", 1, Some(2))])
            .unwrap());
        assert_eq!(hrefs(cache.events("C1").unwrap()), ["only"]);
    }

    #[test]
    fn writes_refuse_unknown_calendars_oversized_text_and_bad_directories() {
        let mut conn = conn();
        let mut cache = CalendarCache::new(&mut conn);
        cache
            .replace_directory(&[account("A1")], &[calendar("C1", "A1")])
            .unwrap();
        assert!(!cache
            .apply_events("nope", &[event("a", 1, None)], &[])
            .unwrap());
        assert!(!cache.replace_events("nope", &[]).unwrap());
        let mut huge = event("big", 1, None);
        huge.ical = "x".repeat(MAX_ICAL_BYTES + 1);
        assert!(!cache.apply_events("C1", &[huge.clone()], &[]).unwrap());
        assert!(!cache.replace_events("C1", &[huge]).unwrap());
        // A calendar of an unknown account is refused and nothing changes.
        assert!(!cache
            .replace_directory(&[account("A1")], &[calendar("C9", "ghost")])
            .unwrap());
        assert_eq!(cache.calendars().unwrap().len(), 1);
        // The schema itself rejects malformed ids and kinds.
        let mut bad = account("a b");
        assert!(cache.replace_directory(&[bad.clone()], &[]).is_err());
        bad.id = "A1".into();
        bad.kind = "exchange".into();
        assert!(cache.replace_directory(&[bad], &[]).is_err());
        assert_eq!(cache.calendars().unwrap().len(), 1);
    }

    #[test]
    fn the_event_limit_per_calendar_rolls_the_write_back() {
        let mut conn = conn();
        let mut cache = CalendarCache::new(&mut conn);
        cache
            .replace_directory(&[account("A1")], &[calendar("C1", "A1")])
            .unwrap();
        let many: Vec<_> = (0..=MAX_EVENTS_PER_CALENDAR)
            .map(|i| event(&format!("e{i}"), i as i64, None))
            .collect();
        assert!(!cache.replace_events("C1", &many).unwrap());
        assert!(cache.events("C1").unwrap().is_empty());
        assert!(cache
            .apply_events("C1", &many[..MAX_EVENTS_PER_CALENDAR], &[])
            .unwrap());
        assert!(!cache
            .apply_events("C1", &many[MAX_EVENTS_PER_CALENDAR..], &[])
            .unwrap());
        assert_eq!(cache.events("C1").unwrap().len(), MAX_EVENTS_PER_CALENDAR);
    }
}
