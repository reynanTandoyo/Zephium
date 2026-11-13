//! Microsoft Graph calendars (`/me/calendars`, `/me/calendarView/delta`,
//! `/me/events`).
//!
//! Reading uses the **expanded** view: `calendarView/delta` returns every
//! occurrence in a window as its own event, exceptions and cancelled
//! occurrences already applied by Exchange, so recurrence needs no local
//! interpretation and a removed occurrence is simply a removed event. Each
//! instance is cached as a plain event keyed by its Graph id; editing or
//! deleting one therefore changes that occurrence only. Series masters are
//! understood when they appear (a create response, for example): their
//! `recurrence` pattern maps to an RRULE and back, and cancelled occurrences
//! become EXDATEs. The window is re-read from scratch about every ninety days
//! (a delta link keeps its original window).

use super::{
    clip, encode_segment, hex_color, id_ok, parse_json, Api, Delta, Item, RemoteCalendar,
    MAX_CALENDARS_PER_ACCOUNT, MAX_DESCRIPTION_BYTES, MAX_LOCATION_BYTES, MAX_PAGES,
    MAX_SUMMARY_CHARS, MAX_TOKEN_BYTES,
};
use crate::api::CalendarRecurrenceV1;
use crate::error::{CalendarError, Result};
use crate::http::{check_status, HttpRequest, HttpResponse, Method};
use crate::ical;
use crate::model::{Alarm, Event, EventStatus, EventTime, PartStat, Person, TimeKind};
use crate::recurrence;
use crate::tz;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

/// Root of the API; every address this module builds starts here.
pub const API_ROOT: &str = "https://graph.microsoft.com/v1.0";
const HOST: &str = "graph.microsoft.com";
/// How far back and ahead the expanded window reaches.
const WINDOW_PAST_DAYS: i64 = 180;
const WINDOW_FUTURE_DAYS: i64 = 365;
/// A saved delta link whose window ends sooner than this is dropped.
const RESYNC_BEFORE_END_DAYS: i64 = 270;
const TOKEN_PREFIX: &str = "g1";
const PREFER: &str =
    "odata.maxpagesize=100, outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"";

