//! Fork-owned (review R2-B2): writing an event back by patching the stored
//! iCalendar text instead of rewriting it from the reduced model.
//!
//! The model keeps only some properties. A CalDAV resource also carries
//! `CATEGORIES`, `TRANSP`, `CLASS`, `ATTACH`, `X-` properties, attendee
//! parameters (`ROLE`, `RSVP`, `CUTYPE`), `EMAIL` alarms, other components and
//! the server's own `VTIMEZONE`s. Writing the model alone drops all of them.
//! [`patch`] keeps every line of the original and replaces only the
//! properties whose modelled value differs between the original (parsed again)
//! and the event to write. Everything else passes through byte for byte
//! (after unfolding and folding again).

use super::{
    build_event, escape, fold, parse_alarm, parse_line, parse_person, person_property,
    time_property, unfold, write_event, zone_component, zones_in, Line, RawEvent, Zones,
    MAX_ATTENDEES, MAX_ICS_BYTES,
};
use crate::model::{Alarm, Event, EventStatus, EventTime};
use chrono::{DateTime, Utc};

enum Item {
    Prop(String, Line),
    Nested {
        name: String,
        raws: Vec<String>,
        lines: Vec<Line>,
    },
}

struct Comp {
    items: Vec<Item>,
}

enum Piece {
    Raw(String),
    Event(Comp),
}

/// Splits `text` into raw lines outside VEVENTs and VEVENT components that
/// keep their lines. `None` for a document that does not nest properly.
fn read(text: &str) -> Option<Vec<Piece>> {
    if text.len() > MAX_ICS_BYTES {
        return None;
    }
    let lines = unfold(text).ok()?;
    let mut pieces = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut current: Option<Comp> = None;
    let mut nested: Option<(String, Vec<String>, Vec<Line>, usize)> = None;
    for raw in lines {
        let parsed = parse_line(&raw);
        let (name, value) = parsed
            .as_ref()
            .map_or((String::new(), String::new()), |line| {
                (line.name.clone(), line.value.trim().to_ascii_uppercase())
            });
        let name = name.as_str();
        if let Some(comp) = current.as_mut() {
            if let Some((kind, raws, inner, depth)) = nested.as_mut() {
                raws.push(raw.clone());
                if let Some(line) = parsed {
                    if line.name == "BEGIN" {
                        *depth += 1;
                    } else if line.name == "END" {
                        *depth -= 1;
                    }
                    inner.push(line);
                }
                if *depth == 0 {
                    // The BEGIN and END lines are in `inner`; drop them for parsing.
                    let lines = inner
                        .drain(..)
                        .filter(|line| line.name != "BEGIN" && line.name != "END")
                        .collect();
                    comp.items.push(Item::Nested {
                        name: std::mem::take(kind),
                        raws: std::mem::take(raws),
                        lines,
                    });
                    nested = None;
                }
                continue;
            }
            match (name, parsed) {
                ("BEGIN", Some(line)) => {
                    nested = Some((value, vec![raw.clone()], vec![line], 1));
                }
                ("END", _) if value == "VEVENT" => {
                    pieces.push(Piece::Event(current.take()?));
                    stack.pop();
                }
                (_, Some(line)) => comp.items.push(Item::Prop(raw, line)),
                (_, None) => return None,
            }
            continue;
        }
        match name {
            "BEGIN" => {
                if stack.len() >= 8 {
                    return None;
                }
                if value == "VEVENT" && stack.last().is_some_and(|top| top == "VCALENDAR") {
                    current = Some(Comp { items: Vec::new() });
                    stack.push(value);
                    continue;
                }
                stack.push(value);
            }
            "END" => {
                if stack.pop().as_deref() != Some(value.as_str()) {
                    return None;
                }
            }
            _ => {}
        }
        pieces.push(Piece::Raw(raw));
    }
    (current.is_none() && stack.is_empty()).then_some(pieces)
}

fn same_instant(a: &EventTime, b: &EventTime) -> bool {
    a.to_utc() == b.to_utc() && a.is_date() == b.is_date()
}

fn same_email(a: &str, b: &str) -> bool {
    crate::invite::normalize_address(a) == crate::invite::normalize_address(b)
}

