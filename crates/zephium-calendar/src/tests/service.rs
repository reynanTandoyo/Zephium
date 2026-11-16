use super::caldav::{multistatus, script_account, HEADER};
use super::{response, utc, Fake};
use crate::api::{CalendarAccountKindV1, CalendarEventDraftV1};
use crate::error::CalendarError;
use crate::http::{Method, Secret};
use crate::service::CalendarService;
use crate::store::{
    CalendarStorage, EventCache, JsonFileStorage, MemoryEventCache, MemorySecrets, MemoryStorage,
    SecretStore, StoredState,
};
use std::sync::Arc;

const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Public holidays\r\n\
BEGIN:VEVENT\r\nUID:h1@x\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20260101\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:h2@x\r\nSUMMARY:Weekly\r\nDTSTART:20260105T100000Z\r\nDTEND:20260105T110000Z\r\n\
RRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

pub(super) struct Harness {
    pub(super) fake: Arc<Fake>,
    storage: Arc<MemoryStorage>,
    secrets: Arc<MemorySecrets>,
    pub(super) service: CalendarService,
}

fn harness() -> Harness {
    harness_with(Arc::new(MemoryEventCache::default()))
}

fn harness_with(cache: Arc<dyn EventCache>) -> Harness {
    let fake = Arc::new(Fake::default());
    let storage = Arc::new(MemoryStorage::default());
    let secrets = Arc::new(MemorySecrets::default());
    let service = CalendarService::with_cache(
        fake.clone(),
        storage.clone(),
        secrets.clone(),
        cache,
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
        recurrence: None,
    }
}

pub(super) async fn signed_in() -> Harness {
    signed_in_on(harness(), work_report("Planning", "\"e1\"")).await
}