#[derive(Debug, Default, Deserialize)]
struct GDateTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    #[serde(rename = "timeZone")]
    time_zone: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GAddress {
    name: Option<String>,
    address: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GRecipient {
    #[serde(rename = "emailAddress")]
    email: Option<GAddress>,
}

#[derive(Debug, Default, Deserialize)]
struct GResponse {
    response: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GAttendee {
    #[serde(rename = "emailAddress")]
    email: Option<GAddress>,
    status: Option<GResponse>,
}

#[derive(Debug, Default, Deserialize)]
struct GBody {
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GLocation {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GMeeting {
    #[serde(rename = "joinUrl")]
    join_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GPattern {
    #[serde(rename = "type")]
    kind: Option<String>,
    interval: Option<u32>,
    #[serde(rename = "daysOfWeek", default)]
    days_of_week: Vec<String>,
    #[serde(rename = "dayOfMonth")]
    day_of_month: Option<i32>,
    month: Option<u32>,
    index: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GRange {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "endDate")]
    end_date: Option<String>,
    #[serde(rename = "numberOfOccurrences")]
    count: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct GRecurrence {
    pattern: Option<GPattern>,
    range: Option<GRange>,
}

#[derive(Debug, Default, Deserialize)]
struct GEvent {
    id: String,
    #[serde(rename = "@odata.etag")]
    etag: Option<String>,
    #[serde(rename = "@removed")]
    removed: Option<Value>,
    subject: Option<String>,
    body: Option<GBody>,
    start: Option<GDateTime>,
    end: Option<GDateTime>,
    #[serde(rename = "isAllDay")]
    is_all_day: Option<bool>,
    #[serde(rename = "isCancelled")]
    is_cancelled: Option<bool>,
    location: Option<GLocation>,
    #[serde(rename = "type")]
    kind: Option<String>,
    recurrence: Option<GRecurrence>,
    #[serde(rename = "cancelledOccurrences", default)]
    cancelled_occurrences: Vec<String>,
    organizer: Option<GRecipient>,
    #[serde(default)]
    attendees: Vec<GAttendee>,
    #[serde(rename = "isReminderOn")]
    is_reminder_on: Option<bool>,
    #[serde(rename = "reminderMinutesBeforeStart")]
    reminder_minutes: Option<i64>,
    #[serde(rename = "onlineMeeting")]
    online_meeting: Option<GMeeting>,
    #[serde(rename = "showAs")]
    show_as: Option<String>,
}

// -- reading ------------------------------------------------------------------

fn time(value: &GDateTime, all_day: bool) -> Option<EventTime> {
    let text = value.date_time.as_deref()?;
    let local = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
    if all_day {
        return Some(EventTime::date(local.date()));
    }
    let zone = value.time_zone.as_deref().unwrap_or("UTC").trim();
    if zone.eq_ignore_ascii_case("UTC") {
        return Some(EventTime {
            kind: TimeKind::Utc,
            local,
        });
    }
    Some(EventTime {
        kind: TimeKind::Zoned(tz::resolve(zone)?),
        local,
    })
}

fn person(address: &GAddress) -> Option<(String, Option<String>)> {
    let email = address.address.as_deref()?.trim();
    if email.is_empty() || email.len() > 320 || email.chars().any(char::is_control) {
        return None;
    }
    let name = address
        .name
        .as_deref()
        .map(|name| clip(name.trim(), 200))
        .filter(|name| !name.is_empty());
    Some((email.to_owned(), name))
}

const DAYS: [(&str, &str); 7] = [
    ("monday", "MO"),
    ("tuesday", "TU"),
    ("wednesday", "WE"),
    ("thursday", "TH"),
    ("friday", "FR"),
    ("saturday", "SA"),
    ("sunday", "SU"),
];
const INDEXES: [(&str, i32); 5] = [
    ("first", 1),
    ("second", 2),
    ("third", 3),
    ("fourth", 4),
    ("last", -1),
];

fn day_code(name: &str) -> Option<&'static str> {
    DAYS.iter()
        .find(|(long, _)| long.eq_ignore_ascii_case(name))
        .map(|(_, code)| *code)
}

fn day_name(code: &str) -> Option<&'static str> {
    DAYS.iter()
        .find(|(_, short)| short.eq_ignore_ascii_case(code))
        .map(|(long, _)| *long)
}

/// A Graph `recurrence` as an RRULE value for a series starting at `start`;
/// `None` for a pattern this model cannot hold.
fn rrule_from(recurrence: &GRecurrence, start: &EventTime) -> Option<String> {
    let pattern = recurrence.pattern.as_ref()?;
    let range = recurrence.range.as_ref()?;
    let kind = pattern.kind.as_deref()?;
    let indexed_day = |day: &str| -> Option<String> {
        let index = INDEXES
            .iter()
            .find(|(name, _)| {
                pattern
                    .index
                    .as_deref()
                    .is_some_and(|i| i.eq_ignore_ascii_case(name))
            })?
            .1;
        Some(format!("{index}{}", day_code(day)?))
    };
    let mut rule = CalendarRecurrenceV1 {
        freq: String::new(),
        interval: pattern.interval.unwrap_or(1),
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        custom: false,
    };
    match kind {
        "daily" => "daily".clone_into(&mut rule.freq),
        "weekly" => {
            "weekly".clone_into(&mut rule.freq);
            rule.by_day = pattern
                .days_of_week
                .iter()
                .map(|day| day_code(day).map(str::to_owned))
                .collect::<Option<_>>()?;
        }
        "absoluteMonthly" => {
            "monthly".clone_into(&mut rule.freq);
            rule.by_month_day = vec![pattern.day_of_month?];
        }
        "relativeMonthly" => {
            "monthly".clone_into(&mut rule.freq);
            let [day] = pattern.days_of_week.as_slice() else {
                return None;
            };
            rule.by_day = vec![indexed_day(day)?];
        }
        "absoluteYearly" => {
            "yearly".clone_into(&mut rule.freq);
            rule.by_month = vec![pattern.month?];
            rule.by_month_day = vec![pattern.day_of_month?];
        }
        "relativeYearly" => {
            "yearly".clone_into(&mut rule.freq);
            rule.by_month = vec![pattern.month?];
            let [day] = pattern.days_of_week.as_slice() else {
                return None;
            };
            rule.by_day = vec![indexed_day(day)?];
        }
        _ => return None,
    }
    match range.kind.as_deref() {
        Some("numbered") => rule.count = Some(range.count?),
        Some("endDate") => {
            let end = range.end_date.as_deref()?;
            NaiveDate::parse_from_str(end, "%Y-%m-%d").ok()?;
            rule.until = Some(end.to_owned());
        }
        _ => {}
    }
    recurrence::build(&rule, start).ok()
}

/// The Graph `recurrence` for an RRULE value of a series starting at `start`.
/// Rules Graph cannot express (several month days, BYSETPOS, ...) are
/// [`CalendarError::Unsupported`].
pub fn recurrence_json(rule: &str, start: &EventTime) -> Result<Value> {
    let described = recurrence::describe(rule, start).ok_or(CalendarError::Unsupported)?;
    if described.custom {
        return Err(CalendarError::Unsupported);
    }
    let interval = described.interval;
    let own_day = |date: NaiveDate| {
        DAYS[usize::try_from(date.weekday().num_days_from_monday()).unwrap_or(0)].0
    };
    let indexed = |code: &str| -> Result<(String, &'static str)> {
        let digits = code.trim_start_matches(['+', '-']);
        let (number, day) = digits.split_at(digits.len().saturating_sub(2));
        let negative = code.starts_with('-');
        let index = if negative {
            "last"
        } else {
            INDEXES
                .iter()
                .find(|(_, value)| number.parse::<i32>().ok() == Some(*value))
                .map(|(name, _)| *name)
                .ok_or(CalendarError::Unsupported)?
        };
        Ok((
            index.to_owned(),
            day_name(day).ok_or(CalendarError::Unsupported)?,
        ))
    };
    let date = start.local.date();
    let month = described.by_month.first().copied().unwrap_or(date.month());
    if described.by_month.len() > 1 || described.by_month_day.len() > 1 {
        return Err(CalendarError::Unsupported);
    }
    let day_of_month = match described.by_month_day.first() {
        Some(day) if *day > 0 => *day,
        Some(_) => return Err(CalendarError::Unsupported),
        None => i32::try_from(date.day()).unwrap_or(1),
    };
    let pattern = match described.freq.as_str() {
        "daily" => json!({"type": "daily", "interval": interval}),
        "weekly" => {
            let days: Vec<&str> = if described.by_day.is_empty() {
                vec![own_day(date)]
            } else {
                described
                    .by_day
                    .iter()
                    .map(|code| {
                        if code.len() == 2 {
                            day_name(code).ok_or(CalendarError::Unsupported)
                        } else {
                            Err(CalendarError::Unsupported)
                        }
                    })
                    .collect::<Result<_>>()?
            };
            json!({"type": "weekly", "interval": interval, "daysOfWeek": days,
                   "firstDayOfWeek": "monday"})
        }
        "monthly" => match described.by_day.as_slice() {
            [] => json!({"type": "absoluteMonthly", "interval": interval,
                         "dayOfMonth": day_of_month}),
            [code] => {
                let (index, day) = indexed(code)?;
                json!({"type": "relativeMonthly", "interval": interval,
                       "daysOfWeek": [day], "index": index})
            }
            _ => return Err(CalendarError::Unsupported),
        },
        "yearly" => match described.by_day.as_slice() {
            [] => json!({"type": "absoluteYearly", "interval": interval,
                         "month": month, "dayOfMonth": day_of_month}),
            [code] => {
                let (index, day) = indexed(code)?;
                json!({"type": "relativeYearly", "interval": interval, "month": month,
                       "daysOfWeek": [day], "index": index})
            }
            _ => return Err(CalendarError::Unsupported),
        },
        _ => return Err(CalendarError::Unsupported),
    };
    let start_date = date.format("%Y-%m-%d").to_string();
    let zone = match &start.kind {
        TimeKind::Zoned(zone) => zone.name(),
        _ => "UTC",
    };
    let range = match (&described.count, &described.until) {
        (Some(count), _) => json!({"type": "numbered", "startDate": start_date,
                                   "numberOfOccurrences": count, "recurrenceTimeZone": zone}),
        (None, Some(until)) => json!({"type": "endDate", "startDate": start_date,
                                      "endDate": clip(until, 10), "recurrenceTimeZone": zone}),
        (None, None) => json!({"type": "noEnd", "startDate": start_date,
                               "recurrenceTimeZone": zone}),
    };
    Ok(json!({"pattern": pattern, "range": range}))
}

fn convert(g: &GEvent) -> Option<Event> {
    let all_day = g.is_all_day == Some(true);
    let start = time(g.start.as_ref()?, all_day)?;
    let end = g
        .end
        .as_ref()
        .and_then(|end| time(end, all_day))
        .filter(|end| end.to_utc() >= start.to_utc())
        .unwrap_or_else(|| start.clone());
    // A date range needs at least one day.
    let end = if all_day && end.local <= start.local {
        start.plus(Duration::days(1))
    } else {
        end
    };
    let organizer = g
        .organizer
        .as_ref()
        .and_then(|recipient| recipient.email.as_ref())
        .and_then(person)
        .map(|(email, name)| Person {
            email,
            name,
            partstat: PartStat::Accepted,
        });
    let mut event = Event {
        uid: g.id.clone(),
        summary: clip(g.subject.as_deref().unwrap_or("").trim(), MAX_SUMMARY_CHARS),
        description: clip(
            g.body
                .as_ref()
                .and_then(|body| body.content.as_deref())
                .unwrap_or(""),
            MAX_DESCRIPTION_BYTES,
        ),
        location: clip(
            g.location
                .as_ref()
                .and_then(|location| location.display_name.as_deref())
                .unwrap_or("")
                .trim(),
            MAX_LOCATION_BYTES,
        ),
        start,
        end,
        status: match g.show_as.as_deref() {
            Some("tentative") => EventStatus::Tentative,
            _ => EventStatus::Confirmed,
        },
        organizer,
        attendees: g
            .attendees
            .iter()
            .filter_map(|attendee| {
                let (email, name) = person(attendee.email.as_ref()?)?;
                Some(Person {
                    email,
                    name,
                    partstat: match attendee
                        .status
                        .as_ref()
                        .and_then(|status| status.response.as_deref())
                    {
                        Some("accepted" | "organizer") => PartStat::Accepted,
                        Some("declined") => PartStat::Declined,
                        Some("tentativelyAccepted") => PartStat::Tentative,
                        _ => PartStat::NeedsAction,
                    },
                })
            })
            .take(ical::MAX_ATTENDEES)
            .collect(),
        alarms: g
            .reminder_minutes
            .filter(|minutes| g.is_reminder_on == Some(true) && (0..=40_320).contains(minutes))
            .map(|minutes| Alarm {
                minutes_before: minutes,
                from_end: false,
                action: "DISPLAY".to_owned(),
            })
            .into_iter()
            .collect(),
        url: g
            .online_meeting
            .as_ref()
            .and_then(|meeting| meeting.join_url.as_deref())
            .filter(|link| link.starts_with("https://") && link.len() <= 2_000)
            .map(str::to_owned),
        sequence: 0,
        rrule: None,
        rdates: Vec::new(),
        exdates: Vec::new(),
        recurrence_id: None,
        overrides: Vec::new(),
    };
    if g.kind.as_deref() == Some("seriesMaster") {
        event.rrule = g
            .recurrence
            .as_ref()
            .and_then(|recurrence| rrule_from(recurrence, &event.start));
        if event.rrule.is_some() {
            for text in g.cancelled_occurrences.iter().take(ical::MAX_LIST) {
                // "OID.<series id>.<yyyy-MM-dd>": the day names the occurrence.
                let Some(day) = text.get(text.len().saturating_sub(10)..) else {
                    continue;
                };
                let Ok(date) = NaiveDate::parse_from_str(day, "%Y-%m-%d") else {
                    continue;
                };
                event.exdates.push(if event.start.is_date() {
                    EventTime::date(date)
                } else {
                    EventTime {
                        kind: event.start.kind.clone(),
                        local: date.and_time(event.start.local.time()),
                    }
                });
            }
        }
    }
    Some(event)
}

/// Turns one event resource (or `@removed` stub) into an item; `None` for
/// data that cannot be understood.
pub fn parse_event(value: &Value) -> Option<Item> {
    let g: GEvent = serde_json::from_value(value.clone()).ok()?;
    if !id_ok(&g.id) {
        return None;
    }
    if g.removed.is_some() || g.is_cancelled == Some(true) {
        return Some(Item::Gone { href: g.id });
    }
    Some(Item::Event {
        href: g.id.clone(),
        etag: g.etag.clone().filter(|etag| etag.len() <= 1_024),
        event: convert(&g)?,
    })
}

// -- paging ---------------------------------------------------------------------

/// A link the server sent is followed only on Graph's own host.
fn trusted_link(link: &str) -> Result<Url> {
    let url = Url::parse(link).map_err(|_| CalendarError::Server)?;
    if url.scheme() != "https"
        || url.host_str() != Some(HOST)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(CalendarError::Server);
    }
    Ok(url)
}

#[derive(Default, Deserialize)]
struct Page {
    #[serde(default)]
    value: Vec<Value>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

fn get(url: Url) -> HttpRequest {
    HttpRequest::new(Method::Get, url)
        .header("accept", "application/json")
        .header("prefer", PREFER)
}

/// The URL of a calendar's collection.
pub fn calendar_url(id: &str) -> String {
    format!("{API_ROOT}/me/calendars/{}", encode_segment(id))
}

fn owned_calendar(calendar_url: &str) -> Result<()> {
    if calendar_url.starts_with(&format!("{API_ROOT}/me/calendars/")) {
        Ok(())
    } else {
        Err(CalendarError::InvalidInput)
    }
}

#[derive(Deserialize)]
struct CalendarEntry {
    id: String,
    name: Option<String>,
    #[serde(rename = "hexColor")]
    hex_color: Option<String>,
    #[serde(rename = "canEdit")]
    can_edit: Option<bool>,
    owner: Option<GAddress>,
}

/// Lists the calendars of the signed-in account.
pub async fn calendars(api: &Api<'_>) -> Result<Vec<RemoteCalendar>> {
    let mut out = Vec::new();
    let mut url = Url::parse(&format!("{API_ROOT}/me/calendars?%24top=100"))
        .map_err(|_| CalendarError::Server)?;
    for _ in 0..MAX_PAGES {
        let response = api.call(get(url.clone())).await?;
        check_status(response.status)?;
        let page: Page = parse_json(&response)?;
        for value in &page.value {
            let Ok(entry) = serde_json::from_value::<CalendarEntry>(value.clone()) else {
                continue;
            };
            if !id_ok(&entry.id) {
                continue;
            }
            if out.len() >= MAX_CALENDARS_PER_ACCOUNT {
                return Err(CalendarError::Limit);
            }
            out.push(RemoteCalendar {
                url: calendar_url(&entry.id),
                name: entry
                    .name
                    .as_deref()
                    .map(|name| clip(name.trim(), 200))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "Calendar".to_owned()),
                color: entry.hex_color.as_deref().and_then(hex_color),
                read_only: entry.can_edit != Some(true),
                owner: entry
                    .owner
                    .as_ref()
                    .and_then(person)
                    .map(|(address, _)| address),
            });
        }
        match page.next_link {
            Some(link) => url = trusted_link(&link)?,
            None => return Ok(out),
        }
    }
    Err(CalendarError::Limit)
}

// -- sync ------------------------------------------------------------------------

fn window(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    (
        now - Duration::days(WINDOW_PAST_DAYS),
        now + Duration::days(WINDOW_FUTURE_DAYS),
    )
}

/// A stored token is `g1|<window end, unix seconds>|<delta link>`; it is used
/// while the window still reaches far enough ahead.
fn saved_link(token: Option<&str>, now: DateTime<Utc>) -> Option<Url> {
    let mut parts = token?.splitn(3, '|');
    if parts.next()? != TOKEN_PREFIX {
        return None;
    }
    let end: i64 = parts.next()?.parse().ok()?;
    if end - now.timestamp() < Duration::days(RESYNC_BEFORE_END_DAYS).num_seconds() {
        return None;
    }
    trusted_link(parts.next()?).ok()
}

struct Expired;

async fn pages(
    api: &Api<'_>,
    mut url: Url,
    window_end: DateTime<Utc>,
    incremental: bool,
) -> Result<std::result::Result<Delta, Expired>> {
    let mut delta = Delta {
        full: !incremental,
        ..Delta::default()
    };
    for _ in 0..MAX_PAGES {
        let response = api.call(get(url.clone())).await?;
        // A delta link that expired answers 410 (`syncStateNotFound`).
        if response.status == 410 && incremental {
            return Ok(Err(Expired));
        }
        check_status(response.status)?;
        let page: Page = parse_json(&response)?;
        delta
            .items
            .extend(page.value.iter().filter_map(parse_event));
        if delta.items.len() > ical::MAX_EVENTS {
            return Err(CalendarError::Limit);
        }
        if let Some(link) = page.next_link {
            url = trusted_link(&link)?;
            continue;
        }
        if let Some(link) = page.delta_link {
            trusted_link(&link)?;
            let token = format!("{TOKEN_PREFIX}|{}|{link}", window_end.timestamp());
            delta.next_token = (token.len() <= MAX_TOKEN_BYTES).then_some(token);
        }
        return Ok(Ok(delta));
    }
    Err(CalendarError::Limit)
}

/// Reads the changes since `token`, or the expanded window when there is no
/// usable token. A delta link the provider no longer knows (`410`) falls back
/// to reading the window again; the result is then marked [`Delta::full`].
pub async fn sync(
    api: &Api<'_>,
    calendar_url: &str,
    token: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Delta> {
    owned_calendar(calendar_url)?;
    let (from, to) = window(now);
    if let Some(link) = saved_link(token, now) {
        let end = token
            .and_then(|token| token.split('|').nth(1))
            .and_then(|end| end.parse::<i64>().ok())
            .and_then(|end| DateTime::from_timestamp(end, 0))
            .unwrap_or(to);
        if let Ok(delta) = pages(api, link, end, true).await? {
            return Ok(delta);
        }
    }
    let mut url = Url::parse(&format!("{calendar_url}/calendarView/delta"))
        .map_err(|_| CalendarError::InvalidInput)?;
    url.query_pairs_mut()
        .append_pair(
            "startDateTime",
            &from.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        )
        .append_pair("endDateTime", &to.format("%Y-%m-%dT%H:%M:%SZ").to_string());
    match pages(api, url, to, false).await? {
        Ok(delta) => Ok(delta),
        Err(Expired) => Err(CalendarError::Server),
    }
}

// -- writing ---------------------------------------------------------------------

fn time_json(time: &EventTime) -> Value {
    let zone = match &time.kind {
        TimeKind::Zoned(zone) => zone.name(),
        _ => "UTC",
    };
    json!({
        "dateTime": time.local.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "timeZone": zone,
    })
}

/// The JSON body that writes `event`. `series` adds the repeat rule (only
/// possible when creating; an existing occurrence keeps its series).
pub fn event_body(event: &Event, series: bool) -> Result<Value> {
    let reminder = event
        .alarms
        .iter()
        .find(|alarm| !alarm.from_end && alarm.minutes_before >= 0);
    let mut body = json!({
        "subject": event.summary,
        "body": {"contentType": "text", "content": event.description},
        "start": time_json(&event.start),
        "end": time_json(&event.end),
        "isAllDay": event.start.is_date(),
        "location": {"displayName": event.location},
        "isReminderOn": reminder.is_some(),
    });
    if let Some(alarm) = reminder {
        body["reminderMinutesBeforeStart"] = json!(alarm.minutes_before);
    }
    if series {
        if let Some(rule) = &event.rrule {
            body["recurrence"] = recurrence_json(rule, &event.start)?;
        }
    }
    Ok(body)
}

fn json_request(method: Method, url: Url, body: &Value) -> Result<HttpRequest> {
    let bytes = serde_json::to_vec(body).map_err(|_| CalendarError::InvalidInput)?;
    let mut request = get(url).body("application/json", bytes);
    request.method = method;
    Ok(request)
}

fn item_of(response: &HttpResponse) -> Result<Item> {
    check_status(response.status)?;
    let value: Value = parse_json(response)?;
    parse_event(&value).ok_or(CalendarError::Parse)
}

fn event_url(id: &str) -> Result<Url> {
    if !id_ok(id) {
        return Err(CalendarError::InvalidInput);
    }
    Url::parse(&format!("{API_ROOT}/me/events/{}", encode_segment(id)))
        .map_err(|_| CalendarError::InvalidInput)
}

/// Creates an event. A series comes back as its master, which the caller must
/// not cache (the expanded instances arrive with the next sync).
pub async fn create(api: &Api<'_>, calendar_url: &str, event: &Event) -> Result<Item> {
    owned_calendar(calendar_url)?;
    let url =
        Url::parse(&format!("{calendar_url}/events")).map_err(|_| CalendarError::InvalidInput)?;
    let request = json_request(Method::Post, url, &event_body(event, true)?)?;
    item_of(&api.call(request).await?)
}

/// Changes one event or occurrence (`id` is the cached event's id).
pub async fn update(api: &Api<'_>, event: &Event) -> Result<Item> {
    let request = json_request(
        Method::Patch,
        event_url(&event.uid)?,
        &event_body(event, false)?,
    )?;
    item_of(&api.call(request).await?)
}

/// Deletes one event or occurrence.
pub async fn delete(api: &Api<'_>, id: &str) -> Result<()> {
    let response = api
        .call(HttpRequest::new(Method::Delete, event_url(id)?))
        .await?;
    match response.status {
        404 | 410 => Ok(()),
        _ => check_status(response.status),
    }
}
