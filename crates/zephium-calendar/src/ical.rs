//! iCalendar (RFC 5545) reading and writing for VEVENTs. Input is bounded and
//! malformed events are skipped rather than failing the whole calendar.

use crate::error::{CalendarError, Result};
use crate::model::{Alarm, Event, EventStatus, EventTime, PartStat, Person, TimeKind};
use crate::tz;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};

/// Largest iCalendar document accepted (a subscription feed).
pub const MAX_ICS_BYTES: usize = 8 * 1024 * 1024;
/// Most VEVENT components kept per document.
pub const MAX_EVENTS: usize = 20_000;
/// Most content lines read per document.
pub const MAX_LINES: usize = 400_000;
/// Most attendees kept per event.
pub const MAX_ATTENDEES: usize = 500;
/// Most alarms, RDATEs and EXDATEs kept per event.
pub const MAX_LIST: usize = 1_000;

const MAX_SUMMARY: usize = 1_000;
const MAX_TEXT: usize = 100_000;
const MAX_SHORT: usize = 2_000;
const MAX_RRULE: usize = 500;

#[derive(Debug)]
struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Line {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn unfold(text: &str) -> Result<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if raw.is_empty() {
            continue;
        }
        if lines.len() >= MAX_LINES {
            return Err(CalendarError::Limit);
        }
        lines.push(raw.to_owned());
    }
    Ok(lines)
}

fn split_outside_quotes(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == separator && !quoted {
            parts.push(&text[start..index]);
            start = index + ch.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

fn parse_line(raw: &str) -> Option<Line> {
    let mut quoted = false;
    let mut colon = None;
    for (index, ch) in raw.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == ':' && !quoted {
            colon = Some(index);
            break;
        }
    }
    let colon = colon?;
    let (head, value) = (&raw[..colon], &raw[colon + 1..]);
    let mut pieces = split_outside_quotes(head, ';').into_iter();
    let name = pieces.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = pieces
        .filter_map(|piece| {
            let (key, value) = piece.split_once('=')?;
            Some((
                key.trim().to_ascii_uppercase(),
                value.trim().trim_matches('"').to_owned(),
            ))
        })
        .collect();
    Some(Line {
        name,
        params,
        value: value.to_owned(),
    })
}

fn unescape(value: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if out.len() >= limit {
            break;
        }
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y%m%d").ok()
}

fn parse_datetime(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value.trim().trim_end_matches(['Z', 'z']), "%Y%m%dT%H%M%S").ok()
}

fn time_from(value: &str, params: &dyn Fn(&str) -> Option<String>) -> Option<EventTime> {
    let value = value.trim();
    let is_date = params("VALUE").is_some_and(|kind| kind.eq_ignore_ascii_case("DATE"))
        || (value.len() == 8 && !value.contains('T'));
    if is_date {
        return parse_date(value).map(EventTime::date);
    }
    let local = parse_datetime(value)?;
    if value.ends_with(['Z', 'z']) {
        return Some(EventTime {
            kind: TimeKind::Utc,
            local,
        });
    }
    let kind = params("TZID")
        .and_then(|tzid| tz::resolve(&tzid))
        .map_or(TimeKind::Floating, TimeKind::Zoned);
    Some(EventTime { kind, local })
}

fn parse_time(line: &Line) -> Option<EventTime> {
    time_from(&line.value, &|name| line.param(name).map(str::to_owned))
}

fn parse_time_list(line: &Line) -> Vec<EventTime> {
    line.value
        .split(',')
        .take(MAX_LIST)
        .filter_map(|piece| {
            // RDATE;VALUE=PERIOD: keep the start.
            let piece = piece.split('/').next().unwrap_or(piece);
            time_from(piece, &|name| line.param(name).map(str::to_owned))
        })
        .collect()
}

