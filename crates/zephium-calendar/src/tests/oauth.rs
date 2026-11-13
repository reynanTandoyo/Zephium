//! Google and Microsoft Graph accounts against scripted HTTP answers and a
//! fake token port: parsing, incremental sync, expired tokens, refresh and
//! error mapping. No test touches the network or a vault.

use super::{response, utc};
use crate::api::{CalendarAccountKindV1, CalendarAccountV1, CalendarEventDraftV1, CalendarOAuthV1};
use crate::error::{CalendarError, Result};
use crate::http::{HttpClient, HttpRequest, HttpResponse, Method};
use crate::model::TimeKind;
use crate::persist::StoreCalendar;
use crate::provider::{google, graph, Item};
use crate::service::CalendarService;
use crate::store::{CalendarStorage, MemoryEventCache, MemorySecrets, MemoryStorage, StoredState};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zephium_oauth::flow::Grant;
use zephium_oauth::manager::{AccessTokenSource, TokenSessions};
use zephium_oauth::provider::{ClientConfig, Provider, Purpose};
use zephium_oauth::{BoxFuture, OAuthError};
use zeroize::Zeroizing;

// -- fakes ---------------------------------------------------------------------

/// Answers requests from a queue, in order; an empty queue answers 404.
#[derive(Default)]
struct Script {
    queue: Mutex<VecDeque<HttpResponse>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl Script {
    fn push(&self, status: u16, body: &Value) {
        self.queue
            .lock()
            .unwrap()
            .push_back(response(status, &[], &body.to_string()));
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().unwrap().clone()
    }

    fn url(&self, index: usize) -> String {
        self.requests()[index].url.to_string()
    }

    fn bearer(&self, index: usize) -> String {
        self.requests()[index]
            .bearer
            .as_ref()
            .map(|token| token.expose().to_owned())
            .unwrap_or_default()
    }

    fn body(&self, index: usize) -> Value {
        serde_json::from_slice(&self.requests()[index].body).unwrap_or(Value::Null)
    }
}

#[async_trait]
impl HttpClient for Script {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        self.seen.lock().unwrap().push(request);
        Ok(self
            .queue
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| response(404, &[], "")))
    }
}

/// Hands out `tok-<generation>`; `invalidate` starts a new generation, like a
/// refresh would.
#[derive(Default)]
struct FakeTokens {
    generation: AtomicUsize,
    adopted: Mutex<Vec<String>>,
    signed_out: Mutex<Vec<String>>,
    fail: Mutex<Option<OAuthError>>,
}

impl AccessTokenSource for FakeTokens {
    fn access_token<'a>(
        &'a self,
        _account_id: &'a str,
        _config: &'a ClientConfig,
        purpose: Purpose,
    ) -> BoxFuture<'a, zephium_oauth::Result<Zeroizing<String>>> {
        Box::pin(async move {
            assert_eq!(purpose, Purpose::Calendar);
            if let Some(error) = *self.fail.lock().unwrap() {
                return Err(error);
            }
            Ok(Zeroizing::new(format!(
                "tok-{}",
                self.generation.load(Ordering::SeqCst)
            )))
        })
    }
}

impl TokenSessions for FakeTokens {
    fn adopt<'a>(
        &'a self,
        account_id: &'a str,
        _grant: &'a Grant,
    ) -> BoxFuture<'a, zephium_oauth::Result<()>> {
        Box::pin(async move {
            self.adopted.lock().unwrap().push(account_id.to_owned());
            Ok(())
        })
    }

    fn invalidate<'a>(&'a self, _account_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.generation.fetch_add(1, Ordering::SeqCst);
        })
    }

    fn sign_out<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, zephium_oauth::Result<()>> {
        Box::pin(async move {
            self.signed_out.lock().unwrap().push(account_id.to_owned());
            Ok(())
        })
    }
}

struct Rig {
    http: Arc<Script>,
    tokens: Arc<FakeTokens>,
    service: CalendarService,
}

fn rig() -> Rig {
    let http = Arc::new(Script::default());
    let tokens = Arc::new(FakeTokens::default());
    let service = CalendarService::with_cache(
        http.clone(),
        Arc::new(MemoryStorage::default()),
        Arc::new(MemorySecrets::default()),
        Arc::new(MemoryEventCache::default()),
        Arc::new(|| utc("2026-01-10T12:00:00Z")),
    )
    .expect("service")
    .with_oauth(tokens.clone());
    Rig {
        http,
        tokens,
        service,
    }
}

fn grant(provider: Provider, purposes: Vec<Purpose>) -> Grant {
    Grant {
        config: ClientConfig {
            provider,
            client_id: "client-1".to_owned(),
            tenant: None,
        },
        client_secret: None,
        access_token: Zeroizing::new("first".to_owned()),
        refresh_token: Zeroizing::new("refresh".to_owned()),
        lifetime: Duration::from_secs(3600),
        purposes,
    }
}

fn google_calendars() -> Value {
    json!({"items": [
        {"id": "ann@example.com", "summary": "Ann", "accessRole": "owner",
         "primary": true, "backgroundColor": "#9fe1e7"},
        {"id": "en.german#holiday@group.v.calendar.google.com", "summary": "Holidays",
         "accessRole": "reader"},
        {"id": "busy@example.com", "summary": "Busy", "accessRole": "freeBusyReader"}
    ]})
}

