//! A simple description of a repeat rule for the frame, and the way back to
//! an RRULE. The description covers daily/weekly/monthly/yearly repeats with
//! an interval, a count or an end, and BYDAY / BYMONTHDAY / BYMONTH lists; a
//! rule with other parts (BYSETPOS, BYYEARDAY, BYHOUR, a WKST other than
//! Monday, ...) is described as far as it goes and flagged `custom`, so the
//! frame can keep such a rule through `rrule` instead of rebuilding it.

use crate::api::CalendarRecurrenceV1;
use crate::error::{CalendarError, Result};
use crate::model::{resolve_local, EventTime, TimeKind};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

const MAX_INTERVAL: u32 = 1_000;
const MAX_COUNT: u32 = 1_000;
const DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

fn day_ok(value: &str, prefix_allowed: bool) -> bool {
    let digits = value.trim_start_matches(['+', '-']);
    let (number, day) = digits.split_at(digits.len().saturating_sub(2));
    DAYS.contains(&day)
        && (number.is_empty()
            || (prefix_allowed
                && value.len() <= 4
                && number.len() == 1
                && number.chars().all(|c| matches!(c, '1'..='5'))))
}

fn list<T: std::str::FromStr>(value: &str) -> Option<Vec<T>> {
    value
        .split(',')
        .map(|item| item.trim().parse().ok())
        .collect()
}

/// Describes `rule` (an RRULE value) of an event starting at `start`; `None`
/// when its frequency is not daily, weekly, monthly or yearly or a part is
/// malformed.
pub fn describe(rule: &str, start: &EventTime) -> Option<CalendarRecurrenceV1> {
    let mut out = CalendarRecurrenceV1 {
        freq: String::new(),
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        custom: false,
    };
    for part in rule.split(';').filter(|part| !part.trim().is_empty()) {
        let (key, value) = part.split_once('=')?;
        let value = value.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                out.freq = match value.to_ascii_uppercase().as_str() {
                    "DAILY" => "daily",
                    "WEEKLY" => "weekly",
                    "MONTHLY" => "monthly",
                    "YEARLY" => "yearly",
                    _ => return None,
                }
                .to_owned();
            }
            "INTERVAL" => out.interval = value.parse().ok().filter(|n| *n >= 1)?,
            "COUNT" => out.count = Some(value.parse().ok()?),
            "UNTIL" => out.until = Some(until_text(value, start)?),
            "BYDAY" => {
                out.by_day = value
                    .split(',')
                    .map(|day| day.trim().to_ascii_uppercase())
                    .collect();
                if !out.by_day.iter().all(|day| day_ok(day, true)) {
                    return None;
                }
            }
            "BYMONTHDAY" => out.by_month_day = list(value)?,
            "BYMONTH" => out.by_month = list(value)?,
            "WKST" if value.eq_ignore_ascii_case("MO") => {}
            _ => out.custom = true,
        }
    }
    (!out.freq.is_empty()).then_some(out)
}

fn until_text(value: &str, start: &EventTime) -> Option<String> {
    if value.len() == 8 {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some(date.format("%Y-%m-%d").to_string());
    }
    if let Some(stripped) = value.strip_suffix(['Z', 'z']) {
        let instant = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some(if start.is_date() {
            instant.format("%Y-%m-%d").to_string()
        } else {
            instant.format("%Y-%m-%dT%H:%M:%SZ").to_string()
        });
    }
    let local = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Some(local.format("%Y-%m-%dT%H:%M:%S").to_string())
}

/// The RRULE UNTIL value for an end given as `YYYY-MM-DD` (the whole day
/// counts), `YYYY-MM-DDTHH:MM:SSZ` or `YYYY-MM-DDTHH:MM:SS` (wall clock), in
/// the form RFC 5545 requires for the event's start.
fn until_value(text: &str, start: &EventTime) -> Result<String> {
    let text = text.trim();
    let end_of_day = NaiveTime::from_hms_opt(23, 59, 59).ok_or(CalendarError::InvalidInput)?;
    let (local, is_utc) = if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        (date.and_time(end_of_day), false)
    } else {
        let is_utc = text.ends_with(['Z', 'z']);
        let local =
            NaiveDateTime::parse_from_str(text.trim_end_matches(['Z', 'z']), "%Y-%m-%dT%H:%M:%S")
                .map_err(|_| CalendarError::InvalidInput)?;
        (local, is_utc)
    };
    Ok(match &start.kind {
        TimeKind::Date => local.format("%Y%m%d").to_string(),
        TimeKind::Floating => local.format("%Y%m%dT%H%M%S").to_string(),
        TimeKind::Utc => local.format("%Y%m%dT%H%M%SZ").to_string(),
        TimeKind::Zoned(zone) => {
            let instant = if is_utc {
                Utc.from_utc_datetime(&local)
            } else {
                resolve_local(*zone, local)
            };
            instant.format("%Y%m%dT%H%M%SZ").to_string()
        }
    })
}

/// Builds the RRULE value for `recurrence` on an event starting at `start`.
/// Refuses unknown frequencies, a zero or oversized interval or count, both a
/// count and an end, and malformed day, month-day or month lists.
pub fn build(recurrence: &CalendarRecurrenceV1, start: &EventTime) -> Result<String> {
    let freq = match recurrence.freq.trim().to_ascii_lowercase().as_str() {
        "daily" => "DAILY",
        "weekly" => "WEEKLY",
        "monthly" => "MONTHLY",
        "yearly" => "YEARLY",
        _ => return Err(CalendarError::InvalidInput),
    };
    if !(1..=MAX_INTERVAL).contains(&recurrence.interval)
        || recurrence
            .count
            .is_some_and(|n| !(1..=MAX_COUNT).contains(&n))
        || (recurrence.count.is_some() && recurrence.until.is_some())
        || recurrence.by_day.len() > 7
        || recurrence.by_month_day.len() > 31
        || recurrence.by_month.len() > 12
    {
        return Err(CalendarError::InvalidInput);
    }
    let prefix_allowed = matches!(freq, "MONTHLY" | "YEARLY");
    let mut parts = vec![format!("FREQ={freq}")];
    if recurrence.interval > 1 {
        parts.push(format!("INTERVAL={}", recurrence.interval));
    }
    if !recurrence.by_day.is_empty() {
        let days: Vec<String> = recurrence
            .by_day
            .iter()
            .map(|day| day.trim().to_ascii_uppercase())
            .collect();
        if !days.iter().all(|day| day_ok(day, prefix_allowed)) {
            return Err(CalendarError::InvalidInput);
        }
        parts.push(format!("BYDAY={}", days.join(",")));
    }
    if !recurrence.by_month_day.is_empty() {
        if recurrence
            .by_month_day
            .iter()
            .any(|day| *day == 0 || !(-31..=31).contains(day))
        {
            return Err(CalendarError::InvalidInput);
        }
        parts.push(format!(
            "BYMONTHDAY={}",
            recurrence
                .by_month_day
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !recurrence.by_month.is_empty() {
        if recurrence
            .by_month
            .iter()
            .any(|month| !(1..=12).contains(month))
        {
            return Err(CalendarError::InvalidInput);
        }
        parts.push(format!(
            "BYMONTH={}",
            recurrence
                .by_month
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if let Some(count) = recurrence.count {
        parts.push(format!("COUNT={count}"));
    }
    if let Some(until) = &recurrence.until {
        parts.push(format!("UNTIL={}", until_value(until, start)?));
    }
    Ok(parts.join(";"))
}
