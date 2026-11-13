use super::utc;
use crate::error::CalendarError;
use crate::ical::{
    calendar_properties, parse_calendar, serialize_event, MAX_EVENTS, MAX_ICS_BYTES,
};
use crate::model::{EventStatus, PartStat, TimeKind};
use chrono::NaiveDate;

fn wrap(body: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n{body}END:VCALENDAR\r\n")
}

#[test]
fn zoned_event_keeps_its_zone_and_instant() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:a@x\r\nSUMMARY:Standup\r\n\
         DTSTART;TZID=Europe/Berlin:20260105T090000\r\nDTEND;TZID=Europe/Berlin:20260105T093000\r\n\
         END:VEVENT\r\n",
    );
    let events = parse_calendar(&ics).expect("parse");
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert!(matches!(event.start.kind, TimeKind::Zoned(tz) if tz.name() == "Europe/Berlin"));
    assert_eq!(event.start.to_utc(), utc("2026-01-05T08:00:00Z"));
    assert_eq!(event.end.to_utc(), utc("2026-01-05T08:30:00Z"));
}

#[test]
fn windows_zone_names_are_mapped() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:w@x\r\nDTSTART;TZID=W. Europe Standard Time:20260705T100000\r\n\
         DURATION:PT1H\r\nEND:VEVENT\r\n",
    );
    let event = &parse_calendar(&ics).expect("parse")[0];
    // Summer time: UTC+2.
    assert_eq!(event.start.to_utc(), utc("2026-07-05T08:00:00Z"));
    assert_eq!(event.end.to_utc(), utc("2026-07-05T09:00:00Z"));
}

#[test]
fn unknown_zone_is_floating_not_a_failure() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:f@x\r\nDTSTART;TZID=Mars/Olympus:20260105T090000\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:g@x\r\nDTSTART:20260105T090000\r\nEND:VEVENT\r\n",
    );
    let events = parse_calendar(&ics).expect("parse");
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.start.kind == TimeKind::Floating));
}

#[test]
fn utc_instant_and_date_forms() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:u@x\r\nDTSTART:20260105T080000Z\r\nDTEND:20260105T090000Z\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:d@x\r\nDTSTART;VALUE=DATE:20260110\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:e@x\r\nDTSTART;VALUE=DATE:20260110\r\nDTEND;VALUE=DATE:20260113\r\nEND:VEVENT\r\n",
    );
    let events = parse_calendar(&ics).expect("parse");
    assert_eq!(events[0].start.kind, TimeKind::Utc);
    assert_eq!(events[0].start.to_utc(), utc("2026-01-05T08:00:00Z"));
    // All-day without DTEND lasts one day; DTEND is exclusive.
    assert!(events[1].start.is_date());
    assert_eq!(
        events[1].end.local.date(),
        NaiveDate::from_ymd_opt(2026, 1, 11).expect("date")
    );
    assert_eq!(
        events[2].end.local.date(),
        NaiveDate::from_ymd_opt(2026, 1, 13).expect("date")
    );
}

#[test]
fn people_alarms_status_and_url() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:p@x\r\nDTSTART:20260105T080000Z\r\nDTEND:20260105T090000Z\r\n\
         STATUS:TENTATIVE\r\nURL:https://meet.example.com/abc\r\n\
         ORGANIZER;CN=\"Boss, The\":mailto:boss@example.com\r\n\
         ATTENDEE;CN=Ann;PARTSTAT=ACCEPTED:mailto:ann@example.com\r\n\
         ATTENDEE;PARTSTAT=DECLINED:MAILTO:bob@example.com\r\n\
         ATTENDEE:mailto:cy@example.com\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER;RELATED=END:PT5M\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nTRIGGER;VALUE=DATE-TIME:20260105T070000Z\r\nEND:VALARM\r\n\
         END:VEVENT\r\n",
    );
    let event = &parse_calendar(&ics).expect("parse")[0];
    assert_eq!(event.status, EventStatus::Tentative);
    assert_eq!(event.url.as_deref(), Some("https://meet.example.com/abc"));
    let organizer = event.organizer.as_ref().expect("organizer");
    assert_eq!(organizer.email, "boss@example.com");
    assert_eq!(organizer.name.as_deref(), Some("Boss, The"));
    let states: Vec<PartStat> = event.attendees.iter().map(|a| a.partstat).collect();
    assert_eq!(
        states,
        [
            PartStat::Accepted,
            PartStat::Declined,
            PartStat::NeedsAction
        ]
    );
    assert_eq!(event.attendees[1].email, "bob@example.com");
    assert_eq!(event.alarms.len(), 3);
    assert_eq!(event.alarms[0].minutes_before, 15);
    assert!(!event.alarms[0].from_end);
    assert_eq!(event.alarms[1].minutes_before, -5);
    assert!(event.alarms[1].from_end);
    assert_eq!(event.alarms[2].minutes_before, 60);
}