/// The VEVENT as the model reads it.
fn parse_comp(comp: &Comp) -> Option<Event> {
    let lines = comp
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Prop(raw, _) => parse_line(raw),
            Item::Nested { .. } => None,
        })
        .collect();
    let alarms = comp
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Nested { name, lines, .. } if name == "VALARM" => Some(
                lines
                    .iter()
                    .map(|line| Line {
                        name: line.name.clone(),
                        params: line.params.clone(),
                        value: line.value.clone(),
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect();
    build_event(&RawEvent { lines, alarms })
}

fn head_and_value(raw: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (index, ch) in raw.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == ':' && !quoted {
            return Some((&raw[..index], &raw[index + 1..]));
        }
    }
    None
}

fn param_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '"' | '\r' | '\n' | ';' | ':' | ','))
        .collect()
}

/// `raw` (an ORGANIZER or ATTENDEE line) with its `CN` and, for attendees,
/// `PARTSTAT` set from `person`; every other parameter stays.
fn patch_person(raw: &str, person: &crate::model::Person, with_status: bool) -> String {
    let Some((head, value)) = head_and_value(raw) else {
        return person_property(
            if with_status { "ATTENDEE" } else { "ORGANIZER" },
            person,
            with_status,
        );
    };
    let mut parts = super::split_outside_quotes(head, ';').into_iter();
    let name = parts.next().unwrap_or("ATTENDEE").to_owned();
    let mut kept: Vec<String> = Vec::new();
    for part in parts {
        let key = part
            .split_once('=')
            .map(|(key, _)| key.trim().to_ascii_uppercase());
        match key.as_deref() {
            Some("CN") => {}
            Some("PARTSTAT") if with_status => {}
            _ => kept.push(part.to_owned()),
        }
    }
    let mut line = name;
    if let Some(cn) = &person.name {
        line.push_str(&format!(";CN=\"{}\"", param_text(cn)));
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
    for part in kept {
        line.push(';');
        line.push_str(&part);
    }
    line.push(':');
    line.push_str(value);
    line
}

fn alarm_lines(alarm: &Alarm) -> Vec<String> {
    let mut out = Vec::new();
    let action: String = alarm
        .action
        .chars()
        .filter(|ch| !matches!(ch, '"' | '\r' | '\n' | ';' | ':' | ','))
        .collect();
    out.push("BEGIN:VALARM".to_owned());
    out.push(format!("ACTION:{action}"));
    let sign = if alarm.minutes_before >= 0 { "-" } else { "" };
    let related = if alarm.from_end { ";RELATED=END" } else { "" };
    out.push(format!(
        "TRIGGER{related}:{sign}PT{}M",
        alarm.minutes_before.abs()
    ));
    out.push("DESCRIPTION:Reminder".to_owned());
    out.push("END:VALARM".to_owned());
    out
}

struct Change {
    names: &'static [&'static str],
    lines: Vec<String>,
    done: bool,
}

fn note(zones: &mut Zones, time: &EventTime) {
    zones_in(zones, time);
}

fn times(name: &str, list: &[EventTime], zones: &mut Zones) -> Vec<String> {
    list.iter()
        .map(|time| {
            note(zones, time);
            time_property(name, time)
        })
        .collect()
}

