use super::utc;
use crate::ical::parse_calendar;
use crate::model::Event;
use crate::recur::{expand, validate_rule};
use chrono::{DateTime, Utc};

fn events(body: &str) -> Vec<Event> {
    parse_calendar(&format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n"
    ))
    .expect("parse")
}

fn starts(event: &Event, from: &str, to: &str) -> Vec<DateTime<Utc>> {
    let expansion = expand(event, utc(from), utc(to), 1_000);
    let mut out: Vec<_> = expansion
        .occurrences
        .iter()
        .map(|occurrence| occurrence.start.to_utc())
        .collect();
    out.sort();
    out
}

const WEEKLY: &str = "BEGIN:VEVENT\r\nUID:w@x\r\nSUMMARY:Sync\r\n\
    DTSTART;TZID=Europe/Berlin:20260105T090000\r\nDTEND;TZID=Europe/Berlin:20260105T100000\r\n\
    RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6\r\n";

#[test]
fn weekly_rule_with_exdate() {
    let event = events(&format!(
        "{WEEKLY}EXDATE;TZID=Europe/Berlin:20260107T090000\r\nEND:VEVENT\r\n"
    ))
    .remove(0);
    // Jan 5, 7, 12, 14, 19, 21 at 09:00 Berlin (08:00Z); Jan 7 excluded.
    let got = starts(&event, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z");
    let want: Vec<_> = [
        "2026-01-05T08:00:00Z",
        "2026-01-12T08:00:00Z",
        "2026-01-14T08:00:00Z",
        "2026-01-19T08:00:00Z",
        "2026-01-21T08:00:00Z",
    ]
    .iter()
    .map(|s| utc(s))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn range_filters_occurrences_and_keeps_the_duration() {
    let event = events(&format!("{WEEKLY}END:VEVENT\r\n")).remove(0);
    let expansion = expand(
        &event,
        utc("2026-01-12T00:00:00Z"),
        utc("2026-01-13T00:00:00Z"),
        1_000,
    );
    assert_eq!(expansion.occurrences.len(), 1);
    let occurrence = &expansion.occurrences[0];
    assert_eq!(occurrence.end.to_utc(), utc("2026-01-12T09:00:00Z"));
    assert!(occurrence.instance.is_some());
    // An occurrence that started before the range but still runs is included.
    let running = expand(
        &event,
        utc("2026-01-12T08:30:00Z"),
        utc("2026-01-12T08:40:00Z"),
        1_000,
    );
    assert_eq!(running.occurrences.len(), 1);
}

#[test]
fn override_replaces_and_moves_one_occurrence() {
    let event = events(&format!(
        "{WEEKLY}END:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:w@x\r\nSUMMARY:Sync (moved)\r\n\
         RECURRENCE-ID;TZID=Europe/Berlin:20260112T090000\r\n\
         DTSTART;TZID=Europe/Berlin:20260113T140000\r\nDTEND;TZID=Europe/Berlin:20260113T150000\r\n\
         END:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:w@x\r\nSUMMARY:Sync (cancelled)\r\nSTATUS:CANCELLED\r\n\
         RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000\r\n\
         DTSTART;TZID=Europe/Berlin:20260114T090000\r\nEND:VEVENT\r\n"
    ))
    .remove(0);
    let expansion = expand(
        &event,
        utc("2026-01-10T00:00:00Z"),
        utc("2026-01-16T00:00:00Z"),
        1_000,
    );
    let mut found: Vec<(DateTime<Utc>, String)> = expansion
        .occurrences
        .iter()
        .map(|o| (o.start.to_utc(), o.event.summary.clone()))
        .collect();
    found.sort();
    // Jan 12 09:00 is gone, the moved one shows at Jan 13 14:00 (13:00Z) and
    // the cancelled Jan 14 instance is neither shown nor regenerated.
    assert_eq!(
        found,
        vec![(utc("2026-01-13T13:00:00Z"), "Sync (moved)".to_owned())]
    );
}

#[test]
fn monthly_rules() {
    let day31 = events(
        "BEGIN:VEVENT\r\nUID:m@x\r\nDTSTART:20260131T100000Z\r\nDTEND:20260131T110000Z\r\n\
         RRULE:FREQ=MONTHLY;BYMONTHDAY=31\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        starts(&day31, "2026-01-01T00:00:00Z", "2026-07-01T00:00:00Z"),
        vec![
            utc("2026-01-31T10:00:00Z"),
            utc("2026-03-31T10:00:00Z"),
            utc("2026-05-31T10:00:00Z")
        ]
    );
    let second_tuesday = events(
        "BEGIN:VEVENT\r\nUID:t@x\r\nDTSTART;TZID=Europe/Berlin:20260113T180000\r\n\
         DTEND;TZID=Europe/Berlin:20260113T190000\r\nRRULE:FREQ=MONTHLY;BYDAY=2TU\r\n\
         EXDATE;TZID=Europe/Berlin:20260210T180000\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        starts(
            &second_tuesday,
            "2026-01-01T00:00:00Z",
            "2026-04-01T00:00:00Z"
        ),
        vec![utc("2026-01-13T17:00:00Z"), utc("2026-03-10T17:00:00Z")]
    );
}

#[test]
fn daylight_saving_keeps_the_wall_clock_time() {
    // Berlin switches to summer time on 2026-03-29 (02:00 -> 03:00).
    let event = events(
        "BEGIN:VEVENT\r\nUID:d@x\r\nDTSTART;TZID=Europe/Berlin:20260316T090000\r\n\
         DTEND;TZID=Europe/Berlin:20260316T100000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        starts(&event, "2026-03-01T00:00:00Z", "2026-05-01T00:00:00Z"),
        vec![
            utc("2026-03-16T08:00:00Z"),
            utc("2026-03-23T08:00:00Z"),
            utc("2026-03-30T07:00:00Z"),
        ]
    );
}

#[test]
fn daylight_saving_gap_moves_forward() {
    // 02:30 does not exist in Berlin on 2026-03-29.
    let event = events(
        "BEGIN:VEVENT\r\nUID:g@x\r\nDTSTART;TZID=Europe/Berlin:20260329T023000\r\n\
         DTEND;TZID=Europe/Berlin:20260329T033000\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(event.start.to_utc(), utc("2026-03-29T01:30:00Z"));
    // A repeated hour in autumn resolves to its first occurrence.
    let fold = events(
        "BEGIN:VEVENT\r\nUID:h@x\r\nDTSTART;TZID=Europe/Berlin:20261025T023000\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(fold.start.to_utc(), utc("2026-10-25T00:30:00Z"));
}

#[test]
fn all_day_recurrence_with_until_date() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:a@x\r\nDTSTART;VALUE=DATE:20260105\r\nDTEND;VALUE=DATE:20260106\r\n\
         RRULE:FREQ=DAILY;UNTIL=20260107\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    let expansion = expand(
        &event,
        utc("2026-01-01T00:00:00Z"),
        utc("2026-02-01T00:00:00Z"),
        1_000,
    );
    assert_eq!(expansion.occurrences.len(), 3);
    assert!(expansion.occurrences.iter().all(|o| o.start.is_date()));
    assert_eq!(
        expansion.occurrences[2].end.local.date().to_string(),
        "2026-01-08"
    );
}

#[test]
fn rdate_adds_an_instance_and_cancelled_events_vanish() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:r@x\r\nDTSTART:20260105T100000Z\r\nDTEND:20260105T110000Z\r\n\
         RDATE:20260110T100000Z\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        starts(&event, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z"),
        vec![utc("2026-01-05T10:00:00Z"), utc("2026-01-10T10:00:00Z")]
    );
    let cancelled = events(
        "BEGIN:VEVENT\r\nUID:c@x\r\nDTSTART:20260105T100000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert!(starts(&cancelled, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z").is_empty());
}