/// Parses an RFC 5545 DURATION into seconds.
fn parse_duration(value: &str) -> Option<i64> {
    let value = value.trim();
    let (sign, rest) = match value.as_bytes().first()? {
        b'-' => (-1, &value[1..]),
        b'+' => (1, &value[1..]),
        _ => (1, value),
    };
    let rest = rest.strip_prefix('P')?;
    let mut total: i64 = 0;
    let mut number = String::new();
    let mut in_time = false;
    for ch in rest.chars() {
        match ch {
            'T' => in_time = true,
            '0'..='9' => {
                if number.len() > 9 {
                    return None;
                }
                number.push(ch);
            }
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                total += n * match (unit, in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    if !number.is_empty() {
        return None;
    }
    Some(sign * total)
}

fn parse_person(line: &Line) -> Option<Person> {
    let value = line.value.trim();
    let email = value
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("mailto:"))
        .map_or(value, |_| &value[7..]);
    if email.is_empty() || email.len() > 320 {
        return None;
    }
    let partstat = match line
        .param("PARTSTAT")
        .map(str::to_ascii_uppercase)
        .as_deref()
    {
        Some("ACCEPTED") => PartStat::Accepted,
        Some("DECLINED") => PartStat::Declined,
        Some("TENTATIVE") => PartStat::Tentative,
        Some("DELEGATED") => PartStat::Delegated,
        _ => PartStat::NeedsAction,
    };
    Some(Person {
        email: email.to_owned(),
        name: line
            .param("CN")
            .filter(|name| !name.is_empty())
            .map(|name| name.chars().take(200).collect()),
        partstat,
    })
}

fn parse_alarm(lines: &[Line], start: &EventTime) -> Option<Alarm> {
    let trigger = lines.iter().find(|line| line.name == "TRIGGER")?;
    let action = lines.iter().find(|line| line.name == "ACTION").map_or_else(
        || "DISPLAY".to_owned(),
        |line| line.value.to_ascii_uppercase(),
    );
    let absolute = trigger
        .param("VALUE")
        .is_some_and(|kind| kind.eq_ignore_ascii_case("DATE-TIME"));
    if absolute {
        let moment = parse_time(&Line {
            name: String::new(),
            params: Vec::new(),
            value: trigger.value.clone(),
        })?;
        let seconds = (start.to_utc() - moment.to_utc()).num_seconds();
        return Some(Alarm {
            minutes_before: seconds / 60,
            from_end: false,
            action,
        });
    }
    let seconds = parse_duration(&trigger.value)?;
    Some(Alarm {
        minutes_before: -seconds / 60,
        from_end: trigger
            .param("RELATED")
            .is_some_and(|related| related.eq_ignore_ascii_case("END")),
        action,
    })
}

pub(crate) struct RawEvent {
    lines: Vec<Line>,
    alarms: Vec<Vec<Line>>,
}

fn build_event(raw: &RawEvent) -> Option<Event> {
    let first = |name: &str| raw.lines.iter().find(|line| line.name == name);
    let uid = first("UID")?.value.trim().to_owned();
    if uid.is_empty() || uid.len() > 512 {
        return None;
    }
    let start = parse_time(first("DTSTART")?)?;
    let end = if let Some(line) = first("DTEND") {
        parse_time(line)?
    } else if let Some(seconds) = first("DURATION").and_then(|line| parse_duration(&line.value)) {
        start.plus(Duration::seconds(seconds))
    } else if start.is_date() {
        start.plus(Duration::days(1))
    } else {
        start.clone()
    };
    // An end before the start is malformed; collapse it to the start.
    let end = if end.to_utc() < start.to_utc() {
        start.clone()
    } else {
        end
    };
    let text = |name: &str, limit: usize| {
        first(name)
            .map(|line| unescape(&line.value, limit))
            .unwrap_or_default()
    };
    let status = match first("STATUS")
        .map(|line| line.value.trim().to_ascii_uppercase())
        .as_deref()
    {
        Some("CONFIRMED") => EventStatus::Confirmed,
        Some("TENTATIVE") => EventStatus::Tentative,
        Some("CANCELLED") => EventStatus::Cancelled,
        _ => EventStatus::None,
    };
    let rrule = first("RRULE")
        .map(|line| line.value.trim().to_owned())
        .filter(|rule| !rule.is_empty() && rule.len() <= MAX_RRULE);
    let mut rdates = Vec::new();
    let mut exdates = Vec::new();
    for line in &raw.lines {
        match line.name.as_str() {
            "RDATE" => rdates.extend(parse_time_list(line)),
            "EXDATE" => exdates.extend(parse_time_list(line)),
            _ => {}
        }
    }
    rdates.truncate(MAX_LIST);
    exdates.truncate(MAX_LIST);
    let url = first("URL")
        .map(|line| line.value.trim().to_owned())
        .filter(|url| !url.is_empty() && url.len() <= MAX_SHORT);
    Some(Event {
        uid,
        summary: text("SUMMARY", MAX_SUMMARY),
        description: text("DESCRIPTION", MAX_TEXT),
        location: text("LOCATION", MAX_SHORT),
        status,
        organizer: first("ORGANIZER").and_then(parse_person),
        attendees: raw
            .lines
            .iter()
            .filter(|line| line.name == "ATTENDEE")
            .filter_map(parse_person)
            .take(MAX_ATTENDEES)
            .collect(),
        alarms: raw
            .alarms
            .iter()
            .filter_map(|lines| parse_alarm(lines, &start))
            .take(MAX_LIST)
            .collect(),
        url,
        sequence: first("SEQUENCE")
            .and_then(|line| line.value.trim().parse().ok())
            .unwrap_or(0),
        recurrence_id: first("RECURRENCE-ID").and_then(parse_time),
        rrule,
        rdates,
        exdates,
        overrides: Vec::new(),
        start,
        end,
    })
}

/// Parses every VEVENT in `text`; overrides (RECURRENCE-ID) are attached to
/// their master by UID. Events that cannot be understood are skipped.
pub fn parse_calendar(text: &str) -> Result<Vec<Event>> {
    let (_, raws) = read_document(text)?;
    let mut masters: Vec<Event> = Vec::new();
    let mut overrides: Vec<Event> = Vec::new();
    for raw in &raws {
        if let Some(event) = build_event(raw) {
            if event.recurrence_id.is_some() {
                overrides.push(event);
            } else {
                masters.push(event);
            }
        }
    }
    for item in overrides {
        match masters.iter_mut().find(|master| master.uid == item.uid) {
            Some(master) if master.overrides.len() < MAX_LIST => master.overrides.push(item),
            Some(_) => {}
            // No master in this document: keep it as a plain event.
            None => masters.push(item),
        }
    }
    Ok(masters)
}

/// Splits a document into its VCALENDAR `METHOD` and raw VEVENTs.
fn read_document(text: &str) -> Result<(Option<String>, Vec<RawEvent>)> {
    if text.len() > MAX_ICS_BYTES {
        return Err(CalendarError::TooLarge);
    }
    let lines = unfold(text)?;
    if !lines
        .iter()
        .any(|line| line.trim().eq_ignore_ascii_case("BEGIN:VCALENDAR"))
    {
        return Err(CalendarError::Parse);
    }
    let mut stack: Vec<String> = Vec::new();
    let mut method: Option<String> = None;
    let mut raws: Vec<RawEvent> = Vec::new();
    let mut current: Option<RawEvent> = None;
    let mut alarm: Option<Vec<Line>> = None;
    for raw in &lines {
        let Some(line) = parse_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                let kind = line.value.trim().to_ascii_uppercase();
                if stack.len() >= 8 {
                    return Err(CalendarError::Parse);
                }
                if kind == "VEVENT" && stack.last().is_some_and(|top| top == "VCALENDAR") {
                    if raws.len() >= MAX_EVENTS {
                        return Err(CalendarError::Limit);
                    }
                    current = Some(RawEvent {
                        lines: Vec::new(),
                        alarms: Vec::new(),
                    });
                } else if kind == "VALARM" && current.is_some() && alarm.is_none() {
                    alarm = Some(Vec::new());
                }
                stack.push(kind);
            }
            "END" => {
                let kind = line.value.trim().to_ascii_uppercase();
                if stack.pop().as_deref() != Some(kind.as_str()) {
                    return Err(CalendarError::Parse);
                }
                if kind == "VALARM" {
                    if let (Some(lines), Some(event)) = (alarm.take(), current.as_mut()) {
                        if event.alarms.len() < MAX_LIST {
                            event.alarms.push(lines);
                        }
                    }
                } else if kind == "VEVENT" {
                    if let Some(event) = current.take() {
                        raws.push(event);
                    }
                }
            }
            "METHOD" if method.is_none() && stack.len() == 1 && stack[0] == "VCALENDAR" => {
                method = Some(line.value.trim().to_ascii_uppercase());
            }
            _ => {
                if let Some(lines) = alarm.as_mut() {
                    lines.push(line);
                } else if let Some(event) = current.as_mut() {
                    // Only direct VEVENT properties; nested unknown components
                    // never reach here because VALARM is the only one tracked.
                    if stack.last().is_some_and(|top| top == "VEVENT") {
                        event.lines.push(line);
                    }
                }
            }
        }
    }
    Ok((method, raws))
}

