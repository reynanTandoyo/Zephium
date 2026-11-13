//! Wire types shared with the desktop IPC (exported to TypeScript) and the
//! conversions between them and the iCalendar model.

use crate::error::{CalendarError, Result};
use crate::model::{Alarm, Event, EventStatus, EventTime, TimeKind};
use crate::recur::{self, Occurrence};
use crate::tz;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a calendar comes from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CalendarAccountKindV1 {
    /// A CalDAV server with a user name and password.
    Caldav,
    /// A read-only `.ics` address.
    Subscription,
}

/// A CalDAV account or an ICS subscription. Never holds a secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarAccountV1 {
    /// Stable id.
    pub id: String,
    /// Account or subscription kind.
    pub kind: CalendarAccountKindV1,
    /// Display name.
    pub name: String,
    /// Server or feed address.
    pub server_url: String,
    /// CalDAV user name.
    pub username: Option<String>,
}

/// One calendar of an account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarV1 {
    /// Stable id.
    pub id: String,
    /// Owning account.
    pub account_id: String,
    /// Collection address.
    pub url: String,
    /// Display name.
    pub name: String,
    /// `#RRGGBB`.
    pub color: Option<String>,
    /// Events cannot be changed.
    pub read_only: bool,
    /// Shown in `calendar_events`.
    pub visible: bool,
}

/// Where a login address was found.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarDiscoveryV1 {
    /// Address to sign in at.
    pub server_url: String,
    /// Suggested user name (the email address when one was given).
    pub username: String,
}

/// A person on an event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarPersonV1 {
    /// Address.
    pub email: String,
    /// Display name.
    pub name: Option<String>,
    /// `needs_action`, `accepted`, `declined`, `tentative` or `delegated`.
    pub partstat: String,
}

/// One concrete instance of an event.
///
/// `start`/`end` are `YYYY-MM-DD` for all-day events, `YYYY-MM-DDTHH:MM:SSZ`
/// for instants, and `YYYY-MM-DDTHH:MM:SS` for floating (viewer-local) times.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarOccurrenceV1 {
    /// Calendar holding the event.
    pub calendar_id: String,
    /// Event UID.
    pub uid: String,
    /// Start of this instance in its series, same format as `start`.
    pub recurrence_id: Option<String>,
    /// Title.
    pub summary: String,
    /// Notes.
    pub description: String,
    /// Place.
    pub location: String,
    /// Start.
    pub start: String,
    /// End (exclusive for all-day).
    pub end: String,
    /// Whole-day event.
    pub all_day: bool,
    /// IANA zone of the event, when it has one.
    pub tzid: Option<String>,
    /// Calendar colour.
    pub color: Option<String>,
    /// `none`, `confirmed`, `tentative` or `cancelled`.
    pub status: String,
    /// The event repeats.
    pub recurring: bool,
    /// The calendar is read-only.
    pub read_only: bool,
    /// Link (for example a meeting).
    pub url: Option<String>,
    /// Organizer.
    pub organizer: Option<CalendarPersonV1>,
    /// Attendees.
    pub attendees: Vec<CalendarPersonV1>,
    /// Reminders as minutes before the start.
    pub reminders: Vec<i32>,
}

/// Occurrences across the visible calendars.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarEventsV1 {
    /// Sorted by start.
    pub occurrences: Vec<CalendarOccurrenceV1>,
    /// A size limit cut the list short.
    pub truncated: bool,
}

/// Fields the user edits. Times are `YYYY-MM-DD` (all-day, `end` exclusive) or
/// `YYYY-MM-DDTHH:MM:SS`: wall-clock in `tzid` when given, UTC with a `Z`
/// suffix, otherwise floating.
#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarEventDraftV1 {
    /// Title.
    pub summary: String,
    /// Notes.
    pub description: String,
    /// Place.
    pub location: String,
    /// Start.
    pub start: String,
    /// End.
    pub end: String,
    /// Whole-day event.
    pub all_day: bool,
    /// IANA zone for wall-clock times.
    pub tzid: Option<String>,
    /// RRULE value such as `FREQ=WEEKLY;BYDAY=MO`.
    pub rrule: Option<String>,
    /// Reminders as minutes before the start.
    pub reminders: Vec<u32>,
    /// Link.
    pub url: Option<String>,
}

/// An event on a calendar.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarEventRefV1 {
    /// Calendar id.
    pub calendar_id: String,
    /// Event UID.
    pub uid: String,
}

/// What one calendar's refresh did.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarSyncOutcomeV1 {
    /// Calendar id.
    pub calendar_id: String,
    /// Why it failed, when it did.
    pub error: Option<CalendarError>,
}

/// Result of a refresh.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarSyncReportV1 {
    /// One entry per calendar that was refreshed.
    pub outcomes: Vec<CalendarSyncOutcomeV1>,
}

// -- conversions -------------------------------------------------------------

