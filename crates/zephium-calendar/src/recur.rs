//! Bounded occurrence expansion for one event: RRULE, RDATE, EXDATE and
//! RECURRENCE-ID overrides.

use crate::error::{CalendarError, Result};
use crate::model::{Event, EventStatus, EventTime, TimeKind};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use rrule::{RRuleSet, Tz};
use std::str::FromStr;

/// Most occurrences produced for one event in one query.
pub const MAX_OCCURRENCES_PER_EVENT: usize = 1_000;

/// One concrete instance of an event.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence<'a> {
    /// Event that supplies the text: the master, or the override.
    pub event: &'a Event,
    /// Start of this instance.
    pub start: EventTime,
    /// End of this instance.
    pub end: EventTime,
    /// Original start of this instance in the series (RECURRENCE-ID), when
    /// the event repeats or is an override.
    pub instance: Option<EventTime>,
}

/// Instances found for one event.
#[derive(Debug, Default)]
pub struct Expansion<'a> {
    /// Instances overlapping the range, unsorted.
    pub occurrences: Vec<Occurrence<'a>>,
    /// `true` when a limit cut the result short.
    pub truncated: bool,
}

fn overlaps(start: &EventTime, end: &EventTime, from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
    let (s, e) = (start.to_utc(), end.to_utc());
    if e <= s {
        s >= from && s < to
    } else {
        s < to && e > from
    }
}

fn same_instance(a: &EventTime, b: &EventTime) -> bool {
    if a.is_date() || b.is_date() {
        a.local.date() == b.local.date()
    } else {
        a.to_utc() == b.to_utc()
    }
}

/// Rewrites UNTIL into a UTC instant, the one form the generator accepts for
/// every kind of DTSTART.
fn normalize_rule(rule: &str, start: &EventTime) -> Option<String> {
    let mut parts = Vec::new();
    for part in rule.split(';') {
        let (key, value) = part.split_once('=')?;
        let key = key.trim().to_ascii_uppercase();
        if key == "UNTIL" {
            let value = value.trim();
            let instant = if let Some(stripped) = value.strip_suffix(['Z', 'z']) {
                NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S")
                    .ok()?
                    .and_utc()
            } else if value.len() == 8 {
                let day_end = NaiveDate::parse_from_str(value, "%Y%m%d")
                    .ok()?
                    .and_time(NaiveTime::from_hms_opt(23, 59, 59)?);
                resolve_in(start, day_end)
            } else {
                let local = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
                resolve_in(start, local)
            };
            parts.push(format!("UNTIL={}", instant.format("%Y%m%dT%H%M%SZ")));
        } else {
            parts.push(format!("{key}={}", value.trim()));
        }
    }
    Some(parts.join(";"))
}

fn resolve_in(start: &EventTime, local: NaiveDateTime) -> DateTime<Utc> {
    match &start.kind {
        TimeKind::Zoned(tz) => crate::model::resolve_local(*tz, local),
        _ => local.and_utc(),
    }
}

/// Checks that `rule` is a usable RRULE for an event starting at `start`.
pub fn validate_rule(rule: &str, start: &EventTime) -> Result<()> {
    build_set(rule, start).map(drop)
}

fn build_set(rule: &str, start: &EventTime) -> Result<RRuleSet> {
    let rule = normalize_rule(rule, start).ok_or(CalendarError::InvalidInput)?;
    let dtstart = match &start.kind {
        TimeKind::Zoned(tz) => format!(
            "DTSTART;TZID={}:{}",
            tz.name(),
            start.local.format("%Y%m%dT%H%M%S")
        ),
        _ => format!("DTSTART:{}Z", start.local.format("%Y%m%dT%H%M%S")),
    };
    RRuleSet::from_str(&format!("{dtstart}\nRRULE:{rule}")).map_err(|_| CalendarError::InvalidInput)
}

fn instance_from(start: &EventTime, generated: &DateTime<Tz>) -> EventTime {
    match &start.kind {
        TimeKind::Zoned(tz) => EventTime {
            kind: TimeKind::Zoned(*tz),
            local: generated.naive_local(),
        },
        TimeKind::Date => EventTime::date(generated.date_naive()),
        kind => EventTime {
            kind: kind.clone(),
            local: generated.naive_utc(),
        },
    }
}