/// One VEVENT of an iTIP message with the attributes the event model drops.
pub(crate) struct ItipComponent {
    /// The event; a missing DTSTART (allowed in CANCEL and REPLY) is read as
    /// the epoch, see `has_start`.
    pub(crate) event: Event,
    /// `false` when the component had no DTSTART.
    pub(crate) has_start: bool,
    /// DTSTAMP.
    pub(crate) dtstamp: Option<DateTime<Utc>>,
    /// ROLE and RSVP of each attendee, parallel to `event.attendees`.
    pub(crate) attendee_meta: Vec<(AttendeeRole, bool)>,
}

/// ROLE of an attendee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttendeeRole {
    /// `CHAIR`.
    Chair,
    /// `REQ-PARTICIPANT` (the default).
    Required,
    /// `OPT-PARTICIPANT`.
    Optional,
    /// `NON-PARTICIPANT`.
    NonParticipant,
}

impl AttendeeRole {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chair => "chair",
            Self::Required => "required",
            Self::Optional => "optional",
            Self::NonParticipant => "non_participant",
        }
    }
}

/// Reads an iTIP message: its METHOD and every VEVENT, not grouped.
pub(crate) fn parse_itip(text: &str) -> Result<(Option<String>, Vec<ItipComponent>)> {
    let (method, raws) = read_document(text)?;
    let mut components = Vec::new();
    for mut raw in raws {
        let has_start = raw.lines.iter().any(|line| line.name == "DTSTART");
        if !has_start {
            raw.lines.push(Line {
                name: "DTSTART".to_owned(),
                params: Vec::new(),
                value: "19700101T000000Z".to_owned(),
            });
        }
        let Some(event) = build_event(&raw) else {
            continue;
        };
        let dtstamp = raw
            .lines
            .iter()
            .find(|line| line.name == "DTSTAMP")
            .and_then(parse_time)
            .map(|time| time.to_utc());
        let attendee_meta = raw
            .lines
            .iter()
            .filter(|line| line.name == "ATTENDEE")
            .filter(|line| parse_person(line).is_some())
            .take(MAX_ATTENDEES)
            .map(|line| {
                let role = match line.param("ROLE").map(str::to_ascii_uppercase).as_deref() {
                    Some("CHAIR") => AttendeeRole::Chair,
                    Some("OPT-PARTICIPANT") => AttendeeRole::Optional,
                    Some("NON-PARTICIPANT") => AttendeeRole::NonParticipant,
                    _ => AttendeeRole::Required,
                };
                let rsvp = line
                    .param("RSVP")
                    .is_some_and(|value| value.eq_ignore_ascii_case("TRUE"));
                (role, rsvp)
            })
            .collect();
        components.push(ItipComponent {
            event,
            has_start,
            dtstamp,
            attendee_meta,
        });
    }
    Ok((method, components))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

pub(crate) fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for ch in line.chars() {
        let size = ch.len_utf8();
        if width + size > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += size;
    }
    out.push_str("\r\n");
}