#[test]
fn folded_lines_and_escapes_are_read() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:t@x\r\nDTSTART:20260105T080000Z\r\n\
         SUMMARY:Plan\\, review\\; ship\r\n\
         DESCRIPTION:line one\\nline two \r\n that continues\r\n\
         LOCATION:Room\\\\5\r\nEND:VEVENT\r\n",
    );
    let event = &parse_calendar(&ics).expect("parse")[0];
    assert_eq!(event.summary, "Plan, review; ship");
    assert_eq!(event.description, "line one\nline two that continues");
    assert_eq!(event.location, "Room\\5");
}

#[test]
fn recurrence_data_is_collected_with_overrides_attached() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:r@x\r\nDTSTART;TZID=Europe/Berlin:20260105T090000\r\n\
         DTEND;TZID=Europe/Berlin:20260105T100000\r\nRRULE:FREQ=WEEKLY;COUNT=4\r\n\
         EXDATE;TZID=Europe/Berlin:20260112T090000,20260119T090000\r\n\
         RDATE;TZID=Europe/Berlin:20260106T120000\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:r@x\r\nRECURRENCE-ID;TZID=Europe/Berlin:20260126T090000\r\n\
         DTSTART;TZID=Europe/Berlin:20260126T110000\r\nDTEND;TZID=Europe/Berlin:20260126T120000\r\n\
         SUMMARY:Moved\r\nEND:VEVENT\r\n",
    );
    let events = parse_calendar(&ics).expect("parse");
    assert_eq!(events.len(), 1);
    let master = &events[0];
    assert_eq!(master.rrule.as_deref(), Some("FREQ=WEEKLY;COUNT=4"));
    assert_eq!(master.exdates.len(), 2);
    assert_eq!(master.rdates.len(), 1);
    assert_eq!(master.overrides.len(), 1);
    assert!(master.overrides[0].recurrence_id.is_some());
}

#[test]
fn malformed_input_is_refused_or_skipped() {
    assert_eq!(parse_calendar("hello"), Err(CalendarError::Parse));
    let big = format!("BEGIN:VCALENDAR\r\n{}", "X".repeat(MAX_ICS_BYTES));
    assert_eq!(parse_calendar(&big), Err(CalendarError::TooLarge));
    let ics = wrap(
        "BEGIN:VEVENT\r\nDTSTART:20260105T080000Z\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:nostart\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:bad\r\nDTSTART:garbage\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:ok\r\nDTSTART:20260105T080000Z\r\nDTEND:20260105T070000Z\r\nEND:VEVENT\r\n",
    );
    let events = parse_calendar(&ics).expect("parse");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].uid, "ok");
    // An end before the start collapses to the start.
    assert_eq!(events[0].end, events[0].start);
    // Mismatched nesting.
    let broken = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n";
    assert_eq!(parse_calendar(broken), Err(CalendarError::Parse));
}

#[test]
fn event_count_is_bounded() {
    let mut body = String::new();
    for index in 0..=MAX_EVENTS {
        body.push_str(&format!(
            "BEGIN:VEVENT\r\nUID:{index}\r\nDTSTART:20260105T080000Z\r\nEND:VEVENT\r\n"
        ));
    }
    assert_eq!(parse_calendar(&wrap(&body)), Err(CalendarError::Limit));
}

#[test]
fn feed_name_and_colour_are_read() {
    let ics = "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Holidays\r\nX-APPLE-CALENDAR-COLOR:#ff8800\r\n\
               BEGIN:VEVENT\r\nX-WR-CALNAME:no\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let (name, color) = calendar_properties(ics);
    assert_eq!(name.as_deref(), Some("Holidays"));
    assert_eq!(color.as_deref(), Some("#FF8800"));
}

#[test]
fn serialization_round_trips() {
    let ics = wrap(
        "BEGIN:VEVENT\r\nUID:rt@x\r\nSUMMARY:Plan\\, ship\r\nDESCRIPTION:a\\nb\r\n\
         DTSTART;TZID=Europe/Berlin:20260105T090000\r\nDTEND;TZID=Europe/Berlin:20260105T100000\r\n\
         RRULE:FREQ=DAILY;COUNT=3\r\nEXDATE;TZID=Europe/Berlin:20260106T090000\r\n\
         ORGANIZER;CN=\"Boss\":mailto:boss@example.com\r\n\
         ATTENDEE;CN=\"Ann\";PARTSTAT=ACCEPTED:mailto:ann@example.com\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
    );
    let original = parse_calendar(&ics).expect("parse").remove(0);
    let text = serialize_event(&original, utc("2026-01-01T00:00:00Z"));
    assert!(text.lines().all(|line| line.len() <= 75));
    let again = parse_calendar(&text).expect("reparse").remove(0);
    assert_eq!(again, original);
}
