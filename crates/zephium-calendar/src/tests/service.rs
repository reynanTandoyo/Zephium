use super::caldav::{multistatus, script_account, HEADER};
use super::{response, utc, Fake};
use crate::api::{CalendarAccountKindV1, CalendarEventDraftV1};
use crate::error::CalendarError;
use crate::http::{Method, Secret};
use crate::service::CalendarService;
use crate::store::{
    CalendarStorage, JsonFileStorage, MemorySecrets, MemoryStorage, SecretStore, StoredState,
};
use std::sync::Arc;

const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Public holidays\r\n\
BEGIN:VEVENT\r\nUID:h1@x\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20260101\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:h2@x\r\nSUMMARY:Weekly\r\nDTSTART:20260105T100000Z\r\nDTEND:20260105T110000Z\r\n\
RRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

struct Harness {
    fake: Arc<Fake>,
    storage: Arc<MemoryStorage>,
    secrets: Arc<MemorySecrets>,
    service: CalendarService,
}

fn harness() -> Harness {
    let fake = Arc::new(Fake::default());
    let storage = Arc::new(MemoryStorage::default());
    let secrets = Arc::new(MemorySecrets::default());
    let service = CalendarService::with_clock(
        fake.clone(),
        storage.clone(),
        secrets.clone(),
        Arc::new(|| utc("2026-01-10T12:00:00Z")),
    )
    .expect("service");
    Harness {
        fake,
        storage,
        secrets,
        service,
    }
}

fn work_report(summary: &str, etag: &str) -> String {
    multistatus(&format!(
        "<d:response><d:href>/dav/calendars/ann/work/one.ics</d:href><d:propstat><d:prop>\
         <d:getetag>{etag}</d:getetag><c:calendar-data>BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\n\
         UID:one@x\nSUMMARY:{summary}\nDTSTART;TZID=Europe/Berlin:20260112T090000\n\
         DTEND;TZID=Europe/Berlin:20260112T100000\nEND:VEVENT\nEND:VCALENDAR\n</c:calendar-data>\
         </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    ))
}

fn draft(summary: &str) -> CalendarEventDraftV1 {
    CalendarEventDraftV1 {
        summary: summary.to_owned(),
        description: String::new(),
        location: "Room 1".to_owned(),
        start: "2026-01-13T10:00:00".to_owned(),
        end: "2026-01-13T11:00:00".to_owned(),
        all_day: false,
        tzid: Some("Europe/Berlin".to_owned()),
        rrule: None,
        reminders: vec![10],
        url: None,
    }
}

async fn signed_in() -> Harness {
    let h = harness();
    script_account(&h.fake);
    h.fake.route(
        Method::Propfind,
        "/dav/calendars/ann/work/",
        "",
        response(
            207,
            &[],
            &format!(
                "{HEADER}<d:response><d:href>/dav/calendars/ann/work/</d:href><d:propstat><d:prop>\
                 <cs:getctag>c1</cs:getctag><d:sync-token>t1</d:sync-token></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"
            ),
        ),
    );
    h.fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "calendar-query",
        response(207, &[], &work_report("Planning", "\"e1\"")),
    );
    h.service
        .add_account(
            "https://cal.example.com/dav/",
            "ann@example.com",
            Secret::new("hunter2".to_owned()),
            Some("Work"),
        )
        .await
        .expect("account");
    h
}

#[tokio::test]
async fn account_is_stored_without_its_password() {
    let h = signed_in().await;
    let accounts = h.service.accounts();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].kind, CalendarAccountKindV1::Caldav);
    assert_eq!(accounts[0].name, "Work");
    assert_eq!(h.service.calendars().len(), 2);
    let saved = serde_json::to_string(&h.storage.load().expect("load")).expect("json");
    assert!(!saved.contains("hunter2"));
    assert!(!format!("{accounts:?}").contains("hunter2"));
    let secret = h
        .secrets
        .get(&accounts[0].id)
        .expect("get")
        .expect("present");
    assert_eq!(secret.expose(), "hunter2");
    // A second account for the same login is refused.
    let again = h
        .service
        .add_account(
            "https://cal.example.com/dav/",
            "ann@example.com",
            Secret::new("x".to_owned()),
            None,
        )
        .await;
    assert_eq!(again.err(), Some(CalendarError::InvalidInput));
}

