//! Google Calendar API v3.
//!
//! Chosen over Google's CalDAV endpoint: the REST API has first-class
//! incremental sync (`syncToken`, with `410` / `fullSyncRequired` when it
//! expires), plain JSON instead of XML, ETag-guarded writes and stable
//! instance ids for single-occurrence changes, and it needs the same OAuth
//! scope and the same enabled API as CalDAV. Recurrence is Google's own
//! RRULE strings, which map straight onto the iCalendar model.

use super::{
    clip, encode_segment, hex_color, id_ok, parse_json, Api, Delta, Item, RemoteCalendar,
    MAX_CALENDARS_PER_ACCOUNT, MAX_DESCRIPTION_BYTES, MAX_LOCATION_BYTES, MAX_PAGES,
    MAX_SUMMARY_CHARS, MAX_TOKEN_BYTES, PAGE_SIZE,
};
use crate::error::{CalendarError, Result};
use crate::http::{check_status, HttpRequest, HttpResponse, Method};
use crate::ical;
use crate::model::{Alarm, Event, EventStatus, EventTime, PartStat, Person, TimeKind};
use crate::recur;
use crate::tz;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

/// Root of the API; every address this module builds starts here.
pub const API_ROOT: &str = "https://www.googleapis.com/calendar/v3";
const MAX_REMINDERS: usize = 5;
const MAX_RECURRENCE_LINES: usize = 600;

