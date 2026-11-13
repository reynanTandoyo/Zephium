//! Safe, owned iCalendar model. Everything here is plain data.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

/// How an iCalendar time is anchored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimeKind {
    /// A whole day (`VALUE=DATE`).
    Date,
    /// An absolute instant (`...Z`).
    Utc,
    /// Wall-clock time in an IANA zone (`TZID=`).
    Zoned(chrono_tz::Tz),
    /// Wall-clock time without a zone; read as the viewer's local time.
    Floating,
}

/// One start, end, RDATE, EXDATE or RECURRENCE-ID value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventTime {
    /// How `local` is anchored.
    pub kind: TimeKind,
    /// Wall-clock value. For [`TimeKind::Date`] the time is midnight; for
    /// [`TimeKind::Utc`] it is the UTC value.
    pub local: NaiveDateTime,
}

impl EventTime {
    /// A whole-day value.
    pub fn date(date: NaiveDate) -> Self {
        Self {
            kind: TimeKind::Date,
            local: date.and_time(NaiveTime::MIN),
        }
    }

    /// An absolute instant.
    pub fn utc(instant: DateTime<Utc>) -> Self {
        Self {
            kind: TimeKind::Utc,
            local: instant.naive_utc(),
        }
    }

    /// `true` for whole-day values.
    pub fn is_date(&self) -> bool {
        self.kind == TimeKind::Date
    }

    /// The instant this value denotes. Dates and floating times are read as
    /// UTC so they sort consistently; a wall-clock time skipped by a DST jump
    /// moves forward by the gap.
    pub fn to_utc(&self) -> DateTime<Utc> {
        match &self.kind {
            TimeKind::Zoned(tz) => resolve_local(*tz, self.local),
            _ => self.local.and_utc(),
        }
    }

    /// The same anchoring, shifted by an absolute duration.
    pub fn plus(&self, delta: Duration) -> Self {
        match &self.kind {
            TimeKind::Zoned(tz) => Self {
                kind: self.kind.clone(),
                local: (self.to_utc() + delta).with_timezone(tz).naive_local(),
            },
            _ => Self {
                kind: self.kind.clone(),
                local: self.local + delta,
            },
        }
    }

    /// Absolute duration from `earlier` to `self`.
    pub fn since(&self, earlier: &Self) -> Duration {
        self.to_utc() - earlier.to_utc()
    }
}

/// Maps a wall-clock value to an instant: a time skipped by a DST jump moves
/// forward by the gap, a repeated one takes its first occurrence.
pub fn resolve_local(tz: chrono_tz::Tz, local: NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(&local) {
        chrono::LocalResult::Single(value) => value.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        chrono::LocalResult::None => {
            // Inside a spring-forward gap: use the offset in force before it.
            let offset = match tz.from_local_datetime(&(local - Duration::hours(3))) {
                chrono::LocalResult::Single(value) | chrono::LocalResult::Ambiguous(value, _) => {
                    i64::from(value.offset().fix().local_minus_utc())
                }
                chrono::LocalResult::None => 0,
            };
            (local - Duration::seconds(offset)).and_utc()
        }
    }
}

use chrono::Offset;

/// STATUS of an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventStatus {
    /// No STATUS or an unknown one.
    None,
    /// `CONFIRMED`.
    Confirmed,
    /// `TENTATIVE`.
    Tentative,
    /// `CANCELLED`.
    Cancelled,
}

impl EventStatus {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Confirmed => "confirmed",
            Self::Tentative => "tentative",
            Self::Cancelled => "cancelled",
        }
    }
}

/// PARTSTAT of an attendee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PartStat {
    /// `NEEDS-ACTION` (the default).
    NeedsAction,
    /// `ACCEPTED`.
    Accepted,
    /// `DECLINED`.
    Declined,
    /// `TENTATIVE`.
    Tentative,
    /// `DELEGATED`.
    Delegated,
}

impl PartStat {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NeedsAction => "needs_action",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Tentative => "tentative",
            Self::Delegated => "delegated",
        }
    }
}

/// An organizer or attendee.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Person {
    /// Address without the `mailto:` prefix.
    pub email: String,
    /// `CN` parameter.
    pub name: Option<String>,
    /// Participation status (attendees only).
    pub partstat: PartStat,
}

/// A VALARM with a trigger relative to the event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Alarm {
    /// Minutes before the start; negative means after.
    pub minutes_before: i64,
    /// `true` when relative to the end instead of the start.
    pub from_end: bool,
    /// ACTION (`DISPLAY`, `AUDIO`, `EMAIL`), upper-case.
    pub action: String,
}

/// A parsed VEVENT with its exceptions.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// UID.
    pub uid: String,
    /// SUMMARY.
    pub summary: String,
    /// DESCRIPTION.
    pub description: String,
    /// LOCATION.
    pub location: String,
    /// DTSTART.
    pub start: EventTime,
    /// DTEND or DTSTART + DURATION; defaults per RFC 5545.
    pub end: EventTime,
    /// STATUS.
    pub status: EventStatus,
    /// ORGANIZER.
    pub organizer: Option<Person>,
    /// ATTENDEE list.
    pub attendees: Vec<Person>,
    /// VALARMs.
    pub alarms: Vec<Alarm>,
    /// URL.
    pub url: Option<String>,
    /// SEQUENCE.
    pub sequence: u32,
    /// RRULE value (validated), without the `RRULE:` prefix.
    pub rrule: Option<String>,
    /// RDATE values.
    pub rdates: Vec<EventTime>,
    /// EXDATE values.
    pub exdates: Vec<EventTime>,
    /// RECURRENCE-ID when this is an override of one occurrence.
    pub recurrence_id: Option<EventTime>,
    /// Overrides of single occurrences (masters only).
    pub overrides: Vec<Event>,
}

impl Event {
    /// `true` when the event repeats.
    pub fn is_recurring(&self) -> bool {
        self.rrule.is_some() || !self.rdates.is_empty()
    }
}