#[tokio::test]
async fn wrong_password_adds_nothing() {
    let h = harness();
    h.fake
        .route(Method::Propfind, "/dav/", "", response(401, &[], ""));
    let result = h
        .service
        .add_account(
            "https://cal.example.com/dav/",
            "ann@example.com",
            Secret::new("nope".to_owned()),
            None,
        )
        .await;
    assert_eq!(result.err(), Some(CalendarError::Unauthorized));
    assert!(h.service.accounts().is_empty());
    assert!(h.storage.load().expect("load").accounts.is_empty());
}

#[tokio::test]
async fn sync_fills_the_cache_and_events_expand_with_colour() {
    let h = signed_in().await;
    let report = h.service.sync_all().await;
    assert_eq!(report.outcomes.len(), 2);
    let work = h.service.calendars()[0].clone();
    let events = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    // The second calendar answers 404 to its sync, so only "Planning" shows.
    assert_eq!(events.occurrences.len(), 1);
    let item = &events.occurrences[0];
    assert_eq!(item.summary, "Planning");
    assert_eq!(item.calendar_id, work.id);
    assert_eq!(item.start, "2026-01-12T08:00:00Z");
    assert_eq!(item.tzid.as_deref(), Some("Europe/Berlin"));
    assert_eq!(item.color.as_deref(), Some("#FF8800"));
    assert!(!item.read_only);
    assert!(report.outcomes.iter().any(|o| o.error.is_some()));
}

#[tokio::test]
async fn hidden_calendars_and_bad_ranges_are_handled() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.service.set_visible(&work, false).expect("hide");
    let none = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    assert!(none.occurrences.is_empty());
    assert!(!h.service.calendars()[0].visible);
    assert_eq!(
        h.service
            .events(utc("2026-01-20T00:00:00Z"), utc("2026-01-10T00:00:00Z"))
            .err(),
        Some(CalendarError::InvalidInput)
    );
    assert_eq!(
        h.service
            .events(utc("2000-01-01T00:00:00Z"), utc("2026-01-10T00:00:00Z"))
            .err(),
        Some(CalendarError::InvalidInput)
    );
    assert_eq!(
        h.service.set_visible("nope", true),
        Err(CalendarError::NotFound)
    );
}

#[tokio::test]
async fn unchanged_ctag_skips_the_report_unless_forced() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let before = h.fake.requests().len();
    h.service.sync_all().await; // forced: reads again
    let forced = h.fake.requests().len() - before;
    assert!(forced > 2);
    let calls_before = h.fake.requests().len();
    // Due-only sync leaves the fresh calendar alone; only the one that never
    // loaded (its server answers 404) is tried again.
    let work = h.service.calendars()[0].id.clone();
    let report = h.service.sync_due().await;
    assert_eq!(report.outcomes.len(), 1);
    assert_ne!(report.outcomes[0].calendar_id, work);
    assert!(h.fake.requests()[calls_before..]
        .iter()
        .all(|r| r.url.path() != "/dav/calendars/ann/work/"));
}

#[tokio::test]
async fn incremental_sync_uses_the_token_and_falls_back_when_refused() {
    let h = signed_in().await;
    h.service.sync_all().await;
    h.fake.clear();
    h.fake.route(
        Method::Propfind,
        "/dav/calendars/ann/work/",
        "",
        response(
            207,
            &[],
            &format!(
                "{HEADER}<d:response><d:href>/dav/calendars/ann/work/</d:href><d:propstat><d:prop>\
                 <cs:getctag>c2</cs:getctag><d:sync-token>t2</d:sync-token></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"
            ),
        ),
    );
    h.fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "sync-collection",
        response(
            207,
            &[],
            &format!(
                "{HEADER}<d:response><d:href>/dav/calendars/ann/work/one.ics</d:href>\
                 <d:status>HTTP/1.1 404 Not Found</d:status></d:response>\
                 <d:sync-token>t2</d:sync-token></d:multistatus>"
            ),
        ),
    );
    h.service.sync_all().await;
    let asked: Vec<String> = h
        .fake
        .requests()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert!(asked
        .iter()
        .any(|b| b.contains("sync-collection") && b.contains(">t1<")));
    let events = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    assert!(events.occurrences.is_empty(), "the removed event is gone");

    // Next time the server refuses the token: a full read repairs the cache.
    h.fake.clear();
    h.fake.route(
        Method::Propfind,
        "/dav/calendars/ann/work/",
        "",
        response(
            207,
            &[],
            &format!(
                "{HEADER}<d:response><d:href>/dav/calendars/ann/work/</d:href><d:propstat><d:prop>\
                 <cs:getctag>c3</cs:getctag><d:sync-token>t3</d:sync-token></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"
            ),
        ),
    );
    h.fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "sync-collection",
        response(403, &[], ""),
    );
    h.fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "calendar-query",
        response(207, &[], &work_report("Planning v2", "\"e5\"")),
    );
    h.service.sync_all().await;
    let events = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    assert_eq!(events.occurrences[0].summary, "Planning v2");
}