#[test]
fn expansion_is_bounded() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:inf@x\r\nDTSTART:20260101T100000Z\r\nDTEND:20260101T103000Z\r\n\
         RRULE:FREQ=DAILY\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    let expansion = expand(
        &event,
        utc("2026-01-01T00:00:00Z"),
        utc("2030-01-01T00:00:00Z"),
        50,
    );
    assert_eq!(expansion.occurrences.len(), 50);
    assert!(expansion.truncated);
    // An empty or inverted range yields nothing.
    let none = expand(
        &event,
        utc("2026-02-01T00:00:00Z"),
        utc("2026-01-01T00:00:00Z"),
        50,
    );
    assert!(none.occurrences.is_empty());
}

#[test]
fn unusable_rule_degrades_to_the_first_instance() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:bad@x\r\nDTSTART:20260105T100000Z\r\nDTEND:20260105T110000Z\r\n\
         RRULE:FREQ=SOMETIMES\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        starts(&event, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z").len(),
        1
    );
    assert!(validate_rule("FREQ=SOMETIMES", &event.start).is_err());
    assert!(validate_rule("FREQ=WEEKLY;BYDAY=MO", &event.start).is_ok());
}

#[test]
fn a_rule_that_repeats_within_the_hour_degrades_to_the_first_instance() {
    // Reaching a window walks every instance since DTSTART, so a rule from a
    // feed or an invitation that repeats each minute or second would hold the
    // calendar for minutes on every query. Such a rule is not expanded.
    for rule in [
        "FREQ=MINUTELY",
        "FREQ=SECONDLY",
        "freq=secondly;interval=2",
        "FREQ=DAILY;BYSECOND=0,1,2,3",
        "FREQ=DAILY;BYHOUR=0,1,2;BYMINUTE=0,1,2,3,4,5,6,7,8,9",
    ] {
        let text = format!(
            "BEGIN:VEVENT\r\nUID:fast@x\r\nDTSTART:20160105T100000Z\r\n\
             DTEND:20160105T100500Z\r\nRRULE:{rule}\r\nEND:VEVENT\r\n"
        );
        let event = events(&text).remove(0);
        let started = std::time::Instant::now();
        assert!(
            starts(&event, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z").is_empty(),
            "{rule} took {:?}",
            started.elapsed()
        );
        assert_eq!(
            starts(&event, "2016-01-01T00:00:00Z", "2016-02-01T00:00:00Z"),
            vec![utc("2016-01-05T10:00:00Z")],
            "{rule}"
        );
        assert!(validate_rule(rule, &event.start).is_err(), "{rule}");
    }
    // Hourly rules, also with a few minutes named, still repeat.
    let hourly = events(
        "BEGIN:VEVENT\r\nUID:hour@x\r\nDTSTART:20260105T100000Z\r\nDTEND:20260105T100500Z\r\n\
         RRULE:FREQ=HOURLY;BYMINUTE=0,30;BYSECOND=0;COUNT=4\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        starts(&hourly, "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z").len(),
        4
    );
}