async fn google_rig() -> Rig {
    let rig = rig();
    rig.http.push(200, &google_calendars());
    rig.service
        .add_oauth_account(&grant(Provider::Google, vec![Purpose::Calendar]), None)
        .await
        .expect("account");
    rig
}

fn berlin(date_time: &str) -> Value {
    json!({"dateTime": date_time, "timeZone": "Europe/Berlin"})
}

fn google_first_sync() -> (Value, Value) {
    (
        json!({"items": [
            {"id": "hol1", "status": "confirmed", "summary": "Holiday", "etag": "\"e1\"",
             "start": {"date": "2026-01-12"}, "end": {"date": "2026-01-13"}}
        ], "nextPageToken": "P2"}),
        json!({"items": [
            {"id": "wk", "summary": "Standup", "etag": "\"w1\"",
             "start": berlin("2026-01-12T09:00:00+01:00"),
             "end": berlin("2026-01-12T09:30:00+01:00"),
             "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=MO",
                            "EXDATE;TZID=Europe/Berlin:20260126T090000"]},
            {"id": "wk_20260119T080000Z", "summary": "Standup (moved)",
             "recurringEventId": "wk",
             "originalStartTime": berlin("2026-01-19T09:00:00+01:00"),
             "start": berlin("2026-01-19T10:00:00+01:00"),
             "end": berlin("2026-01-19T10:30:00+01:00")}
        ], "nextSyncToken": "S1"}),
    )
}

fn summaries(rig: &Rig, from: &str, to: &str) -> Vec<String> {
    rig.service
        .events(utc(from), utc(to))
        .expect("events")
        .occurrences
        .into_iter()
        .map(|o| format!("{} {}", o.summary, o.start))
        .collect()
}

fn draft(summary: &str, rrule: Option<&str>) -> CalendarEventDraftV1 {
    CalendarEventDraftV1 {
        summary: summary.to_owned(),
        description: "Notes".to_owned(),
        location: "Room 1".to_owned(),
        start: "2026-01-13T10:00:00".to_owned(),
        end: "2026-01-13T11:00:00".to_owned(),
        all_day: false,
        tzid: Some("Europe/Berlin".to_owned()),
        rrule: rrule.map(str::to_owned),
        reminders: vec![10],
        recurrence: None,
        url: None,
    }
}

// -- pure parsing -----------------------------------------------------------------