pub(crate) fn time_property(name: &str, time: &EventTime) -> String {
    match &time.kind {
        TimeKind::Date => format!("{name};VALUE=DATE:{}", time.local.format("%Y%m%d")),
        TimeKind::Utc => format!("{name}:{}Z", time.local.format("%Y%m%dT%H%M%S")),
        TimeKind::Zoned(zone) => format!(
            "{name};TZID={}:{}",
            zone.name(),
            time.local.format("%Y%m%dT%H%M%S")
        ),
        TimeKind::Floating => format!("{name}:{}", time.local.format("%Y%m%dT%H%M%S")),
    }
}

fn param_value(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '"' | '\r' | '\n' | ';' | ':' | ','))
        .collect()
}

fn write_event(event: &Event, stamp: DateTime<Utc>, out: &mut String) {
    fold("BEGIN:VEVENT", out);
    fold(&format!("UID:{}", escape(&event.uid)), out);
    fold(&format!("DTSTAMP:{}Z", stamp.format("%Y%m%dT%H%M%S")), out);
    if let Some(id) = &event.recurrence_id {
        fold(&time_property("RECURRENCE-ID", id), out);
    }
    fold(&time_property("DTSTART", &event.start), out);
    fold(&time_property("DTEND", &event.end), out);
    fold(&format!("SUMMARY:{}", escape(&event.summary)), out);
    if !event.description.is_empty() {
        fold(&format!("DESCRIPTION:{}", escape(&event.description)), out);
    }
    if !event.location.is_empty() {
        fold(&format!("LOCATION:{}", escape(&event.location)), out);
    }
    if let Some(url) = &event.url {
        fold(&format!("URL:{}", url.replace(['\r', '\n'], "")), out);
    }
    if event.status != EventStatus::None {
        fold(
            &format!("STATUS:{}", event.status.as_str().to_ascii_uppercase()),
            out,
        );
    }
    fold(&format!("SEQUENCE:{}", event.sequence), out);
    if let Some(rule) = &event.rrule {
        fold(&format!("RRULE:{}", rule.replace(['\r', '\n'], "")), out);
    }
    for date in &event.rdates {
        fold(&time_property("RDATE", date), out);
    }
    for date in &event.exdates {
        fold(&time_property("EXDATE", date), out);
    }
    if let Some(person) = &event.organizer {
        fold(&person_property("ORGANIZER", person, false), out);
    }
    for person in &event.attendees {
        fold(&person_property("ATTENDEE", person, true), out);
    }
    for alarm in &event.alarms {
        fold("BEGIN:VALARM", out);
        fold(&format!("ACTION:{}", param_value(&alarm.action)), out);
        let sign = if alarm.minutes_before >= 0 { "-" } else { "" };
        let related = if alarm.from_end { ";RELATED=END" } else { "" };
        fold(
            &format!("TRIGGER{related}:{sign}PT{}M", alarm.minutes_before.abs()),
            out,
        );
        fold("DESCRIPTION:Reminder", out);
        fold("END:VALARM", out);
    }
    fold("END:VEVENT", out);
}