fn patch_comp(
    comp: &Comp,
    old: &Event,
    new: &Event,
    stamp: DateTime<Utc>,
    zones: &mut Zones,
) -> Vec<String> {
    let mut changes: Vec<Change> = Vec::new();
    let mut change = |names: &'static [&'static str], lines: Vec<String>| {
        changes.push(Change {
            names,
            lines,
            done: false,
        });
    };
    change(
        &["DTSTAMP"],
        vec![format!("DTSTAMP:{}Z", stamp.format("%Y%m%dT%H%M%S"))],
    );
    if new.recurrence_id != old.recurrence_id {
        change(
            &["RECURRENCE-ID"],
            new.recurrence_id
                .iter()
                .map(|id| {
                    note(zones, id);
                    time_property("RECURRENCE-ID", id)
                })
                .collect(),
        );
    }
    if new.start != old.start {
        note(zones, &new.start);
        change(&["DTSTART"], vec![time_property("DTSTART", &new.start)]);
    }
    if new.end != old.end {
        note(zones, &new.end);
        change(
            &["DTEND", "DURATION"],
            vec![time_property("DTEND", &new.end)],
        );
    }
    let text = |name: &str, value: &str| {
        if value.is_empty() {
            Vec::new()
        } else {
            vec![format!("{name}:{}", escape(value))]
        }
    };
    if new.summary != old.summary {
        change(
            &["SUMMARY"],
            vec![format!("SUMMARY:{}", escape(&new.summary))],
        );
    }
    if new.description != old.description {
        change(&["DESCRIPTION"], text("DESCRIPTION", &new.description));
    }
    if new.location != old.location {
        change(&["LOCATION"], text("LOCATION", &new.location));
    }
    if new.url != old.url {
        change(
            &["URL"],
            new.url
                .iter()
                .map(|url| format!("URL:{}", url.replace(['\r', '\n'], "")))
                .collect(),
        );
    }
    if new.status != old.status {
        change(
            &["STATUS"],
            if new.status == EventStatus::None {
                Vec::new()
            } else {
                vec![format!(
                    "STATUS:{}",
                    new.status.as_str().to_ascii_uppercase()
                )]
            },
        );
    }
    if new.sequence != old.sequence {
        change(&["SEQUENCE"], vec![format!("SEQUENCE:{}", new.sequence)]);
    }
    if new.rrule != old.rrule {
        change(
            &["RRULE"],
            new.rrule
                .iter()
                .map(|rule| format!("RRULE:{}", rule.replace(['\r', '\n'], "")))
                .collect(),
        );
    }
    if new.rdates != old.rdates {
        change(&["RDATE"], times("RDATE", &new.rdates, zones));
    }
    if new.exdates != old.exdates {
        change(&["EXDATE"], times("EXDATE", &new.exdates, zones));
    }

    let mut out: Vec<String> = Vec::new();
    let mut attendee_used = vec![false; new.attendees.len()];
    let mut alarm_used = vec![false; new.alarms.len()];
    let mut organizer_seen = false;
    let mut attendees_read = 0_usize;
    for item in &comp.items {
        match item {
            Item::Prop(raw, line) => {
                if let Some(entry) = changes
                    .iter_mut()
                    .find(|entry| entry.names.contains(&line.name.as_str()))
                {
                    if !entry.done {
                        out.extend(entry.lines.iter().cloned());
                        entry.done = true;
                    }
                    continue;
                }
                match line.name.as_str() {
                    "ORGANIZER" => {
                        organizer_seen = true;
                        let parsed = parse_person(line);
                        match (&new.organizer, parsed) {
                            _ if new.organizer == old.organizer => out.push(raw.clone()),
                            (None, _) => {}
                            (Some(person), Some(existing))
                                if same_email(&person.email, &existing.email) =>
                            {
                                out.push(patch_person(raw, person, false));
                            }
                            (Some(person), _) => {
                                out.push(person_property("ORGANIZER", person, false))
                            }
                        }
                    }
                    "ATTENDEE" => match parse_person(line) {
                        Some(existing) if attendees_read < MAX_ATTENDEES => {
                            attendees_read += 1;
                            let found =
                                new.attendees
                                    .iter()
                                    .enumerate()
                                    .position(|(index, person)| {
                                        !attendee_used[index]
                                            && same_email(&person.email, &existing.email)
                                    });
                            if let Some(index) = found {
                                attendee_used[index] = true;
                                if new.attendees[index] == existing {
                                    out.push(raw.clone());
                                } else {
                                    out.push(patch_person(raw, &new.attendees[index], true));
                                }
                            }
                        }
                        _ => out.push(raw.clone()),
                    },
                    _ => out.push(raw.clone()),
                }
            }
            Item::Nested { name, raws, lines } => {
                if name == "VALARM" {
                    if let Some(alarm) = parse_alarm(lines, &old.start) {
                        let found = new
                            .alarms
                            .iter()
                            .enumerate()
                            .position(|(index, other)| !alarm_used[index] && *other == alarm);
                        if let Some(index) = found {
                            alarm_used[index] = true;
                            out.extend(raws.iter().cloned());
                        }
                        // An alarm the model no longer has was removed.
                        continue;
                    }
                }
                out.extend(raws.iter().cloned());
            }
        }
    }
    for entry in changes.iter().filter(|entry| !entry.done) {
        out.extend(entry.lines.iter().cloned());
    }
    if !organizer_seen {
        if let Some(person) = new
            .organizer
            .as_ref()
            .filter(|_| new.organizer != old.organizer)
        {
            out.push(person_property("ORGANIZER", person, false));
        }
    }
    for (index, person) in new.attendees.iter().enumerate() {
        if !attendee_used[index] {
            out.push(person_property("ATTENDEE", person, true));
        }
    }
    for (index, alarm) in new.alarms.iter().enumerate() {
        if !alarm_used[index] {
            out.extend(alarm_lines(alarm));
        }
    }
    out
}