#[test]
fn google_events_map_all_day_zoned_recurring_and_exceptions() {
    let all_day = json!({"id": "a", "summary": "Day", "start": {"date": "2026-03-01"},
                         "end": {"date": "2026-03-02"}});
    let Some(Item::Event { event, .. }) = google::parse_event(&all_day) else {
        panic!("event");
    };
    assert!(event.start.is_date() && event.end.local.to_string().starts_with("2026-03-02"));

    let timed = json!({"id": "t", "summary": "Call", "status": "tentative",
        "start": berlin("2026-03-10T09:00:00+01:00"), "end": berlin("2026-03-10T10:00:00+01:00"),
        "attendees": [{"email": "bo@example.com", "responseStatus": "accepted"}, {"displayName": "x"}],
        "reminders": {"useDefault": false, "overrides": [{"method": "popup", "minutes": 15}]},
        "hangoutLink": "https://meet.google.com/abc"});
    let Some(Item::Event { event, .. }) = google::parse_event(&timed) else {
        panic!("event");
    };
    assert!(matches!(event.start.kind, TimeKind::Zoned(_)));
    assert_eq!(event.start.local.to_string(), "2026-03-10 09:00:00");
    assert_eq!(event.attendees.len(), 1);
    assert_eq!(event.alarms[0].minutes_before, 15);
    assert_eq!(event.url.as_deref(), Some("https://meet.google.com/abc"));

    // No zone name: the offset decides, the instant is kept as UTC.
    let offset = json!({"id": "o", "start": {"dateTime": "2026-03-10T09:00:00+02:00"},
                        "end": {"dateTime": "2026-03-10T10:00:00+02:00"}});
    let Some(Item::Event { event, .. }) = google::parse_event(&offset) else {
        panic!("event");
    };
    assert_eq!(event.start.kind, TimeKind::Utc);
    assert_eq!(event.start.local.to_string(), "2026-03-10 07:00:00");

    let (_, second) = google_first_sync();
    let items = second["items"].as_array().unwrap();
    let Some(Item::Event { event, .. }) = google::parse_event(&items[0]) else {
        panic!("master");
    };
    assert_eq!(event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO"));
    assert_eq!(event.exdates.len(), 1);
    let Some(Item::Occurrence {
        series,
        event: Some(moved),
        ..
    }) = google::parse_event(&items[1])
    else {
        panic!("exception");
    };
    assert_eq!(series, "wk");
    assert_eq!(moved.uid, "wk");
    assert!(moved.recurrence_id.is_some());

    let cancelled = json!({"id": "wk_x", "status": "cancelled", "recurringEventId": "wk",
        "originalStartTime": berlin("2026-02-02T09:00:00+01:00")});
    assert!(matches!(
        google::parse_event(&cancelled),
        Some(Item::Occurrence { event: None, .. })
    ));
    assert!(matches!(
        google::parse_event(&json!({"id": "hol1", "status": "cancelled"})),
        Some(Item::Gone { .. })
    ));
    // Malformed items are skipped, not fatal.
    assert!(google::parse_event(&json!({"id": "bad", "start": {"date": "nope"}})).is_none());
    assert!(google::parse_event(&json!({"summary": "no id"})).is_none());
}

#[test]
fn google_write_bodies_carry_times_recurrence_and_reminders() {
    let (_, second) = google_first_sync();
    let Some(Item::Event { event, .. }) = google::parse_event(&second["items"][0]) else {
        panic!("master");
    };
    let body = google::event_body(&event, true);
    assert_eq!(
        body["recurrence"],
        json!([
            "RRULE:FREQ=WEEKLY;BYDAY=MO",
            "EXDATE;TZID=Europe/Berlin:20260126T090000"
        ])
    );
    assert_eq!(body["start"]["timeZone"], "Europe/Berlin");
    assert_eq!(body["start"]["dateTime"], "2026-01-12T09:00:00");
    assert_eq!(body["start"]["date"], Value::Null);
    let occurrence = google::event_body(&event, false);
    assert!(occurrence.get("recurrence").is_none());
    let all_day = google::parse_event(
        &json!({"id": "a", "start": {"date": "2026-03-01"}, "end": {"date": "2026-03-02"}}),
    );
    let Some(Item::Event { event, .. }) = all_day else {
        panic!("event");
    };
    assert_eq!(
        google::event_body(&event, true)["start"]["date"],
        "2026-03-01"
    );
    assert_eq!(google::event_body(&event, true)["recurrence"], Value::Null);
    assert_eq!(
        google::instance_id(
            "wk",
            false,
            &crate::model::EventTime::utc(utc("2026-01-19T08:00:00Z"))
        ),
        "wk_20260119T080000Z"
    );
}

fn graph_event(id: &str, subject: &str, start: &str, end: &str) -> Value {
    json!({"id": id, "subject": subject, "type": "singleInstance", "isAllDay": false,
           "start": {"dateTime": format!("{start}.0000000"), "timeZone": "UTC"},
           "end": {"dateTime": format!("{end}.0000000"), "timeZone": "UTC"}})
}

#[test]
fn graph_events_map_all_day_removed_and_series_masters() {
    let Some(Item::Event { event, .. }) = graph::parse_event(&graph_event(
        "e1",
        "Review",
        "2026-01-13T09:00:00",
        "2026-01-13T10:00:00",
    )) else {
        panic!("event");
    };
    assert_eq!(event.start.kind, TimeKind::Utc);

    let all_day = json!({"id": "d", "subject": "Offsite", "isAllDay": true,
        "start": {"dateTime": "2026-01-14T00:00:00.0000000", "timeZone": "UTC"},
        "end": {"dateTime": "2026-01-15T00:00:00.0000000", "timeZone": "UTC"}});
    let Some(Item::Event { event, .. }) = graph::parse_event(&all_day) else {
        panic!("event");
    };
    assert!(event.start.is_date());
    assert_eq!(event.end.local.to_string(), "2026-01-15 00:00:00");

    let zoned = json!({"id": "z", "subject": "Berlin", "isAllDay": false,
        "start": {"dateTime": "2026-01-14T09:00:00.0000000", "timeZone": "W. Europe Standard Time"},
        "end": {"dateTime": "2026-01-14T10:00:00.0000000", "timeZone": "W. Europe Standard Time"}});
    let Some(Item::Event { event, .. }) = graph::parse_event(&zoned) else {
        panic!("event");
    };
    assert!(matches!(event.start.kind, TimeKind::Zoned(_)));
    let unknown = json!({"id": "u", "isAllDay": false,
        "start": {"dateTime": "2026-01-14T09:00:00.0000000", "timeZone": "Nowhere Standard Time"},
        "end": {"dateTime": "2026-01-14T10:00:00.0000000", "timeZone": "Nowhere Standard Time"}});
    assert!(graph::parse_event(&unknown).is_none());

    assert!(matches!(
        graph::parse_event(&json!({"id": "r", "@removed": {"reason": "deleted"}})),
        Some(Item::Gone { .. })
    ));
    let mut cancelled = graph_event("c", "x", "2026-01-13T09:00:00", "2026-01-13T10:00:00");
    cancelled["isCancelled"] = json!(true);
    assert!(matches!(
        graph::parse_event(&cancelled),
        Some(Item::Gone { .. })
    ));

    let mut master = graph_event("m", "Weekly", "2026-01-13T09:00:00", "2026-01-13T10:00:00");
    master["type"] = json!("seriesMaster");
    master["recurrence"] = json!({
        "pattern": {"type": "relativeMonthly", "interval": 1,
                    "daysOfWeek": ["tuesday"], "index": "second"},
        "range": {"type": "numbered", "startDate": "2026-01-13", "numberOfOccurrences": 6}});
    master["cancelledOccurrences"] = json!(["OID.AAA=.2026-02-10"]);
    let Some(Item::Event { event, .. }) = graph::parse_event(&master) else {
        panic!("master");
    };
    assert_eq!(
        event.rrule.as_deref(),
        Some("FREQ=MONTHLY;BYDAY=2TU;COUNT=6")
    );
    assert_eq!(event.exdates.len(), 1);
    assert_eq!(event.exdates[0].local.to_string(), "2026-02-10 09:00:00");

    master["recurrence"]["pattern"] = json!({"type": "weekly", "interval": 2,
        "daysOfWeek": ["monday", "friday"], "firstDayOfWeek": "sunday"});
    master["recurrence"]["range"] = json!({"type": "endDate", "startDate": "2026-01-13",
                                           "endDate": "2026-06-30"});
    let Some(Item::Event { event, .. }) = graph::parse_event(&master) else {
        panic!("master");
    };
    assert_eq!(
        event.rrule.as_deref(),
        Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;UNTIL=20260630T235959Z")
    );
}

#[test]
fn rrules_map_to_graph_patterns_and_unsupported_ones_are_refused() {
    let start = crate::model::EventTime::utc(utc("2026-01-13T09:00:00Z"));
    let weekly = graph::recurrence_json("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=5", &start).unwrap();
    assert_eq!(weekly["pattern"]["type"], "weekly");
    assert_eq!(
        weekly["pattern"]["daysOfWeek"],
        json!(["monday", "wednesday"])
    );
    assert_eq!(weekly["range"]["type"], "numbered");
    assert_eq!(weekly["range"]["numberOfOccurrences"], 5);
    // A weekly rule without BYDAY repeats on the start's weekday (a Tuesday).
    let plain = graph::recurrence_json("FREQ=WEEKLY", &start).unwrap();
    assert_eq!(plain["pattern"]["daysOfWeek"], json!(["tuesday"]));
    assert_eq!(plain["range"]["type"], "noEnd");
    let last_friday = graph::recurrence_json("FREQ=MONTHLY;BYDAY=-1FR", &start).unwrap();
    assert_eq!(last_friday["pattern"]["type"], "relativeMonthly");
    assert_eq!(last_friday["pattern"]["index"], "last");
    let monthly = graph::recurrence_json("FREQ=MONTHLY;INTERVAL=2", &start).unwrap();
    assert_eq!(monthly["pattern"]["type"], "absoluteMonthly");
    assert_eq!(monthly["pattern"]["dayOfMonth"], 13);
    let yearly = graph::recurrence_json("FREQ=YEARLY;UNTIL=20300101", &start).unwrap();
    assert_eq!(yearly["pattern"]["type"], "absoluteYearly");
    assert_eq!(yearly["pattern"]["month"], 1);
    assert_eq!(yearly["range"]["endDate"], "2030-01-01");
    for rule in [
        "FREQ=MONTHLY;BYMONTHDAY=1,15",
        "FREQ=MONTHLY;BYDAY=MO,TU;BYSETPOS=1",
        "FREQ=HOURLY",
        "FREQ=MONTHLY;BYMONTHDAY=-1",
    ] {
        assert_eq!(
            graph::recurrence_json(rule, &start).unwrap_err(),
            CalendarError::Unsupported,
            "{rule}"
        );
    }
}

// -- Google service ---------------------------------------------------------------

#[tokio::test]
async fn a_google_account_lists_calendars_and_keeps_the_oauth_client() {
    let rig = google_rig().await;
    let accounts = rig.service.accounts();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].kind, CalendarAccountKindV1::Google);
    assert_eq!(accounts[0].username.as_deref(), Some("ann@example.com"));
    assert_eq!(
        accounts[0].oauth,
        Some(CalendarOAuthV1 {
            client_id: "client-1".to_owned(),
            tenant: None
        })
    );
    // The free/busy-only calendar is skipped; the reader calendar is read-only.
    let calendars = rig.service.calendars();
    assert_eq!(calendars.len(), 2);
    assert!(!calendars[0].read_only && calendars[1].read_only);
    assert_eq!(calendars[0].color.as_deref(), Some("#9FE1E7"));
    assert!(calendars[1].url.contains("en.german%23holiday%40group"));
    assert_eq!(rig.tokens.adopted.lock().unwrap().len(), 1);
    assert_eq!(rig.http.bearer(0), "tok-0");

    // A second sign-in of the same address is refused and signs itself out.
    rig.http.push(200, &google_calendars());
    let again = rig
        .service
        .add_oauth_account(&grant(Provider::Google, vec![Purpose::Calendar]), None)
        .await;
    assert_eq!(again.unwrap_err(), CalendarError::InvalidInput);
    assert_eq!(rig.tokens.signed_out.lock().unwrap().len(), 1);
    assert_eq!(rig.service.accounts().len(), 1);
}