pub(crate) fn person_property(name: &str, person: &Person, with_status: bool) -> String {
    let mut line = name.to_owned();
    if let Some(cn) = &person.name {
        line.push_str(&format!(";CN=\"{}\"", param_value(cn)));
    }
    if with_status {
        line.push_str(&format!(
            ";PARTSTAT={}",
            person
                .partstat
                .as_str()
                .to_ascii_uppercase()
                .replace('_', "-")
        ));
    }
    line.push_str(&format!(":mailto:{}", param_value(&person.email)));
    line
}

/// Zones used by `event` and its overrides, each with the earliest year it is
/// used in, ordered by name.
fn zones_used(event: &Event) -> Vec<(chrono_tz::Tz, i32)> {
    use chrono::Datelike;
    let mut zones: std::collections::BTreeMap<&'static str, (chrono_tz::Tz, i32)> =
        std::collections::BTreeMap::new();
    let mut visit = |time: &EventTime| {
        if let TimeKind::Zoned(zone) = &time.kind {
            let year = time.local.year();
            zones
                .entry(zone.name())
                .and_modify(|(_, first)| *first = (*first).min(year))
                .or_insert((*zone, year));
        }
    };
    for item in std::iter::once(event).chain(event.overrides.iter()) {
        visit(&item.start);
        visit(&item.end);
        item.rdates.iter().for_each(&mut visit);
        item.exdates.iter().for_each(&mut visit);
        if let Some(id) = &item.recurrence_id {
            visit(id);
        }
    }
    zones.into_values().collect()
}

