use super::{response, utc, Fake};
use crate::api::{CalendarAccountKindV1, CalendarAccountV1, CalendarV1};
use crate::http::Method;
use crate::persist::{StoreCalendar, MIGRATED_FILE};
use crate::service::CalendarService;
use crate::store::{
    CacheMeta, CachedRecord, CalendarStorage, EventCache, JsonFileStorage, MemorySecrets,
    StoredState,
};
use std::path::Path;
use std::sync::Arc;
use zephium_store::SqliteStore;

const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Public holidays\r\n\
BEGIN:VEVENT\r\nUID:h1@x\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20260101\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:h2@x\r\nSUMMARY:Weekly\r\nDTSTART:20260105T100000Z\r\nDTEND:20260105T110000Z\r\n\
RRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn state() -> StoredState {
    StoredState {
        version: 1,
        accounts: vec![
            CalendarAccountV1 {
                id: "A1".to_owned(),
                kind: CalendarAccountKindV1::Caldav,
                name: "Work".to_owned(),
                server_url: "https://dav.example.com/".to_owned(),
                username: Some("ann".to_owned()),
                oauth: None,
            },
            CalendarAccountV1 {
                id: "A2".to_owned(),
                kind: CalendarAccountKindV1::Subscription,
                name: "Holidays".to_owned(),
                server_url: "https://example.org/h.ics".to_owned(),
                username: None,
                oauth: None,
            },
        ],
        calendars: vec![
            CalendarV1 {
                id: "C1".to_owned(),
                account_id: "A1".to_owned(),
                url: "https://dav.example.com/work/".to_owned(),
                name: "Work".to_owned(),
                color: Some("#4285F4".to_owned()),
                read_only: false,
                visible: true,
            },
            CalendarV1 {
                id: "C2".to_owned(),
                account_id: "A2".to_owned(),
                url: "https://example.org/h.ics".to_owned(),
                name: "Holidays".to_owned(),
                color: None,
                read_only: true,
                visible: false,
            },
        ],
    }
}

fn open(dir: &Path, legacy: Option<&Path>) -> StoreCalendar {
    StoreCalendar::new(
        Arc::new(SqliteStore::open(dir).expect("store")),
        legacy.map(Path::to_path_buf),
    )
}

fn record(href: &str, etag: &str, summary: &str) -> CachedRecord {
    CachedRecord {
        href: href.to_owned(),
        etag: Some(etag.to_owned()),
        raw: format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{href}@x\r\nSUMMARY:{summary}\r\n\
             DTSTART:20260113T100000Z\r\nDTEND:20260113T110000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        ),
        last_seen: utc("2026-01-10T12:00:00Z"),
    }
}

#[test]
fn state_events_and_sync_state_survive_a_reopen() {
    let dir = tempfile::tempdir().expect("dir");
    let meta = CacheMeta {
        ctag: Some("c1".to_owned()),
        sync_token: Some("t1".to_owned()),
        synced_at: Some(utc("2026-01-10T12:00:00Z")),
    };
    {
        let storage = open(dir.path(), None);
        assert_eq!(
            CalendarStorage::load(&storage).expect("empty"),
            StoredState {
                version: 1,
                ..StoredState::default()
            }
        );
        storage.save(&state()).expect("save");
        storage
            .apply(
                "C1",
                &[
                    record("a.ics", "\"e1\"", "One"),
                    record("b.ics", "\"e2\"", "Two"),
                ],
                &[],
            )
            .expect("events");
        storage.set_meta("C1", &meta).expect("meta");
        storage
            .apply("C1", &[], &["b.ics".to_owned()])
            .expect("remove");
    }
    let storage = open(dir.path(), None);
    assert_eq!(CalendarStorage::load(&storage).expect("load"), state());
    let (loaded_meta, records) = EventCache::load(&storage, "C1")
        .expect("cache")
        .expect("known");
    assert_eq!(loaded_meta, meta);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].href, "a.ics");
    assert_eq!(records[0].etag.as_deref(), Some("\"e1\""));
    assert!(records[0].raw.contains("SUMMARY:One"));

    // The range index holds the parsed bounds.
    let store = SqliteStore::open(dir.path()).expect("store");
    let (from, to) = (
        utc("2026-01-13T10:30:00Z").timestamp(),
        utc("2026-01-13T10:31:00Z").timestamp(),
    );
    let inside = store
        .with_calendar_cache(move |cache| cache.events_in_range("C1", from, to).unwrap())
        .expect("answer");
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].start_utc, utc("2026-01-13T10:00:00Z").timestamp());
    assert_eq!(
        inside[0].end_utc,
        Some(utc("2026-01-13T11:00:00Z").timestamp())
    );
    let (from, to) = (
        utc("2026-01-14T00:00:00Z").timestamp(),
        utc("2026-01-15T00:00:00Z").timestamp(),
    );
    assert!(store
        .with_calendar_cache(move |cache| cache.events_in_range("C1", from, to).unwrap())
        .expect("answer")
        .is_empty());

    // Removing an account on save drops its calendars' cached events.
    let mut next = state();
    next.accounts.remove(0);
    next.calendars.remove(0);
    storage.save(&next).expect("save");
    assert_eq!(CalendarStorage::load(&storage).expect("load"), next);
    assert!(EventCache::load(&storage, "C1").expect("load").is_none());
}