#[tokio::test]
async fn a_grant_without_the_calendar_purpose_is_refused() {
    let rig = rig();
    let result = rig
        .service
        .add_oauth_account(&grant(Provider::Google, vec![Purpose::Mail]), None)
        .await;
    assert_eq!(result.unwrap_err(), CalendarError::InvalidInput);
    assert!(rig.tokens.adopted.lock().unwrap().is_empty());
    assert!(rig.http.requests().is_empty());
}

#[tokio::test]
async fn a_failed_listing_leaves_no_account_and_no_tokens() {
    let rig = rig();
    rig.http.push(403, &json!({}));
    let result = rig
        .service
        .add_oauth_account(&grant(Provider::Google, vec![Purpose::Calendar]), None)
        .await;
    assert_eq!(result.unwrap_err(), CalendarError::Forbidden);
    assert!(rig.service.accounts().is_empty());
    assert_eq!(rig.tokens.signed_out.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn google_full_then_incremental_sync_folds_exceptions_and_deletions() {
    let rig = google_rig().await;
    let (page1, page2) = google_first_sync();
    rig.http.push(200, &page1);
    rig.http.push(200, &page2);
    rig.http
        .push(200, &json!({"items": [], "nextSyncToken": "H1"}));
    let report = rig.service.sync_all().await;
    assert!(
        report.outcomes.iter().all(|o| o.error.is_none()),
        "{report:?}"
    );
    let first_events = rig.http.url(1);
    assert!(first_events.contains("singleEvents=false") && !first_events.contains("syncToken"));
    assert!(rig.http.url(2).contains("pageToken=P2"));
    assert_eq!(
        summaries(&rig, "2026-01-12T00:00:00Z", "2026-02-02T00:00:00Z"),
        [
            "Holiday 2026-01-12",
            "Standup 2026-01-12T08:00:00Z",
            "Standup (moved) 2026-01-19T09:00:00Z",
        ]
    );

    // Incremental: the 12 January standup is cancelled, the holiday deleted,
    // the moved one renamed.
    rig.http.push(
        200,
        &json!({"items": [
            {"id": "wk_20260112T080000Z", "status": "cancelled", "recurringEventId": "wk",
             "originalStartTime": berlin("2026-01-12T09:00:00+01:00")},
            {"id": "hol1", "status": "cancelled"},
            {"id": "wk_20260119T080000Z", "summary": "Standup (renamed)",
             "recurringEventId": "wk",
             "originalStartTime": berlin("2026-01-19T09:00:00+01:00"),
             "start": berlin("2026-01-19T10:00:00+01:00"),
             "end": berlin("2026-01-19T10:30:00+01:00")}
        ], "nextSyncToken": "S2"}),
    );
    rig.service.sync_all().await;
    let url = rig.http.url(rig.http.requests().len() - 2);
    assert!(url.contains("syncToken=S1"), "{url}");
    assert_eq!(
        summaries(&rig, "2026-01-12T00:00:00Z", "2026-02-02T00:00:00Z"),
        ["Standup (renamed) 2026-01-19T09:00:00Z"]
    );
}

#[tokio::test]
async fn an_expired_sync_token_falls_back_to_a_full_sync() {
    let rig = google_rig().await;
    let (page1, page2) = google_first_sync();
    rig.http.push(200, &page1);
    rig.http.push(200, &page2);
    rig.http.push(200, &json!({"items": []}));
    rig.service.sync_all().await;
    let before = rig.http.requests().len();

    // Token S1 is stale: 410 fullSyncRequired, then the whole calendar again
    // (the holiday of the first sync is no longer there).
    rig.http.push(
        410,
        &json!({"error": {"code": 410, "errors": [{"reason": "fullSyncRequired"}]}}),
    );
    rig.http.push(
        200,
        &json!({"items": [{"id": "new", "summary": "Fresh",
            "start": {"date": "2026-01-20"}, "end": {"date": "2026-01-21"}}],
            "nextSyncToken": "S9"}),
    );
    rig.http.push(200, &json!({"items": []}));
    let report = rig.service.sync_all().await;
    assert!(
        report.outcomes.iter().all(|o| o.error.is_none()),
        "{report:?}"
    );
    assert!(rig.http.url(before).contains("syncToken=S1"));
    assert!(!rig.http.url(before + 1).contains("syncToken"));
    assert_eq!(
        summaries(&rig, "2026-01-01T00:00:00Z", "2026-02-02T00:00:00Z"),
        ["Fresh 2026-01-20"]
    );
}

#[tokio::test]
async fn a_401_refreshes_the_token_once_and_retries() {
    let rig = google_rig().await;
    rig.http.push(401, &json!({}));
    rig.http
        .push(200, &json!({"items": [], "nextSyncToken": "S1"}));
    rig.http.push(200, &json!({"items": []}));
    let report = rig.service.sync_all().await;
    assert!(
        report.outcomes.iter().all(|o| o.error.is_none()),
        "{report:?}"
    );
    // Requests 1 and 2 are the same call: first with the old token, then with
    // the refreshed one.
    assert_eq!(rig.http.bearer(1), "tok-0");
    assert_eq!(rig.http.bearer(2), "tok-1");
    assert_eq!(rig.http.url(1), rig.http.url(2));

    // A second 401 is the answer.
    rig.http.push(401, &json!({}));
    rig.http.push(401, &json!({}));
    rig.http.push(200, &json!({"items": []}));
    let report = rig.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, Some(CalendarError::Unauthorized));
}

#[tokio::test]
async fn failures_map_to_the_closed_error_set() {
    let rig = google_rig().await;
    for (status, expected) in [
        (403, CalendarError::Forbidden),
        (404, CalendarError::NotFound),
        (429, CalendarError::Limit),
        (500, CalendarError::Server),
    ] {
        rig.http.push(status, &json!({"error": "secret text"}));
        let report = rig.service.sync_all().await;
        assert_eq!(report.outcomes[0].error, Some(expected), "{status}");
    }
    rig.http.push(200, &json!("not an object"));
    let report = rig.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, Some(CalendarError::Parse));

    *rig.tokens.fail.lock().unwrap() = Some(OAuthError::SignInRequired);
    let report = rig.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, Some(CalendarError::Unauthorized));
    *rig.tokens.fail.lock().unwrap() = Some(OAuthError::Offline);
    let report = rig.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, Some(CalendarError::Network));
}