#[derive(Debug, Default, Deserialize)]
struct GTime {
    date: Option<String>,
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GPerson {
    email: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "responseStatus")]
    response_status: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GReminder {
    method: Option<String>,
    minutes: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct GReminders {
    #[serde(default)]
    overrides: Vec<GReminder>,
}

#[derive(Debug, Default, Deserialize)]
struct GEvent {
    id: String,
    status: Option<String>,
    etag: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start: Option<GTime>,
    end: Option<GTime>,
    #[serde(default)]
    recurrence: Vec<String>,
    #[serde(rename = "recurringEventId")]
    recurring_event_id: Option<String>,
    #[serde(rename = "originalStartTime")]
    original_start_time: Option<GTime>,
    #[serde(rename = "hangoutLink")]
    hangout_link: Option<String>,
    organizer: Option<GPerson>,
    #[serde(default)]
    attendees: Vec<GPerson>,
    reminders: Option<GReminders>,
    sequence: Option<u32>,
}

// -- reading ------------------------------------------------------------------

fn time(value: &GTime) -> Option<EventTime> {
    if let Some(date) = &value.date {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
        return Some(EventTime::date(date));
    }
    let instant = DateTime::parse_from_rfc3339(value.date_time.as_deref()?).ok()?;
    match value.time_zone.as_deref().and_then(tz::resolve) {
        Some(zone) => Some(EventTime {
            kind: TimeKind::Zoned(zone),
            local: instant.with_timezone(&zone).naive_local(),
        }),
        None => Some(EventTime::utc(instant.with_timezone(&Utc))),
    }
}

/// Values of an `EXDATE` / `RDATE` line, read like the start they belong to.
fn dates_of_line(line: &str, start: &EventTime) -> Vec<EventTime> {
    let Some((head, values)) = line.split_once(':') else {
        return Vec::new();
    };
    let mut zone = match &start.kind {
        TimeKind::Zoned(zone) => Some(*zone),
        _ => None,
    };
    let mut date_only = false;
    for parameter in head.split(';').skip(1) {
        let Some((key, value)) = parameter.split_once('=') else {
            continue;
        };
        match key.to_ascii_uppercase().as_str() {
            "TZID" => zone = tz::resolve(value),
            "VALUE" => date_only = value.eq_ignore_ascii_case("DATE"),
            _ => {}
        }
    }
    values
        .split(',')
        .take(ical::MAX_LIST)
        .filter_map(|value| {
            let value = value.trim();
            if date_only || value.len() == 8 {
                let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
                return Some(EventTime::date(date));
            }
            if let Some(stripped) = value.strip_suffix(['Z', 'z']) {
                let local = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
                return Some(match zone {
                    Some(zone) => EventTime {
                        kind: TimeKind::Zoned(zone),
                        local: local.and_utc().with_timezone(&zone).naive_local(),
                    },
                    None => EventTime {
                        kind: TimeKind::Utc,
                        local,
                    },
                });
            }
            let local = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
            Some(EventTime {
                kind: zone.map_or(TimeKind::Floating, TimeKind::Zoned),
                local,
            })
        })
        .collect()
}

fn person(value: &GPerson) -> Option<Person> {
    let email = value.email.as_deref()?.trim();
    if email.is_empty() || email.len() > 320 || email.chars().any(char::is_control) {
        return None;
    }
    Some(Person {
        email: email.to_owned(),
        name: value
            .display_name
            .as_deref()
            .map(|name| clip(name.trim(), 200))
            .filter(|name| !name.is_empty()),
        partstat: match value.response_status.as_deref() {
            Some("accepted") => PartStat::Accepted,
            Some("declined") => PartStat::Declined,
            Some("tentative") => PartStat::Tentative,
            _ => PartStat::NeedsAction,
        },
    })
}

fn convert(g: &GEvent, uid: &str, recurrence_id: Option<EventTime>) -> Option<Event> {
    let start = time(g.start.as_ref()?)?;
    let end = g
        .end
        .as_ref()
        .and_then(time)
        .filter(|end| end.to_utc() >= start.to_utc())
        .unwrap_or_else(|| start.clone());
    let mut event = Event {
        uid: uid.to_owned(),
        summary: clip(g.summary.as_deref().unwrap_or("").trim(), MAX_SUMMARY_CHARS),
        description: clip(
            g.description.as_deref().unwrap_or(""),
            MAX_DESCRIPTION_BYTES,
        ),
        location: clip(
            g.location.as_deref().unwrap_or("").trim(),
            MAX_LOCATION_BYTES,
        ),
        start,
        end,
        status: match g.status.as_deref() {
            Some("confirmed") => EventStatus::Confirmed,
            Some("tentative") => EventStatus::Tentative,
            _ => EventStatus::None,
        },
        organizer: g.organizer.as_ref().and_then(person),
        attendees: g
            .attendees
            .iter()
            .filter_map(person)
            .take(ical::MAX_ATTENDEES)
            .collect(),
        alarms: g
            .reminders
            .iter()
            .flat_map(|reminders| reminders.overrides.iter())
            .filter_map(|reminder| {
                let minutes = reminder.minutes.filter(|m| (0..=40_320).contains(m))?;
                Some(Alarm {
                    minutes_before: minutes,
                    from_end: false,
                    action: if reminder.method.as_deref() == Some("email") {
                        "EMAIL"
                    } else {
                        "DISPLAY"
                    }
                    .to_owned(),
                })
            })
            .take(MAX_REMINDERS)
            .collect(),
        url: g
            .hangout_link
            .as_deref()
            .filter(|link| link.starts_with("https://") && link.len() <= 2_000)
            .map(str::to_owned),
        sequence: g.sequence.unwrap_or(0),
        rrule: None,
        rdates: Vec::new(),
        exdates: Vec::new(),
        recurrence_id,
        overrides: Vec::new(),
    };
    if event.recurrence_id.is_none() {
        for line in g.recurrence.iter().take(MAX_RECURRENCE_LINES) {
            let upper = line.to_ascii_uppercase();
            if let Some(rule) = upper.strip_prefix("RRULE:") {
                let rule = line[line.len() - rule.len()..].trim();
                if event.rrule.is_none() && recur::validate_rule(rule, &event.start).is_ok() {
                    event.rrule = Some(rule.to_owned());
                }
            } else if upper.starts_with("EXDATE") {
                let room = ical::MAX_LIST.saturating_sub(event.exdates.len());
                event
                    .exdates
                    .extend(dates_of_line(line, &event.start).into_iter().take(room));
            } else if upper.starts_with("RDATE") {
                let room = ical::MAX_LIST.saturating_sub(event.rdates.len());
                event
                    .rdates
                    .extend(dates_of_line(line, &event.start).into_iter().take(room));
            }
        }
    }
    Some(event)
}

fn item_of(g: &GEvent) -> Option<Item> {
    if !id_ok(&g.id) {
        return None;
    }
    let cancelled = g.status.as_deref() == Some("cancelled");
    if let Some(series) = g.recurring_event_id.as_deref().filter(|id| id_ok(id)) {
        let original = g.original_start_time.as_ref().and_then(time)?;
        let event = if cancelled {
            None
        } else {
            Some(convert(g, series, Some(original.clone()))?)
        };
        return Some(Item::Occurrence {
            series: series.to_owned(),
            original,
            event,
        });
    }
    if cancelled {
        return Some(Item::Gone { href: g.id.clone() });
    }
    Some(Item::Event {
        href: g.id.clone(),
        etag: g.etag.clone().filter(|etag| etag.len() <= 1_024),
        event: convert(g, &g.id, None)?,
    })
}

/// Turns one event resource into an item; `None` for data that cannot be
/// understood (it is skipped, not fatal).
pub fn parse_event(value: &Value) -> Option<Item> {
    item_of(&serde_json::from_value::<GEvent>(value.clone()).ok()?)
}

#[derive(Default, Deserialize)]
struct EventsPage {
    #[serde(default)]
    items: Vec<Value>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "nextSyncToken")]
    next_sync_token: Option<String>,
}