#[tokio::test]
async fn create_update_and_delete_go_through_etag_guarded_requests() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "/dav/calendars/ann/work/one.ics",
        "",
        response(204, &[("etag", "\"e2\"")], ""),
    );
    h.fake.route(
        Method::Delete,
        "/dav/calendars/ann/work/one.ics",
        "",
        response(204, &[], ""),
    );

    let before = h.fake.requests().len();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(201, &[("etag", "\"n1\"")], ""),
    );
    let created = h
        .service
        .create_event(&work, &draft("Lunch"))
        .await
        .expect("created");
    assert!(created.uid.ends_with("@zephium"));
    let put = &h.fake.requests()[before];
    assert_eq!(put.method, Method::Put);
    assert!(put.headers.contains(&("if-none-match", "*".to_owned())));
    let body = String::from_utf8_lossy(&put.body).into_owned();
    assert!(
        body.contains("SUMMARY:Lunch")
            && body.contains("DTSTART;TZID=Europe/Berlin:20260113T100000")
    );
    assert!(body.contains("TRIGGER:-PT10M"));

    let updated = h
        .service
        .update_event(&work, "one@x", &draft("Planning!"))
        .await;
    assert!(updated.is_ok());
    let put = h
        .fake
        .requests()
        .into_iter()
        .rfind(|r| r.method == Method::Put)
        .expect("put");
    assert!(put.headers.contains(&("if-match", "\"e1\"".to_owned())));
    let events = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    assert!(events.occurrences.iter().any(|o| o.summary == "Planning!"));
    assert!(events.occurrences.iter().all(|o| o.summary != "Planning"));

    assert_eq!(h.service.delete_event(&work, "one@x").await, Ok(()));
    let events = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    assert!(events.occurrences.iter().all(|o| o.summary != "Planning!"));
    assert_eq!(
        h.service.delete_event(&work, "one@x").await,
        Err(CalendarError::NotFound)
    );
}

#[tokio::test]
async fn invalid_drafts_and_read_only_calendars_are_refused_before_any_request() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let calendars = h.service.calendars();
    let (work, holidays) = (calendars[0].id.clone(), calendars[1].id.clone());
    let before = h.fake.requests().len();
    assert_eq!(
        h.service.create_event(&holidays, &draft("x")).await.err(),
        Some(CalendarError::ReadOnly)
    );
    let mut bad = draft("x");
    bad.end = "2026-01-13T09:00:00".to_owned();
    assert_eq!(
        h.service.create_event(&work, &bad).await.err(),
        Some(CalendarError::InvalidInput)
    );
    let mut bad = draft("x");
    bad.rrule = Some("FREQ=SOMETIMES".to_owned());
    assert_eq!(
        h.service.create_event(&work, &bad).await.err(),
        Some(CalendarError::InvalidInput)
    );
    let mut bad = draft("x");
    bad.tzid = Some("Mars/Olympus".to_owned());
    assert_eq!(
        h.service.create_event(&work, &bad).await.err(),
        Some(CalendarError::InvalidInput)
    );
    let mut bad = draft("x");
    bad.start = "tomorrow".to_owned();
    assert_eq!(
        h.service.create_event(&work, &bad).await.err(),
        Some(CalendarError::InvalidInput)
    );
    assert_eq!(h.fake.requests().len(), before);
}

#[tokio::test]
async fn a_server_conflict_is_reported_not_overwritten() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "/dav/calendars/ann/work/one.ics",
        "",
        response(412, &[], ""),
    );
    assert_eq!(
        h.service
            .update_event(&work, "one@x", &draft("Mine"))
            .await
            .err(),
        Some(CalendarError::Conflict)
    );
    let events = h
        .service
        .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    assert_eq!(events.occurrences[0].summary, "Planning");
}