#[tokio::test]
async fn google_writes_use_the_api_and_update_the_cache() {
    let rig = google_rig().await;
    let (page1, page2) = google_first_sync();
    rig.http.push(200, &page1);
    rig.http.push(200, &page2);
    rig.http.push(200, &json!({"items": []}));
    rig.service.sync_all().await;
    let calendar = rig.service.calendars()[0].id.clone();

    // Create.
    rig.http.push(
        200,
        &json!({"id": "created1", "etag": "\"c1\"", "summary": "Planning",
                "start": berlin("2026-01-13T10:00:00+01:00"),
                "end": berlin("2026-01-13T11:00:00+01:00")}),
    );
    let created = rig
        .service
        .create_event(&calendar, &draft("Planning", None))
        .await
        .expect("create");
    assert_eq!(created.uid, "created1");
    let at = rig.http.requests().len() - 1;
    assert_eq!(rig.http.requests()[at].method, Method::Post);
    assert!(rig.http.url(at).ends_with("/ann%40example.com/events"));
    assert_eq!(rig.http.body(at)["summary"], "Planning");
    assert_eq!(
        rig.http.body(at)["reminders"]["overrides"][0]["minutes"],
        10
    );
    assert!(
        summaries(&rig, "2026-01-13T00:00:00Z", "2026-01-14T00:00:00Z")
            .contains(&"Planning 2026-01-13T09:00:00Z".to_owned())
    );

    // Update: guarded by the ETag the cache holds.
    rig.http.push(
        200,
        &json!({"id": "created1", "etag": "\"c2\"", "summary": "Planning 2",
                "start": berlin("2026-01-13T10:00:00+01:00"),
                "end": berlin("2026-01-13T11:00:00+01:00")}),
    );
    rig.service
        .update_event(&calendar, "created1", &draft("Planning 2", None))
        .await
        .expect("update");
    let at = rig.http.requests().len() - 1;
    assert_eq!(rig.http.requests()[at].method, Method::Patch);
    assert!(rig.http.requests()[at]
        .headers
        .contains(&("if-match", "\"c1\"".to_owned())));

    // A concurrent change is a conflict.
    rig.http.push(412, &json!({}));
    let conflict = rig
        .service
        .update_event(&calendar, "created1", &draft("Planning 3", None))
        .await;
    assert_eq!(conflict.unwrap_err(), CalendarError::Conflict);

    // One occurrence of the series: the instance id is built from its start.
    rig.http.push(
        200,
        &json!({"id": "wk_20260112T080000Z", "summary": "Standup (short)",
                "recurringEventId": "wk",
                "originalStartTime": berlin("2026-01-12T09:00:00+01:00"),
                "start": berlin("2026-01-12T09:00:00+01:00"),
                "end": berlin("2026-01-12T09:15:00+01:00")}),
    );
    let mut occurrence = draft("Standup (short)", None);
    occurrence.start = "2026-01-12T09:00:00".to_owned();
    occurrence.end = "2026-01-12T09:15:00".to_owned();
    rig.service
        .update_occurrence(&calendar, "wk", "2026-01-12T08:00:00Z", &occurrence)
        .await
        .expect("occurrence");
    let at = rig.http.requests().len() - 1;
    assert!(rig.http.url(at).ends_with("/events/wk_20260112T080000Z"));
    assert!(
        summaries(&rig, "2026-01-12T00:00:00Z", "2026-01-13T00:00:00Z")
            .contains(&"Standup (short) 2026-01-12T08:00:00Z".to_owned())
    );

    // Delete one occurrence, then a whole event.
    rig.http.push(204, &Value::Null);
    rig.service
        .delete_occurrence(&calendar, "wk", "2026-01-19T08:00:00Z")
        .await
        .expect("delete occurrence");
    let at = rig.http.requests().len() - 1;
    assert_eq!(rig.http.requests()[at].method, Method::Delete);
    assert!(rig.http.url(at).ends_with("/events/wk_20260119T080000Z"));
    rig.http.push(204, &Value::Null);
    rig.service
        .delete_event(&calendar, "created1")
        .await
        .expect("delete");
    assert!(
        !summaries(&rig, "2026-01-13T00:00:00Z", "2026-01-14T00:00:00Z")
            .iter()
            .any(|line| line.starts_with("Planning"))
    );

    // Read-only calendars refuse writes without any request.
    let holidays = rig.service.calendars()[1].id.clone();
    let before = rig.http.requests().len();
    let refused = rig
        .service
        .create_event(&holidays, &draft("No", None))
        .await;
    assert_eq!(refused.unwrap_err(), CalendarError::ReadOnly);
    assert_eq!(rig.http.requests().len(), before);
}