/// Parses one `events.list` page into items, skipping what is unreadable.
fn page_items(page: &EventsPage) -> Vec<Item> {
    page.items.iter().filter_map(parse_event).collect()
}

// -- calendars -----------------------------------------------------------------

#[derive(Default, Deserialize)]
struct CalendarListPage {
    #[serde(default)]
    items: Vec<Value>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct CalendarEntry {
    id: String,
    summary: Option<String>,
    #[serde(rename = "summaryOverride")]
    summary_override: Option<String>,
    #[serde(rename = "backgroundColor")]
    background_color: Option<String>,
    #[serde(rename = "accessRole")]
    access_role: Option<String>,
    primary: Option<bool>,
    deleted: Option<bool>,
}

/// The URL of a calendar's collection.
pub fn calendar_url(id: &str) -> String {
    format!("{API_ROOT}/calendars/{}", encode_segment(id))
}

fn calendar_events_url(calendar_url: &str) -> Result<Url> {
    if !calendar_url.starts_with(&format!("{API_ROOT}/calendars/")) {
        return Err(CalendarError::InvalidInput);
    }
    Url::parse(&format!("{calendar_url}/events")).map_err(|_| CalendarError::InvalidInput)
}

/// Lists the calendars of the signed-in account.
pub async fn calendars(api: &Api<'_>) -> Result<Vec<RemoteCalendar>> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut url = Url::parse(&format!("{API_ROOT}/users/me/calendarList"))
            .map_err(|_| CalendarError::Server)?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("maxResults", "250");
            query.append_pair("minAccessRole", "reader");
            if let Some(token) = &page_token {
                query.append_pair("pageToken", token);
            }
        }
        let response = api
            .call(HttpRequest::new(Method::Get, url).header("accept", "application/json"))
            .await?;
        check_status(response.status)?;
        let page: CalendarListPage = parse_json(&response)?;
        for value in &page.items {
            let Ok(entry) = serde_json::from_value::<CalendarEntry>(value.clone()) else {
                continue;
            };
            if entry.deleted == Some(true) || !id_ok(&entry.id) {
                continue;
            }
            let role = entry.access_role.as_deref().unwrap_or("reader");
            if role == "freeBusyReader" {
                continue;
            }
            if out.len() >= MAX_CALENDARS_PER_ACCOUNT {
                return Err(CalendarError::Limit);
            }
            let name = entry
                .summary_override
                .as_deref()
                .or(entry.summary.as_deref())
                .map(|name| clip(name.trim(), 200))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Calendar".to_owned());
            out.push(RemoteCalendar {
                url: calendar_url(&entry.id),
                name,
                color: entry.background_color.as_deref().and_then(hex_color),
                read_only: !matches!(role, "owner" | "writer"),
                owner: (entry.primary == Some(true)).then(|| entry.id.clone()),
            });
        }
        match page.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => return Ok(out),
        }
    }
    Err(CalendarError::Limit)
}

// -- sync ------------------------------------------------------------------------

struct Expired;