#[test]
fn calendar_json_is_imported_once_and_renamed() {
    let dir = tempfile::tempdir().expect("dir");
    let legacy = dir.path().join("calendar.json");
    JsonFileStorage::new(legacy.clone())
        .save(&state())
        .expect("old file");
    {
        let storage = open(dir.path(), Some(&legacy));
        assert_eq!(CalendarStorage::load(&storage).expect("import"), state());
    }
    assert!(
        !legacy.exists(),
        "the old file no longer sits where it was read"
    );
    let backup = dir.path().join(MIGRATED_FILE);
    assert_eq!(
        JsonFileStorage::new(backup.clone()).load().expect("backup"),
        state()
    );

    // A later start reads the database; a stray calendar.json is left alone.
    std::fs::write(&legacy, "{\"version\":1,\"accounts\":[],\"calendars\":[]}").expect("stray");
    let storage = open(dir.path(), Some(&legacy));
    assert_eq!(CalendarStorage::load(&storage).expect("load"), state());
    assert!(legacy.exists());
}

#[test]
fn an_unreadable_calendar_json_is_left_untouched_and_nothing_is_imported() {
    let dir = tempfile::tempdir().expect("dir");
    let legacy = dir.path().join("calendar.json");
    std::fs::write(&legacy, "not json").expect("bad file");
    let storage = open(dir.path(), Some(&legacy));
    let loaded = CalendarStorage::load(&storage).expect("load");
    assert!(loaded.accounts.is_empty() && loaded.calendars.is_empty());
    assert_eq!(std::fs::read_to_string(&legacy).expect("kept"), "not json");
    assert!(!dir.path().join(MIGRATED_FILE).exists());

    // An empty old file migrates to an empty database and is renamed.
    JsonFileStorage::new(legacy.clone())
        .save(&StoredState::default())
        .expect("empty file");
    assert!(CalendarStorage::load(&storage)
        .expect("load")
        .accounts
        .is_empty());
    assert!(!legacy.exists());
}

#[tokio::test]
async fn a_service_over_the_store_keeps_accounts_and_events_across_restarts() {
    let dir = tempfile::tempdir().expect("dir");
    let fake = Arc::new(Fake::default());
    fake.route(
        Method::Get,
        "*",
        "",
        response(200, &[("content-type", "text/calendar")], FEED),
    );
    let clock: crate::service::Clock = Arc::new(|| utc("2026-01-10T12:00:00Z"));
    let range = (utc("2026-01-01T00:00:00Z"), utc("2026-01-20T00:00:00Z"));
    {
        let persistent = Arc::new(open(dir.path(), None));
        let service = CalendarService::with_cache(
            fake.clone(),
            persistent.clone(),
            Arc::new(MemorySecrets::default()),
            persistent,
            clock.clone(),
        )
        .expect("service");
        service
            .add_subscription("https://example.org/holidays.ics", "")
            .await
            .expect("subscription");
        assert_eq!(
            service
                .events(range.0, range.1)
                .expect("events")
                .occurrences
                .len(),
            4
        );
    }

    // After a restart the events are there with no request to the server.
    let quiet = Arc::new(Fake::default());
    let persistent = Arc::new(open(dir.path(), None));
    let service = CalendarService::with_cache(
        quiet.clone(),
        persistent.clone(),
        Arc::new(MemorySecrets::default()),
        persistent,
        clock.clone(),
    )
    .expect("restarted");
    service.warm();
    assert_eq!(service.accounts().len(), 1);
    assert_eq!(service.accounts()[0].name, "Public holidays");
    let calendars = service.calendars();
    assert_eq!(calendars.len(), 1);
    assert!(calendars[0].read_only && calendars[0].visible);
    let occurrences = service
        .events(range.0, range.1)
        .expect("events")
        .occurrences;
    assert_eq!(occurrences.len(), 4);
    assert_eq!(occurrences[0].summary, "Holiday");
    assert!(quiet.requests().is_empty());

    // Removing the account removes its calendars and their events.
    service.set_visible(&calendars[0].id, false).expect("hide");
    let account = service.accounts()[0].id.clone();
    service.remove_account(&account).expect("remove");
    drop(service);
    let persistent = Arc::new(open(dir.path(), None));
    let again = CalendarService::with_cache(
        quiet,
        persistent.clone(),
        Arc::new(MemorySecrets::default()),
        persistent.clone(),
        clock,
    )
    .expect("third start");
    assert!(again.accounts().is_empty() && again.calendars().is_empty());
    assert!(EventCache::load(persistent.as_ref(), &calendars[0].id)
        .expect("load")
        .is_none());
}