// -- Microsoft Graph service ------------------------------------------------------

const GRAPH_CAL: &str = "https://graph.microsoft.com/v1.0/me/calendars/AAA%3D";

async fn graph_rig() -> Rig {
    let rig = rig();
    rig.http.push(
        200,
        &json!({"value": [{"id": "AAA=", "name": "Calendar", "hexColor": "#0078D4",
                           "canEdit": true,
                           "owner": {"name": "Ann", "address": "ann@contoso.com"}}]}),
    );
    let mut g = grant(Provider::Microsoft, vec![Purpose::Calendar]);
    g.config.tenant = Some("contoso.onmicrosoft.com".to_owned());
    rig.service
        .add_oauth_account(&g, Some("Work"))
        .await
        .expect("account");
    rig
}

fn delta_link(token: &str) -> String {
    format!("{GRAPH_CAL}/calendarView/delta?$deltatoken={token}")
}

#[tokio::test]
async fn graph_delta_sync_applies_changes_and_removals() {
    let rig = graph_rig().await;
    let accounts = rig.service.accounts();
    assert_eq!(accounts[0].kind, CalendarAccountKindV1::Microsoft);
    assert_eq!(accounts[0].name, "Work");
    assert_eq!(
        accounts[0].oauth.as_ref().and_then(|o| o.tenant.as_deref()),
        Some("contoso.onmicrosoft.com")
    );
    rig.http.push(
        200,
        &json!({"value": [graph_event("e1", "Review", "2026-01-13T09:00:00", "2026-01-13T10:00:00")],
                "@odata.nextLink": format!("{GRAPH_CAL}/calendarView/delta?$skiptoken=x")}),
    );
    rig.http.push(
        200,
        &json!({"value": [
            json!({"id": "d", "subject": "Offsite", "isAllDay": true,
                   "start": {"dateTime": "2026-01-14T00:00:00.0000000", "timeZone": "UTC"},
                   "end": {"dateTime": "2026-01-15T00:00:00.0000000", "timeZone": "UTC"}}),
            graph_event("oc1", "Weekly", "2026-01-15T09:00:00", "2026-01-15T09:30:00"),
        ], "@odata.deltaLink": delta_link("d1")}),
    );
    let report = rig.service.sync_all().await;
    assert!(
        report.outcomes.iter().all(|o| o.error.is_none()),
        "{report:?}"
    );
    let first = rig.http.url(1);
    assert!(
        first.contains("/calendarView/delta?startDateTime=2025-07-14"),
        "{first}"
    );
    assert!(first.contains("endDateTime=2027-01-10"), "{first}");
    assert!(rig.http.requests()[1]
        .headers
        .iter()
        .any(|(name, value)| *name == "prefer" && value.contains("outlook.timezone=\"UTC\"")));
    assert_eq!(
        summaries(&rig, "2026-01-12T00:00:00Z", "2026-01-20T00:00:00Z"),
        [
            "Review 2026-01-13T09:00:00Z",
            "Offsite 2026-01-14",
            "Weekly 2026-01-15T09:00:00Z"
        ]
    );

    // Incremental: the saved delta link is followed as it is.
    rig.http.push(
        200,
        &json!({"value": [
            {"id": "e1", "@removed": {"reason": "deleted"}},
            graph_event("oc1", "Weekly (moved)", "2026-01-15T10:00:00", "2026-01-15T10:30:00"),
        ], "@odata.deltaLink": delta_link("d2")}),
    );
    rig.service.sync_all().await;
    assert_eq!(
        rig.http.url(rig.http.requests().len() - 1),
        delta_link("d1")
    );
    assert_eq!(
        summaries(&rig, "2026-01-12T00:00:00Z", "2026-01-20T00:00:00Z"),
        ["Offsite 2026-01-14", "Weekly (moved) 2026-01-15T10:00:00Z"]
    );
}