async fn pages(
    api: &Api<'_>,
    calendar_url: &str,
    token: Option<&str>,
) -> Result<std::result::Result<Delta, Expired>> {
    let base = calendar_events_url(calendar_url)?;
    let mut delta = Delta {
        full: token.is_none(),
        ..Delta::default()
    };
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut url = base.clone();
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("maxResults", &PAGE_SIZE.to_string());
            query.append_pair("singleEvents", "false");
            match token {
                Some(token) => {
                    query.append_pair("syncToken", token);
                }
                None => {
                    query.append_pair("showDeleted", "false");
                }
            }
            if let Some(next) = &page_token {
                query.append_pair("pageToken", next);
            }
        }
        let response = api
            .call(HttpRequest::new(Method::Get, url).header("accept", "application/json"))
            .await?;
        if response.status == 410 && token.is_some() {
            return Ok(Err(Expired));
        }
        check_status(response.status)?;
        let page: EventsPage = parse_json(&response)?;
        delta.items.extend(page_items(&page));
        if delta.items.len() > ical::MAX_EVENTS {
            return Err(CalendarError::Limit);
        }
        match (page.next_page_token, page.next_sync_token) {
            (Some(next), _) if !next.is_empty() => page_token = Some(next),
            (_, Some(sync)) if sync.len() <= MAX_TOKEN_BYTES => {
                delta.next_token = Some(sync);
                return Ok(Ok(delta));
            }
            _ => return Ok(Ok(delta)),
        }
    }
    Err(CalendarError::Limit)
}

/// Reads the changes since `token`, or the whole calendar when there is no
/// token. When the provider says the token expired (`410`, reason
/// `fullSyncRequired`) the whole calendar is read again and the result is
/// marked [`Delta::full`].
pub async fn sync(api: &Api<'_>, calendar_url: &str, token: Option<&str>) -> Result<Delta> {
    match pages(api, calendar_url, token).await? {
        Ok(delta) => Ok(delta),
        Err(Expired) => match pages(api, calendar_url, None).await? {
            Ok(delta) => Ok(delta),
            Err(Expired) => Err(CalendarError::Server),
        },
    }
}

// -- writing ---------------------------------------------------------------------

fn time_json(time: &EventTime) -> Value {
    match &time.kind {
        TimeKind::Date => json!({
            "date": time.local.format("%Y-%m-%d").to_string(),
            "dateTime": null,
            "timeZone": null,
        }),
        TimeKind::Zoned(zone) => json!({
            "date": null,
            "dateTime": time.local.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "timeZone": zone.name(),
        }),
        // A floating time has no zone to send: it goes out as UTC.
        TimeKind::Utc | TimeKind::Floating => json!({
            "date": null,
            "dateTime": format!("{}Z", time.local.format("%Y-%m-%dT%H:%M:%S")),
            "timeZone": "UTC",
        }),
    }
}

fn date_line(name: &str, time: &EventTime) -> String {
    match &time.kind {
        TimeKind::Date => format!("{name};VALUE=DATE:{}", time.local.format("%Y%m%d")),
        TimeKind::Zoned(zone) => format!(
            "{name};TZID={}:{}",
            zone.name(),
            time.local.format("%Y%m%dT%H%M%S")
        ),
        TimeKind::Utc => format!("{name}:{}Z", time.local.format("%Y%m%dT%H%M%S")),
        TimeKind::Floating => format!("{name}:{}", time.local.format("%Y%m%dT%H%M%S")),
    }
}

/// The `recurrence` array of a series, or `None` for a single event.
pub fn recurrence_lines(event: &Event) -> Option<Vec<String>> {
    if !event.is_recurring() {
        return None;
    }
    let mut lines = Vec::new();
    if let Some(rule) = &event.rrule {
        lines.push(format!("RRULE:{rule}"));
    }
    lines.extend(event.rdates.iter().map(|date| date_line("RDATE", date)));
    lines.extend(event.exdates.iter().map(|date| date_line("EXDATE", date)));
    lines.truncate(MAX_RECURRENCE_LINES);
    Some(lines)
}

/// The JSON body that writes `event`. A series master carries its
/// `recurrence` (or `null` when it no longer repeats); an occurrence does not.
pub fn event_body(event: &Event, series: bool) -> Value {
    let overrides: Vec<Value> = event
        .alarms
        .iter()
        .filter(|alarm| !alarm.from_end && alarm.minutes_before >= 0)
        .take(MAX_REMINDERS)
        .map(|alarm| json!({"method": "popup", "minutes": alarm.minutes_before}))
        .collect();
    let mut body = json!({
        "summary": event.summary,
        "description": event.description,
        "location": event.location,
        "start": time_json(&event.start),
        "end": time_json(&event.end),
        "reminders": {"useDefault": false, "overrides": overrides},
    });
    if series {
        body["recurrence"] = recurrence_lines(event).map_or(Value::Null, |lines| json!(lines));
    }
    body
}