fn format_time(time: &EventTime) -> String {
    match &time.kind {
        TimeKind::Date => time.local.format("%Y-%m-%d").to_string(),
        TimeKind::Floating => time.local.format("%Y-%m-%dT%H:%M:%S").to_string(),
        _ => time.to_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

fn person(value: &crate::model::Person) -> CalendarPersonV1 {
    CalendarPersonV1 {
        email: value.email.clone(),
        name: value.name.clone(),
        partstat: value.partstat.as_str().to_owned(),
    }
}

/// Builds the wire form of one occurrence.
pub fn occurrence_wire(
    occurrence: &Occurrence<'_>,
    calendar_id: &str,
    color: Option<&str>,
    read_only: bool,
    recurring: bool,
) -> CalendarOccurrenceV1 {
    let event = occurrence.event;
    CalendarOccurrenceV1 {
        calendar_id: calendar_id.to_owned(),
        uid: event.uid.clone(),
        recurrence_id: occurrence.instance.as_ref().map(format_time),
        summary: event.summary.clone(),
        description: event.description.clone(),
        location: event.location.clone(),
        start: format_time(&occurrence.start),
        end: format_time(&occurrence.end),
        all_day: occurrence.start.is_date(),
        tzid: match &occurrence.start.kind {
            TimeKind::Zoned(zone) => Some(zone.name().to_owned()),
            _ => None,
        },
        color: color.map(str::to_owned),
        status: event.status.as_str().to_owned(),
        recurring,
        read_only,
        url: event.url.clone(),
        organizer: event.organizer.as_ref().map(person),
        attendees: event.attendees.iter().map(person).collect(),
        reminders: event
            .alarms
            .iter()
            .filter(|alarm| !alarm.from_end)
            .filter_map(|alarm| i32::try_from(alarm.minutes_before).ok())
            .collect(),
    }
}

fn parse_draft_time(text: &str, all_day: bool, tzid: Option<&str>) -> Result<EventTime> {
    let text = text.trim();
    if all_day {
        let date =
            NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| CalendarError::InvalidInput)?;
        return Ok(EventTime::date(date));
    }
    let is_utc = text.ends_with(['Z', 'z']);
    let local =
        NaiveDateTime::parse_from_str(text.trim_end_matches(['Z', 'z']), "%Y-%m-%dT%H:%M:%S")
            .map_err(|_| CalendarError::InvalidInput)?;
    match (tzid, is_utc) {
        (Some(name), false) => Ok(EventTime {
            kind: TimeKind::Zoned(tz::resolve(name).ok_or(CalendarError::InvalidInput)?),
            local,
        }),
        (Some(name), true) => {
            let zone = tz::resolve(name).ok_or(CalendarError::InvalidInput)?;
            Ok(EventTime {
                kind: TimeKind::Zoned(zone),
                local: local.and_utc().with_timezone(&zone).naive_local(),
            })
        }
        (None, true) => Ok(EventTime {
            kind: TimeKind::Utc,
            local,
        }),
        (None, false) => Ok(EventTime {
            kind: TimeKind::Floating,
            local,
        }),
    }
}

const MAX_DRAFT_TEXT: usize = 100_000;

/// Applies `draft` to `base` (an existing master) or builds a new event.
/// Attendees, organizer, status and unrelated properties of `base` survive;
/// exceptions are dropped when the start moves.
pub fn apply_draft(uid: &str, draft: &CalendarEventDraftV1, base: Option<&Event>) -> Result<Event> {
    if draft.summary.chars().count() > 1_000
        || draft.location.len() > 2_000
        || draft.description.len() > MAX_DRAFT_TEXT
        || draft.reminders.len() > 20
        || draft.url.as_ref().is_some_and(|url| url.len() > 2_000)
    {
        return Err(CalendarError::InvalidInput);
    }
    let start = parse_draft_time(&draft.start, draft.all_day, draft.tzid.as_deref())?;
    let end = parse_draft_time(&draft.end, draft.all_day, draft.tzid.as_deref())?;
    if end.to_utc() < start.to_utc() || (draft.all_day && end.local <= start.local) {
        return Err(CalendarError::InvalidInput);
    }
    let rrule = draft
        .rrule
        .as_ref()
        .map(|rule| rule.trim().to_owned())
        .filter(|rule| !rule.is_empty())
        .map(|rule| {
            let rule = rule.strip_prefix("RRULE:").unwrap_or(&rule).to_owned();
            recur::validate_rule(&rule, &start).map(|()| rule)
        })
        .transpose()?;
    let moved = base.is_some_and(|event| event.start != start);
    let mut event = base.cloned().unwrap_or_else(|| Event {
        uid: uid.to_owned(),
        summary: String::new(),
        description: String::new(),
        location: String::new(),
        start: start.clone(),
        end: end.clone(),
        status: EventStatus::None,
        organizer: None,
        attendees: Vec::new(),
        alarms: Vec::new(),
        url: None,
        sequence: 0,
        rrule: None,
        rdates: Vec::new(),
        exdates: Vec::new(),
        recurrence_id: None,
        overrides: Vec::new(),
    });
    if moved {
        event.exdates.clear();
        event.overrides.clear();
        event.rdates.clear();
    }
    event.summary = draft.summary.trim().to_owned();
    event.description = draft.description.clone();
    event.location = draft.location.trim().to_owned();
    event.start = start;
    event.end = end;
    event.rrule = rrule;
    event.url = draft
        .url
        .as_ref()
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty());
    event.alarms = draft
        .reminders
        .iter()
        .map(|minutes| Alarm {
            minutes_before: i64::from(*minutes),
            from_end: false,
            action: "DISPLAY".to_owned(),
        })
        .collect();
    if base.is_some() {
        event.sequence = event.sequence.saturating_add(1);
    }
    Ok(event)
}

/// Parses an RFC 3339 instant for range queries.
pub fn parse_instant(text: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim())
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| CalendarError::InvalidInput)
}