#[tokio::test]
async fn an_expired_graph_delta_link_falls_back_to_the_full_window() {
    let rig = graph_rig().await;
    rig.http.push(
        200,
        &json!({"value": [graph_event("e1", "Old", "2026-01-13T09:00:00", "2026-01-13T10:00:00")],
                "@odata.deltaLink": delta_link("d1")}),
    );
    rig.service.sync_all().await;
    let before = rig.http.requests().len();
    rig.http.push(
        410,
        &json!({"error": {"code": "syncStateNotFound", "message": "gone"}}),
    );
    rig.http.push(
        200,
        &json!({"value": [graph_event("e2", "New", "2026-01-14T09:00:00", "2026-01-14T10:00:00")],
                "@odata.deltaLink": delta_link("d9")}),
    );
    let report = rig.service.sync_all().await;
    assert!(
        report.outcomes.iter().all(|o| o.error.is_none()),
        "{report:?}"
    );
    assert_eq!(rig.http.url(before), delta_link("d1"));
    assert!(rig.http.url(before + 1).contains("startDateTime="));
    assert_eq!(
        summaries(&rig, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z"),
        ["New 2026-01-14T09:00:00Z"]
    );
}

#[tokio::test]
async fn graph_links_to_other_hosts_are_not_followed() {
    let rig = graph_rig().await;
    rig.http.push(
        200,
        &json!({"value": [], "@odata.nextLink": "https://evil.example/steal?x=1"}),
    );
    let report = rig.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, Some(CalendarError::Server));
    assert_eq!(rig.http.requests().len(), 2);
    rig.http.push(
        200,
        &json!({"value": [], "@odata.nextLink": "http://graph.microsoft.com/v1.0/x"}),
    );
    let report = rig.service.sync_all().await;
    assert_eq!(report.outcomes[0].error, Some(CalendarError::Server));
}