fn json_request(method: Method, url: Url, body: &Value, etag: Option<&str>) -> Result<HttpRequest> {
    let bytes = serde_json::to_vec(body).map_err(|_| CalendarError::InvalidInput)?;
    let mut request = HttpRequest::new(method, url)
        .header("accept", "application/json")
        .body("application/json", bytes);
    if let Some(etag) = etag.filter(|etag| !etag.contains(['\r', '\n'])) {
        request = request.header("if-match", etag);
    }
    Ok(request)
}

fn single_item(response: &HttpResponse) -> Result<Vec<Item>> {
    check_status(response.status)?;
    let value: Value = parse_json(response)?;
    parse_event(&value)
        .map(|item| vec![item])
        .ok_or(CalendarError::Parse)
}

/// Creates a (possibly recurring) event. Returns the item to cache.
pub async fn create(api: &Api<'_>, calendar_url: &str, event: &Event) -> Result<Vec<Item>> {
    let url = calendar_events_url(calendar_url)?;
    let request = json_request(Method::Post, url, &event_body(event, true), None)?;
    single_item(&api.call(request).await?)
}

fn event_url(calendar_url: &str, id: &str) -> Result<Url> {
    if !id_ok(id) || !calendar_url.starts_with(&format!("{API_ROOT}/calendars/")) {
        return Err(CalendarError::InvalidInput);
    }
    Url::parse(&format!("{calendar_url}/events/{}", encode_segment(id)))
        .map_err(|_| CalendarError::InvalidInput)
}

/// Replaces the editable fields (and the repeat rule) of a series master or
/// single event, guarded by its ETag (`412` is [`CalendarError::Conflict`]).
pub async fn update(
    api: &Api<'_>,
    calendar_url: &str,
    event: &Event,
    etag: Option<&str>,
) -> Result<Vec<Item>> {
    let url = event_url(calendar_url, &event.uid)?;
    let request = json_request(Method::Patch, url, &event_body(event, true), etag)?;
    single_item(&api.call(request).await?)
}

/// The id Google gives the instance of a series that starts at `instance`.
pub fn instance_id(series: &str, all_day: bool, instance: &EventTime) -> String {
    if all_day {
        format!("{series}_{}", instance.local.format("%Y%m%d"))
    } else {
        format!("{series}_{}", instance.to_utc().format("%Y%m%dT%H%M%SZ"))
    }
}

/// Changes one occurrence of a series (Google writes an exception event).
pub async fn update_occurrence(
    api: &Api<'_>,
    calendar_url: &str,
    series: &str,
    all_day: bool,
    instance: &EventTime,
    item: &Event,
) -> Result<Vec<Item>> {
    let url = event_url(calendar_url, &instance_id(series, all_day, instance))?;
    let request = json_request(Method::Patch, url, &event_body(item, false), None)?;
    single_item(&api.call(request).await?)
}

async fn delete_at(api: &Api<'_>, url: Url, etag: Option<&str>) -> Result<()> {
    let mut request = HttpRequest::new(Method::Delete, url);
    if let Some(etag) = etag.filter(|etag| !etag.contains(['\r', '\n'])) {
        request = request.header("if-match", etag);
    }
    let response = api.call(request).await?;
    // Already gone is the state the person asked for.
    match response.status {
        404 | 410 => Ok(()),
        _ => check_status(response.status),
    }
}

/// Deletes an event (the whole series for a master).
pub async fn delete(api: &Api<'_>, calendar_url: &str, id: &str, etag: Option<&str>) -> Result<()> {
    delete_at(api, event_url(calendar_url, id)?, etag).await
}

/// Deletes one occurrence of a series (Google keeps it as a cancelled
/// exception).
pub async fn delete_occurrence(
    api: &Api<'_>,
    calendar_url: &str,
    series: &str,
    all_day: bool,
    instance: &EventTime,
) -> Result<()> {
    let url = event_url(calendar_url, &instance_id(series, all_day, instance))?;
    delete_at(api, url, None).await
}