/// `work_report` for a daily series of five days.
fn series_report(etag: &str) -> String {
    multistatus(&format!(
        "<d:response><d:href>/dav/calendars/ann/work/one.ics</d:href><d:propstat><d:prop>\
         <d:getetag>{etag}</d:getetag><c:calendar-data>BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\n\
         UID:one@x\nSUMMARY:Planning\nDTSTART;TZID=Europe/Berlin:20260112T090000\n\
         DTEND;TZID=Europe/Berlin:20260112T100000\nRRULE:FREQ=DAILY;COUNT=5\nEND:VEVENT\n\
         END:VCALENDAR\n</c:calendar-data>\
         </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    ))
}

async fn signed_in_on(h: Harness, report: String) -> Harness {
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
        response(207, &[], &report),
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
            oauth: None,
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

fn last_put_body(h: &Harness) -> String {
    let put = h
        .fake
        .requests()
        .into_iter()
        .rfind(|r| r.method == Method::Put)
        .expect("a PUT");
    String::from_utf8_lossy(&put.body).into_owned()
}

fn series_draft(summary: &str, start_hour: u32) -> CalendarEventDraftV1 {
    CalendarEventDraftV1 {
        summary: summary.to_owned(),
        description: String::new(),
        location: String::new(),
        start: format!("2026-01-12T{start_hour:02}:00:00"),
        end: format!("2026-01-12T{:02}:00:00", start_hour + 1),
        all_day: false,
        tzid: Some("Europe/Berlin".to_owned()),
        rrule: Some("FREQ=DAILY;COUNT=5".to_owned()),
        reminders: Vec::new(),
        url: None,
        recurrence: None,
    }
}

fn week(h: &Harness) -> Vec<crate::api::CalendarOccurrenceV1> {
    h.service
        .events(utc("2026-01-11T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
        .expect("events")
        .occurrences
}

#[tokio::test]
async fn one_occurrence_can_be_edited_or_deleted_and_the_series_keeps_its_exceptions() {
    let h = signed_in_on(harness(), series_report("\"e1\"")).await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "/dav/calendars/ann/work/one.ics",
        "",
        response(204, &[("etag", "\"e2\"")], ""),
    );
    assert_eq!(week(&h).len(), 5);

    // Edit the third occurrence (Jan 14, 09:00 Berlin = 08:00Z).
    let mut draft = series_draft("Moved", 15);
    draft.start = "2026-01-14T15:00:00".to_owned();
    draft.end = "2026-01-14T16:00:00".to_owned();
    h.service
        .update_occurrence(&work, "one@x", "2026-01-14T08:00:00Z", &draft)
        .await
        .expect("occurrence edited");
    let body = last_put_body(&h);
    assert!(body.contains("BEGIN:VTIMEZONE"));
    assert!(body.contains("RRULE:FREQ=DAILY;COUNT=5"));
    assert_eq!(body.matches("BEGIN:VEVENT").count(), 2);
    assert!(body.contains("RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000"));
    assert!(body.contains("SUMMARY:Moved"));
    let put = h
        .fake
        .requests()
        .into_iter()
        .rfind(|r| r.method == Method::Put)
        .expect("put");
    assert!(put.headers.contains(&("if-match", "\"e1\"".to_owned())));
    let listed = week(&h);
    assert_eq!(listed.len(), 5);
    let moved: Vec<_> = listed.iter().filter(|o| o.summary == "Moved").collect();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].start, "2026-01-14T14:00:00Z");
    assert_eq!(
        moved[0].recurrence_id.as_deref(),
        Some("2026-01-14T08:00:00Z")
    );

    // Delete the fourth (Jan 15).
    h.service
        .delete_occurrence(&work, "one@x", "2026-01-15T08:00:00Z")
        .await
        .expect("occurrence deleted");
    let body = last_put_body(&h);
    assert!(body.contains("EXDATE;TZID=Europe/Berlin:20260115T090000"));
    assert!(body.contains("RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000"));
    assert_eq!(week(&h).len(), 4);
    // Deleting the same occurrence again adds no second EXDATE.
    h.service
        .delete_occurrence(&work, "one@x", "2026-01-15T08:00:00Z")
        .await
        .expect("again");
    assert_eq!(last_put_body(&h).matches("EXDATE").count(), 1);

    // Editing the series without moving its start keeps both exceptions.
    h.service
        .update_event(&work, "one@x", &series_draft("Series", 9))
        .await
        .expect("series edited");
    let body = last_put_body(&h);
    assert!(body.contains("EXDATE;TZID=Europe/Berlin:20260115T090000"));
    assert!(body.contains("RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000"));
    let listed = week(&h);
    assert_eq!(listed.len(), 4);
    assert!(listed.iter().any(|o| o.summary == "Moved"));
    assert!(listed.iter().any(|o| o.summary == "Series"));

    // Moving the series start drops the exceptions.
    h.service
        .update_event(&work, "one@x", &series_draft("Later", 11))
        .await
        .expect("moved series");
    let body = last_put_body(&h);
    assert!(!body.contains("RECURRENCE-ID") && !body.contains("EXDATE"));
    assert_eq!(week(&h).len(), 5);

    // Stopping the repetition drops what no longer refers to anything.
    h.service
        .update_occurrence(
            &work,
            "one@x",
            "2026-01-13T10:00:00Z",
            &series_draft("X", 11),
        )
        .await
        .expect("override");
    assert!(last_put_body(&h).contains("RECURRENCE-ID"));
    let mut once = series_draft("Once", 11);
    once.rrule = None;
    h.service
        .update_event(&work, "one@x", &once)
        .await
        .expect("single");
    let body = last_put_body(&h);
    assert!(!body.contains("RECURRENCE-ID") && !body.contains("EXDATE"));
    assert!(!body.contains("RRULE:FREQ=DAILY"));
}

#[tokio::test]
async fn occurrence_changes_refuse_bad_instances_and_read_only_calendars() {
    let h = signed_in_on(harness(), series_report("\"e1\"")).await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    let holidays = h.service.calendars()[1].id.clone();
    for bad in [
        "2026-01-14T09:30:00Z",
        "2026-02-14T08:00:00Z",
        "not a time",
        "2026-01-14",
        "",
    ] {
        assert_eq!(
            h.service
                .update_occurrence(&work, "one@x", bad, &series_draft("X", 9))
                .await
                .err(),
            Some(CalendarError::InvalidInput),
            "{bad:?}"
        );
        assert_eq!(
            h.service.delete_occurrence(&work, "one@x", bad).await.err(),
            Some(CalendarError::InvalidInput),
            "{bad:?}"
        );
    }
    assert_eq!(
        h.service
            .delete_occurrence(&work, "missing@x", "2026-01-14T08:00:00Z")
            .await
            .err(),
        Some(CalendarError::NotFound)
    );
    assert_eq!(
        h.service
            .delete_occurrence(&holidays, "one@x", "2026-01-14T08:00:00Z")
            .await
            .err(),
        Some(CalendarError::ReadOnly)
    );
    assert!(
        h.fake
            .requests()
            .iter()
            .all(|request| request.method != Method::Put),
        "nothing was sent"
    );
}

fn changed_report(href: &str, etag: &str, summary: &str) -> String {
    format!(
        "<d:response><d:href>/dav/calendars/ann/work/{href}</d:href><d:propstat><d:prop>\
         <d:getetag>{etag}</d:getetag><c:calendar-data>BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\n\
         UID:{href}@x\nSUMMARY:{summary}\nDTSTART:20260113T100000Z\nDTEND:20260113T110000Z\n\
         END:VEVENT\nEND:VCALENDAR\n</c:calendar-data></d:prop>\
         <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    )
}

fn etag_only(href: &str, etag: &str) -> String {
    format!(
        "<d:response><d:href>/dav/calendars/ann/work/{href}</d:href><d:propstat><d:prop>\
         <d:getetag>{etag}</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status>\
         </d:propstat></d:response>"
    )
}

fn state_response(ctag: &str, token: Option<&str>) -> String {
    let token = token
        .map(|token| format!("<d:sync-token>{token}</d:sync-token>"))
        .unwrap_or_default();
    format!(
        "{HEADER}<d:response><d:href>/dav/calendars/ann/work/</d:href><d:propstat><d:prop>\
         <cs:getctag>{ctag}</cs:getctag>{token}</d:prop><d:status>HTTP/1.1 200 OK</d:status>\
         </d:propstat></d:response></d:multistatus>"
    )
}

fn bodies_of(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .filter(|r| r.url.path() == "/dav/calendars/ann/work/")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

fn all_bodies_of(fake: &Fake) -> Vec<String> {
    fake.requests()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

fn script_delta(h: &Harness, ctag: &str, listing: &str, multiget: Option<String>) {
    h.fake.clear();
    h.fake.route(
        Method::Propfind,
        "/dav/calendars/ann/work/",
        "",
        response(207, &[], &state_response(ctag, None)),
    );
    if let Some(data) = multiget {
        h.fake.route(
            Method::Report,
            "/dav/calendars/ann/work/",
            "calendar-multiget",
            response(207, &[], &multistatus(&data)),
        );
    }
    h.fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "calendar-query",
        response(207, &[], &multistatus(listing)),
    );
}

#[tokio::test]
async fn without_a_sync_token_only_events_with_a_new_etag_are_fetched() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let day = || -> Vec<String> {
        h.service
            .events(utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"))
            .expect("events")
            .occurrences
            .into_iter()
            .map(|o| o.summary)
            .collect()
    };
    assert_eq!(day(), ["Planning"]);

    // The server lists one.ics unchanged and a new two.ics: only two.ics is
    // fetched.
    let first_read = h.fake.requests().len();
    script_delta(
        &h,
        "c2",
        &format!(
            "{}{}",
            etag_only("one.ics", "\"e1\""),
            etag_only("two.ics", "\"e9\"")
        ),
        Some(changed_report("two.ics", "\"e9\"", "Two")),
    );
    h.service.sync_all().await;
    let asked: Vec<String> = h.fake.requests()[first_read..]
        .iter()
        .filter(|r| r.url.path() == "/dav/calendars/ann/work/")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    let multiget: Vec<_> = asked
        .iter()
        .filter(|body| body.contains("calendar-multiget"))
        .collect();
    assert_eq!(multiget.len(), 1);
    assert!(multiget[0].contains("two.ics") && !multiget[0].contains("one.ics"));
    assert!(asked
        .iter()
        .filter(|body| body.contains("calendar-query"))
        .all(|body| !body.contains("calendar-data")));
    let summaries = day();
    assert_eq!(summaries.len(), 2);
    assert!(summaries.contains(&"Planning".to_owned()) && summaries.contains(&"Two".to_owned()));

    // Nothing changed: no data is fetched at all.
    let before = h.fake.requests().len();
    h.service.sync_all().await;
    assert!(all_bodies_of(&h.fake)[before..]
        .iter()
        .all(|body| !body.contains("calendar-multiget")));
    assert_eq!(day().len(), 2);

    // two.ics changed and one.ics vanished from the server.
    script_delta(
        &h,
        "c3",
        &etag_only("two.ics", "\"e10\""),
        Some(changed_report("two.ics", "\"e10\"", "Two v2")),
    );
    h.service.sync_all().await;
    assert_eq!(day(), ["Two v2"]);

    // A changed event whose data did not arrive does not pin its etag: the
    // next refresh asks for the data again.
    script_delta(&h, "c4", &etag_only("two.ics", "\"e11\""), None);
    h.service.sync_all().await;
    let before = h.fake.requests().len();
    h.service.sync_all().await;
    assert!(all_bodies_of(&h.fake)[before..]
        .iter()
        .any(|body| body.contains("calendar-multiget")));
}

#[tokio::test]
async fn a_restart_serves_saved_events_and_continues_with_the_saved_sync_token() {
    let cache: Arc<dyn EventCache> = Arc::new(MemoryEventCache::default());
    let first = signed_in_on(
        harness_with(cache.clone()),
        work_report("Planning", "\"e1\""),
    )
    .await;
    first.service.sync_all().await;
    let work = first.service.calendars()[0].id.clone();
    let range = (utc("2026-01-10T00:00:00Z"), utc("2026-01-20T00:00:00Z"));
    assert_eq!(
        first
            .service
            .events(range.0, range.1)
            .expect("events")
            .occurrences
            .len(),
        1
    );

    // A new service over the same durable state, with a server that answers
    // nothing.
    let fake = Arc::new(Fake::default());
    let second = CalendarService::with_cache(
        fake.clone(),
        first.storage.clone(),
        first.secrets.clone(),
        cache,
        Arc::new(|| utc("2026-01-10T12:00:00Z")),
    )
    .expect("restarted");
    second.warm();
    let occurrences = second.events(range.0, range.1).expect("events").occurrences;
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].summary, "Planning");
    assert_eq!(occurrences[0].calendar_id, work);
    assert!(fake.requests().is_empty(), "served without any request");
    // Fresh enough: a due-only refresh leaves the calendar alone.
    let report = second.sync_due().await;
    assert!(report.outcomes.iter().all(|o| o.calendar_id != work));

    // The next refresh asks for changes since the saved token and reads
    // nothing in full.
    fake.route(
        Method::Propfind,
        "/dav/calendars/ann/work/",
        "",
        response(207, &[], &state_response("c2", Some("t2"))),
    );
    fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "sync-collection",
        response(
            207,
            &[],
            &format!(
                "{HEADER}{}<d:sync-token>t2</d:sync-token></d:multistatus>",
                changed_report("one.ics", "\"e3\"", "Planning v3")
            ),
        ),
    );
    second.sync_all().await;
    let asked = bodies_of(&fake);
    assert!(asked
        .iter()
        .any(|b| b.contains("sync-collection") && b.contains(">t1<")));
    assert!(asked.iter().all(|b| !b.contains("calendar-query")));
    let occurrences = second.events(range.0, range.1).expect("events").occurrences;
    assert_eq!(occurrences[0].summary, "Planning v3");
}

#[tokio::test]
async fn occurrences_carry_the_series_and_the_whole_series_can_be_edited_from_them() {
    let h = signed_in_on(harness(), series_report("\"e1\"")).await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "/dav/calendars/ann/work/one.ics",
        "",
        response(204, &[("etag", "\"e2\"")], ""),
    );
    let listed = week(&h);
    assert_eq!(listed.len(), 5);
    let third = &listed[2];
    // Every instance reports the series as a whole.
    assert_eq!(third.recurrence_id.as_deref(), Some("2026-01-14T08:00:00Z"));
    assert_eq!(third.start, "2026-01-14T08:00:00Z");
    assert_eq!(third.series_start.as_deref(), Some("2026-01-12T08:00:00Z"));
    assert_eq!(third.series_end.as_deref(), Some("2026-01-12T09:00:00Z"));
    assert_eq!(third.series_tzid.as_deref(), Some("Europe/Berlin"));
    assert_eq!(third.rrule.as_deref(), Some("FREQ=DAILY;COUNT=5"));
    let rule = third.recurrence.clone().expect("simple rule");
    assert_eq!(
        (rule.freq.as_str(), rule.interval, rule.count),
        ("daily", 1, Some(5))
    );
    assert!(!rule.custom);

    // Edit one occurrence, delete another, then edit the series the way the
    // frame does: from the occurrence's own fields.
    h.service
        .update_occurrence(
            &work,
            "one@x",
            "2026-01-14T08:00:00Z",
            &series_draft("Moved", 15),
        )
        .await
        .expect("occurrence");
    h.service
        .delete_occurrence(&work, "one@x", "2026-01-15T08:00:00Z")
        .await
        .expect("deleted");
    let first = week(&h).remove(0);
    let mut draft = series_draft("Whole series", 9);
    draft.start = first.series_start.clone().expect("series start");
    draft.end = first.series_end.clone().expect("series end");
    draft.tzid = first.series_tzid.clone();
    draft.rrule = None;
    draft.recurrence = first.recurrence.clone();
    h.service
        .update_event(&work, "one@x", &draft)
        .await
        .expect("series");
    let body = last_put_body(&h);
    assert!(
        body.contains("RRULE:FREQ=DAILY;COUNT=5"),
        "the rule survives"
    );
    assert!(body.contains("DTSTART;TZID=Europe/Berlin:20260112T090000"));
    assert!(body.contains("EXDATE;TZID=Europe/Berlin:20260115T090000"));
    assert!(body.contains("RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000"));
    let listed = week(&h);
    assert_eq!(listed.len(), 4);
    assert!(listed.iter().any(|o| o.summary == "Moved"));
    assert_eq!(
        listed
            .iter()
            .filter(|o| o.summary == "Whole series")
            .count(),
        3
    );
    // The edited occurrence still names its series.
    let moved = listed.iter().find(|o| o.summary == "Moved").expect("moved");
    assert_eq!(moved.series_start.as_deref(), Some("2026-01-12T08:00:00Z"));
    assert_eq!(moved.recurrence_id.as_deref(), Some("2026-01-14T08:00:00Z"));

    // A new rule in simple form replaces the old one and wins over `rrule`.
    let mut weekly = series_draft("Weekly", 9);
    weekly.rrule = Some("FREQ=DAILY".to_owned());
    weekly.recurrence = Some(crate::api::CalendarRecurrenceV1 {
        freq: "weekly".to_owned(),
        interval: 2,
        count: None,
        until: Some("2026-03-01".to_owned()),
        by_day: vec!["MO".to_owned(), "TH".to_owned()],
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        custom: false,
    });
    h.service
        .update_event(&work, "one@x", &weekly)
        .await
        .expect("weekly");
    assert!(last_put_body(&h)
        .contains("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;UNTIL=20260301T225959Z"));
    // A malformed simple rule is refused before anything is sent.
    let puts = || {
        h.fake
            .requests()
            .iter()
            .filter(|r| r.method == Method::Put)
            .count()
    };
    let sent = puts();
    weekly.recurrence.as_mut().expect("rule").interval = 0;
    assert_eq!(
        h.service.update_event(&work, "one@x", &weekly).await.err(),
        Some(CalendarError::InvalidInput)
    );
    assert_eq!(puts(), sent);
}

#[tokio::test]
async fn a_single_event_reports_no_series() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let listed = week(&h);
    assert_eq!(listed.len(), 1);
    let only = &listed[0];
    assert!(!only.recurring && only.recurrence_id.is_none());
    assert!(only.series_start.is_none() && only.series_end.is_none());
    assert!(only.series_tzid.is_none() && only.rrule.is_none() && only.recurrence.is_none());
}

#[tokio::test]
async fn reminders_come_from_visible_calendars_with_the_default_offset() {
    let h = signed_in().await;
    h.service.sync_all().await;
    // "Planning" starts 08:00Z on 12 January; the clock reads 07:00Z.
    let now = utc("2026-01-12T07:00:00Z");
    assert!(h.service.reminders(now, None).is_empty());
    let found = h.service.reminders(now, Some(15));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "Planning");
    assert_eq!(found[0].trigger, utc("2026-01-12T07:45:00Z"));
    assert!(!found[0].all_day);
    assert!(found[0].key.starts_with("calendar|"));
    // A hidden calendar gives no reminders.
    let work = h.service.calendars()[0].clone();
    h.service.set_visible(&work.id, false).expect("hide");
    assert!(h.service.reminders(now, Some(15)).is_empty());
}