/// Serializes one event with its overrides as a complete VCALENDAR. Every
/// time zone the event uses is written as `TZID=<IANA name>` and described by
/// a VTIMEZONE component (see [`crate::vtimezone`]).
pub fn serialize_event(event: &Event, stamp: DateTime<Utc>) -> String {
    let mut out = String::new();
    fold("BEGIN:VCALENDAR", &mut out);
    fold("VERSION:2.0", &mut out);
    fold("PRODID:-//Zephium//Calendar//EN", &mut out);
    for (zone, year) in zones_used(event) {
        for line in crate::vtimezone::component(zone, year) {
            fold(&line, &mut out);
        }
    }
    write_event(event, stamp, &mut out);
    for item in &event.overrides {
        write_event(item, stamp, &mut out);
    }
    fold("END:VCALENDAR", &mut out);
    out
}

/// Earliest start and latest end of everything in `events` (instants), for
/// range queries on the cache. The end is `None` for a series that never
/// ends (an RRULE without `UNTIL` or `COUNT`) or whose expansion is cut off
/// by the expansion limit.
pub fn bounds(events: &[Event]) -> Option<(DateTime<Utc>, Option<DateTime<Utc>>)> {
    let mut start: Option<DateTime<Utc>> = None;
    let mut end: Option<DateTime<Utc>> = None;
    let mut open = false;
    for master in events {
        for item in std::iter::once(master).chain(master.overrides.iter()) {
            let (s, e) = (
                item.start.to_utc(),
                item.end.to_utc().max(item.start.to_utc()),
            );
            start = Some(start.map_or(s, |known| known.min(s)));
            end = Some(end.map_or(e, |known| known.max(e)));
            for time in item.rdates.iter().chain(item.recurrence_id.iter()) {
                let t = time.to_utc();
                start = start.map(|known| known.min(t));
                end = end.map(|known| known.max(t + (e - s)));
            }
        }
        if let Some(rule) = &master.rrule {
            let upper = rule.to_ascii_uppercase();
            if !upper.contains("UNTIL=") && !upper.contains("COUNT=") {
                open = true;
            } else {
                let from = master.start.to_utc() - chrono::Duration::days(1);
                let expansion = crate::recur::expand(
                    master,
                    from,
                    from + chrono::Duration::days(366 * 200),
                    crate::recur::MAX_OCCURRENCES_PER_EVENT,
                );
                if expansion.truncated {
                    open = true;
                }
                for occurrence in &expansion.occurrences {
                    end = end.map(|known| known.max(occurrence.end.to_utc()));
                }
            }
        }
    }
    start.map(|s| (s, if open { None } else { end }))
}

/// Calendar-level name (`X-WR-CALNAME`) and colour (`X-APPLE-CALENDAR-COLOR`
/// or `COLOR`) of a feed, read from the lines before the first event.
pub fn calendar_properties(text: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut color = None;
    let mut depth = 0usize;
    for raw in text.lines().take(2_000) {
        let Some(line) = parse_line(raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => {
                depth += 1;
                if depth > 1 {
                    break;
                }
            }
            "X-WR-CALNAME" if name.is_none() => {
                name = Some(unescape(&line.value, 200)).filter(|value| !value.trim().is_empty());
            }
            "X-APPLE-CALENDAR-COLOR" | "COLOR" if color.is_none() => {
                let value = line.value.trim();
                if let Some(hex) = value.strip_prefix('#') {
                    if matches!(hex.len(), 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                        color = Some(format!("#{}", hex[..6].to_ascii_uppercase()));
                    }
                }
            }
            _ => {}
        }
    }
    (name, color)
}