/// Expands `event` for `[from, to)`, producing at most `max` instances.
pub fn expand(event: &Event, from: DateTime<Utc>, to: DateTime<Utc>, max: usize) -> Expansion<'_> {
    let mut out = Expansion::default();
    let max = max.min(MAX_OCCURRENCES_PER_EVENT);
    if to <= from || event.status == EventStatus::Cancelled {
        return out;
    }
    let delta = if event.start.is_date() || matches!(event.start.kind, TimeKind::Floating) {
        event.end.local - event.start.local
    } else {
        event.end.since(&event.start)
    };

    if !event.is_recurring() {
        if overlaps(&event.start, &event.end, from, to) {
            out.occurrences.push(Occurrence {
                event,
                start: event.start.clone(),
                end: event.end.clone(),
                instance: None,
            });
        }
        return out;
    }

    let mut instances: Vec<EventTime> = Vec::new();
    if let Some(rule) = &event.rrule {
        if let Ok(set) = build_set(rule, &event.start) {
            let after = from - delta - Duration::days(1);
            let set = set
                .after(after.with_timezone(&Tz::UTC))
                .before(to.with_timezone(&Tz::UTC));
            let limit = u16::try_from(max.saturating_add(1)).unwrap_or(u16::MAX);
            let result = set.all(limit);
            out.truncated |= result.limited;
            instances.extend(
                result
                    .dates
                    .iter()
                    .map(|generated| instance_from(&event.start, generated)),
            );
        } else {
            // An unusable rule degrades to the first instance only.
            instances.push(event.start.clone());
        }
    } else {
        instances.push(event.start.clone());
    }
    instances.extend(event.rdates.iter().cloned());

    let overridden = |instance: &EventTime| -> bool {
        event.overrides.iter().any(|item| {
            item.recurrence_id
                .as_ref()
                .is_some_and(|id| same_instance(id, instance))
        })
    };
    let mut seen: Vec<EventTime> = Vec::new();
    for instance in instances {
        if event
            .exdates
            .iter()
            .any(|excluded| same_instance(excluded, &instance))
            || overridden(&instance)
            || seen.iter().any(|known| same_instance(known, &instance))
        {
            continue;
        }
        let end = instance.plus(delta);
        if !overlaps(&instance, &end, from, to) {
            continue;
        }
        if out.occurrences.len() >= max {
            out.truncated = true;
            break;
        }
        seen.push(instance.clone());
        out.occurrences.push(Occurrence {
            event,
            instance: Some(instance.clone()),
            start: instance,
            end,
        });
    }

    // Overrides live at their own time, which may differ from the instance
    // they replace.
    for item in &event.overrides {
        if item.status == EventStatus::Cancelled {
            continue;
        }
        if overlaps(&item.start, &item.end, from, to) {
            if out.occurrences.len() >= max {
                out.truncated = true;
                break;
            }
            out.occurrences.push(Occurrence {
                event: item,
                start: item.start.clone(),
                end: item.end.clone(),
                instance: item.recurrence_id.clone(),
            });
        }
    }
    out
}

/// `true` when `instance` is one of the generated instances of `master`
/// (ignoring its EXDATEs and overrides): the RECURRENCE-ID of an occurrence
/// that can be edited or deleted on its own.
pub fn is_instance(master: &Event, instance: &EventTime) -> bool {
    if !master.is_recurring() {
        return false;
    }
    let mut probe = master.clone();
    probe.overrides.clear();
    probe.exdates.clear();
    let from = instance.to_utc();
    let found = expand(
        &probe,
        from,
        from + Duration::seconds(1),
        MAX_OCCURRENCES_PER_EVENT,
    )
    .occurrences
    .iter()
    .any(|occurrence| {
        occurrence
            .instance
            .as_ref()
            .is_some_and(|known| same_instance(known, instance))
    });
    found
}

/// `true` when two times denote the same instance of a series.
pub fn same_time(a: &EventTime, b: &EventTime) -> bool {
    same_instance(a, b)
}

/// End of the instance of `master` that starts at `instance`.
pub fn instance_end(master: &Event, instance: &EventTime) -> EventTime {
    let delta = if master.start.is_date() || matches!(master.start.kind, TimeKind::Floating) {
        master.end.local - master.start.local
    } else {
        master.end.since(&master.start)
    };
    instance.plus(delta)
}

/// Midnight-UTC helper for tests and callers that hold a date.
pub fn utc_midnight(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN))
}