/// Writes `event` (a master with its overrides) over `original`, the stored
/// resource text: every line the model does not own stays, the changed
/// properties are replaced, overrides that are gone are removed and new ones
/// added. With `itip` the text is an iTIP message: `METHOD` and events of
/// other UIDs are dropped. `None` when `original` cannot be read or holds no
/// master of `event.uid`; the caller then writes the model alone.
pub(super) fn patch(
    original: &str,
    event: &Event,
    stamp: DateTime<Utc>,
    itip: bool,
) -> Option<String> {
    let pieces = read(original)?;
    let mut zones = Zones::new();
    let mut defined: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut override_used = vec![false; event.overrides.len()];
    let mut master_done = false;
    let mut first_event_at: Option<usize> = None;
    for piece in &pieces {
        match piece {
            Piece::Raw(raw) => {
                if raw
                    .get(..5)
                    .is_some_and(|head| head.eq_ignore_ascii_case("TZID:"))
                {
                    defined.push(raw[5..].trim().to_owned());
                }
                if itip
                    && raw
                        .get(..7)
                        .is_some_and(|head| head.eq_ignore_ascii_case("METHOD:"))
                {
                    continue;
                }
                lines.push(raw.clone());
            }
            Piece::Event(comp) => {
                first_event_at.get_or_insert(lines.len());
                let Some(old) = parse_comp(comp) else {
                    emit_raw(comp, &mut lines);
                    continue;
                };
                let target: Option<&Event> = if old.uid != event.uid {
                    if itip {
                        continue;
                    }
                    emit_raw(comp, &mut lines);
                    continue;
                } else if old.recurrence_id.is_none() {
                    (!master_done).then_some(event)
                } else {
                    let found = event
                        .overrides
                        .iter()
                        .enumerate()
                        .position(|(index, item)| {
                            !override_used[index]
                                && item
                                    .recurrence_id
                                    .as_ref()
                                    .zip(old.recurrence_id.as_ref())
                                    .is_some_and(|(a, b)| same_instant(a, b))
                        });
                    found.map(|index| {
                        override_used[index] = true;
                        &event.overrides[index]
                    })
                };
                let Some(new) = target else {
                    // An override that is gone (or a second master): not written.
                    continue;
                };
                if old.recurrence_id.is_none() {
                    master_done = true;
                }
                lines.push("BEGIN:VEVENT".to_owned());
                lines.extend(patch_comp(comp, &old, new, stamp, &mut zones));
                lines.push("END:VEVENT".to_owned());
            }
        }
    }
    if !master_done {
        return None;
    }
    // Overrides the original did not have.
    let mut added: Vec<String> = Vec::new();
    for (index, item) in event.overrides.iter().enumerate() {
        if !override_used[index] {
            let mut text = String::new();
            write_event(item, stamp, &mut text);
            added.extend(text.lines().map(str::to_owned));
            zones_for_event(item, &mut zones);
        }
    }
    let end = lines
        .iter()
        .rposition(|line| line.trim().eq_ignore_ascii_case("END:VCALENDAR"))?;
    for (offset, line) in added.into_iter().enumerate() {
        lines.insert(end + offset, line);
    }
    // Time zones the written properties name that the document does not define.
    let mut zone_lines: Vec<String> = Vec::new();
    for (zone, first, last) in zones.into_values() {
        if !defined
            .iter()
            .any(|name| name.eq_ignore_ascii_case(zone.name()))
        {
            zone_lines.extend(zone_component(zone, first, last, stamp));
        }
    }
    let at = first_event_at.unwrap_or(end).min(lines.len());
    for (offset, line) in zone_lines.into_iter().enumerate() {
        lines.insert(at + offset, line);
    }
    let mut out = String::new();
    for line in &lines {
        fold(line, &mut out);
    }
    Some(out)
}

fn emit_raw(comp: &Comp, lines: &mut Vec<String>) {
    lines.push("BEGIN:VEVENT".to_owned());
    for item in &comp.items {
        match item {
            Item::Prop(raw, _) => lines.push(raw.clone()),
            Item::Nested { raws, .. } => lines.extend(raws.iter().cloned()),
        }
    }
    lines.push("END:VEVENT".to_owned());
}

fn zones_for_event(event: &Event, zones: &mut Zones) {
    for time in [&event.start, &event.end]
        .into_iter()
        .chain(event.rdates.iter())
        .chain(event.exdates.iter())
        .chain(event.recurrence_id.iter())
    {
        zones_in(zones, time);
    }
}