#[tokio::test]
async fn removing_an_account_deletes_its_secret_and_events() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let id = h.service.accounts()[0].id.clone();
    h.service.remove_account(&id).expect("removed");
    assert!(h.service.accounts().is_empty());
    assert!(h.service.calendars().is_empty());
    assert!(h.secrets.get(&id).expect("get").is_none());
    assert_eq!(h.service.remove_account(&id), Err(CalendarError::NotFound));
}

#[tokio::test]
async fn subscription_from_a_webcal_address_is_read_only_and_refreshes() {
    let h = harness();
    h.fake
        .route(Method::Get, "/holidays.ics", "", response(200, &[], FEED));
    let account = h
        .service
        .add_subscription("webcal://example.org/holidays.ics", "")
        .await
        .expect("subscription");
    assert_eq!(account.kind, CalendarAccountKindV1::Subscription);
    assert_eq!(account.name, "Public holidays");
    assert!(account.server_url.starts_with("https://"));
    let calendars = h.service.calendars();
    assert!(calendars[0].read_only);
    let events = h
        .service
        .events(utc("2026-01-01T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events");
    // One holiday plus the weekly event on Jan 5 and Jan 12 and Jan 19.
    assert_eq!(events.occurrences.len(), 4);
    assert!(events.occurrences[0].all_day);
    assert_eq!(events.occurrences[0].start, "2026-01-01");
    assert!(events.occurrences.iter().all(|o| o.read_only));
    assert_eq!(
        h.service
            .create_event(&calendars[0].id, &draft("x"))
            .await
            .err(),
        Some(CalendarError::ReadOnly)
    );

    // Fresh: periodic refresh leaves it alone. Forced refresh re-reads.
    let calls = h.fake.requests().len();
    assert!(h.service.sync_due().await.outcomes.is_empty());
    assert_eq!(h.fake.requests().len(), calls);
    let report = h.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, None);
    assert_eq!(h.fake.requests().len(), calls + 1);
    assert!(h.fake.requests().iter().all(|r| r.credentials.is_none()));
}

#[tokio::test]
async fn subscription_failures_and_unsafe_addresses() {
    let h = harness();
    assert_eq!(
        h.service
            .add_subscription("http://example.org/a.ics", "x")
            .await
            .err(),
        Some(CalendarError::Insecure)
    );
    assert_eq!(
        h.service
            .add_subscription("file:///etc/passwd", "x")
            .await
            .err(),
        Some(CalendarError::InvalidInput)
    );
    assert_eq!(
        h.service
            .add_subscription("https://user:pw@example.org/a.ics", "x")
            .await
            .err(),
        Some(CalendarError::InvalidInput)
    );
    h.fake.route(
        Method::Get,
        "/html",
        "",
        response(200, &[], "<html>nope</html>"),
    );
    assert_eq!(
        h.service
            .add_subscription("https://example.org/html", "x")
            .await
            .err(),
        Some(CalendarError::Parse)
    );
    h.fake
        .route(Method::Get, "/gone", "", response(404, &[], ""));
    assert_eq!(
        h.service
            .add_subscription("https://example.org/gone", "x")
            .await
            .err(),
        Some(CalendarError::NotFound)
    );
    assert!(h.service.accounts().is_empty());
}

#[test]
fn json_storage_round_trips_and_rejects_oversized_or_newer_files() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("calendar.json");
    let storage = JsonFileStorage::new(path.clone());
    assert_eq!(storage.load().expect("empty").accounts.len(), 0);
    let state = StoredState {
        version: 1,
        accounts: vec![crate::api::CalendarAccountV1 {
            id: "A1".to_owned(),
            kind: CalendarAccountKindV1::Subscription,
            name: "n".to_owned(),
            server_url: "https://example.org/a.ics".to_owned(),
            username: None,
        }],
        calendars: Vec::new(),
    };
    storage.save(&state).expect("save");
    assert_eq!(storage.load().expect("load"), state);
    std::fs::write(&path, "{\"version\":99}").expect("write");
    assert_eq!(storage.load().err(), Some(CalendarError::Storage));
    std::fs::write(&path, "not json").expect("write");
    assert_eq!(storage.load().err(), Some(CalendarError::Storage));
}

#[test]
fn vault_targets_use_the_calendar_service_name() {
    assert_eq!(
        crate::store::vault_target("01ABC").expect("target"),
        "app.zephium.calendar|01ABC"
    );
    assert_eq!(
        crate::store::vault_target("a|b"),
        Err(CalendarError::InvalidInput)
    );
}