#[tokio::test]
async fn graph_writes_create_update_and_delete_events() {
    let rig = graph_rig().await;
    rig.http.push(
        200,
        &json!({"value": [], "@odata.deltaLink": delta_link("d1")}),
    );
    rig.service.sync_all().await;
    let calendar = rig.service.calendars()[0].id.clone();

    rig.http.push(
        201,
        &graph_event(
            "new1",
            "Planning",
            "2026-01-13T09:00:00",
            "2026-01-13T10:00:00",
        ),
    );
    let created = rig
        .service
        .create_event(&calendar, &draft("Planning", None))
        .await
        .expect("create");
    assert_eq!(created.uid, "new1");
    let at = rig.http.requests().len() - 1;
    assert_eq!(rig.http.requests()[at].method, Method::Post);
    assert_eq!(rig.http.url(at), format!("{GRAPH_CAL}/events"));
    let body = rig.http.body(at);
    assert_eq!(body["subject"], "Planning");
    assert_eq!(body["start"]["timeZone"], "Europe/Berlin");
    assert_eq!(body["isReminderOn"], true);
    assert_eq!(body["reminderMinutesBeforeStart"], 10);
    assert_eq!(body["recurrence"], Value::Null);
    assert_eq!(
        summaries(&rig, "2026-01-13T00:00:00Z", "2026-01-14T00:00:00Z").len(),
        1
    );

    rig.http.push(
        200,
        &graph_event(
            "new1",
            "Planning 2",
            "2026-01-13T09:00:00",
            "2026-01-13T10:00:00",
        ),
    );
    rig.service
        .update_event(&calendar, "new1", &draft("Planning 2", None))
        .await
        .expect("update");
    let at = rig.http.requests().len() - 1;
    assert_eq!(rig.http.requests()[at].method, Method::Patch);
    assert_eq!(
        rig.http.url(at),
        "https://graph.microsoft.com/v1.0/me/events/new1"
    );
    assert!(rig.http.body(at).get("recurrence").is_none());
    assert_eq!(
        summaries(&rig, "2026-01-13T00:00:00Z", "2026-01-14T00:00:00Z"),
        ["Planning 2 2026-01-13T09:00:00Z"]
    );

    // A new series is sent as a Graph pattern and read back by the next
    // delta (the master itself is never cached).
    let mut master = graph_event(
        "series1",
        "Weekly",
        "2026-01-13T09:00:00",
        "2026-01-13T10:00:00",
    );
    master["type"] = json!("seriesMaster");
    master["recurrence"] = json!({
        "pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["tuesday"]},
        "range": {"type": "noEnd", "startDate": "2026-01-13"}});
    rig.http.push(201, &master);
    rig.http.push(
        200,
        &json!({"value": [
            graph_event("i1", "Weekly", "2026-01-20T09:00:00", "2026-01-20T10:00:00")],
            "@odata.deltaLink": delta_link("d2")}),
    );
    let series = rig
        .service
        .create_event(&calendar, &draft("Weekly", Some("FREQ=WEEKLY;BYDAY=TU")))
        .await
        .expect("series");
    assert_eq!(series.uid, "series1");
    let at = rig.http.requests().len() - 2;
    assert_eq!(rig.http.body(at)["recurrence"]["pattern"]["type"], "weekly");
    assert_eq!(
        rig.http.body(at)["recurrence"]["pattern"]["daysOfWeek"],
        json!(["tuesday"])
    );
    let listed = summaries(&rig, "2026-01-19T00:00:00Z", "2026-01-21T00:00:00Z");
    assert_eq!(listed, ["Weekly 2026-01-20T09:00:00Z"]);

    rig.http.push(204, &Value::Null);
    rig.service
        .delete_event(&calendar, "new1")
        .await
        .expect("delete");
    let at = rig.http.requests().len() - 1;
    assert_eq!(rig.http.requests()[at].method, Method::Delete);
    assert!(summaries(&rig, "2026-01-13T00:00:00Z", "2026-01-14T00:00:00Z").is_empty());

    // A repeat is a set of single events on Microsoft: no occurrence API.
    let result = rig
        .service
        .delete_occurrence(&calendar, "i1", "2026-01-20T09:00:00Z")
        .await;
    assert_eq!(result.unwrap_err(), CalendarError::Unsupported);
}

// -- accounts and persistence -------------------------------------------------------

#[tokio::test]
async fn removing_an_oauth_account_signs_it_out() {
    let rig = google_rig().await;
    let id = rig.service.accounts()[0].id.clone();
    rig.service
        .remove_account_and_session(&id)
        .await
        .expect("removed");
    assert!(rig.service.accounts().is_empty());
    assert_eq!(rig.tokens.signed_out.lock().unwrap().as_slice(), [id]);
}

#[tokio::test]
async fn oauth_accounts_without_a_token_port_are_unsupported() {
    let service = CalendarService::new(
        Arc::new(Script::default()),
        Arc::new(MemoryStorage::default()),
        Arc::new(MemorySecrets::default()),
    )
    .expect("service");
    let result = service
        .add_oauth_account(&grant(Provider::Google, vec![Purpose::Calendar]), None)
        .await;
    assert_eq!(result.unwrap_err(), CalendarError::Unsupported);
}

#[test]
fn oauth_accounts_survive_the_database_round_trip() {
    let dir = tempfile::tempdir().expect("dir");
    let storage = StoreCalendar::new(
        Arc::new(zephium_store::SqliteStore::open(dir.path()).expect("store")),
        None,
    );
    let account =
        |id: &str, kind, tenant: Option<&str>, username: Option<&str>| CalendarAccountV1 {
            id: id.to_owned(),
            kind,
            name: "n".to_owned(),
            server_url: "https://example.org/".to_owned(),
            username: username.map(str::to_owned),
            oauth: matches!(
                kind,
                CalendarAccountKindV1::Google | CalendarAccountKindV1::Microsoft
            )
            .then(|| CalendarOAuthV1 {
                client_id: "client-1".to_owned(),
                tenant: tenant.map(str::to_owned),
            }),
        };
    let state = StoredState {
        version: 1,
        accounts: vec![
            account(
                "A1",
                CalendarAccountKindV1::Google,
                None,
                Some("ann@example.com"),
            ),
            account("A2", CalendarAccountKindV1::Microsoft, Some("common"), None),
            account("A3", CalendarAccountKindV1::Caldav, None, Some("ann")),
        ],
        calendars: Vec::new(),
    };
    storage.save(&state).expect("save");
    assert_eq!(CalendarStorage::load(&storage).expect("load"), state);
}
