//! Reminder times, declines, missed batching and dedupe. Fixtures only.

use super::utc;
use crate::ical::parse_calendar;
use crate::model::{Event, PartStat, Person};
use crate::recur::{expand, Occurrence};
use crate::reminders::{
    day_start, declined_by, occurrence_triggers, plan, reminder_key, Ledger, Plan, Reminder,
    MAX_DUE_PER_PASS, MAX_FIRED,
};
use chrono::Duration;
use std::collections::HashSet;

fn events(body: &str) -> Vec<Event> {
    parse_calendar(&format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n"
    ))
    .expect("valid calendar")
}

fn triggers_between(event: &Event, from: &str, to: &str, default: Option<i64>) -> Vec<String> {
    let expansion = expand(event, utc(from), utc(to), 100);
    let mut out: Vec<String> = Vec::new();
    for occurrence in &expansion.occurrences {
        for trigger in occurrence_triggers(occurrence, default) {
            out.push(trigger.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        }
    }
    out.sort();
    out
}

fn first_occurrence(event: &Event) -> Occurrence<'_> {
    expand(
        event,
        utc("2026-01-01T00:00:00Z"),
        utc("2026-12-31T00:00:00Z"),
        10,
    )
    .occurrences
    .remove(0)
}

fn reminder(key: &str, trigger: &str) -> Reminder {
    Reminder {
        key: key.to_owned(),
        title: "Standup".to_owned(),
        start: utc(trigger) + Duration::minutes(15),
        all_day: false,
        location: String::new(),
        trigger: utc(trigger),
    }
}

#[test]
fn relative_to_start_and_to_end_fire_at_their_offsets() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:a@x\r\nSUMMARY:Review\r\nDTSTART:20260110T100000Z\r\n\
         DTEND:20260110T110000Z\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
         BEGIN:VALARM\r\nACTION:AUDIO\r\nTRIGGER;RELATED=END:-PT5M\r\nEND:VALARM\r\n\
         END:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        triggers_between(&event, "2026-01-10T00:00:00Z", "2026-01-11T00:00:00Z", None),
        vec!["2026-01-10T09:45:00Z", "2026-01-10T10:55:00Z"]
    );
}

#[test]
fn an_email_alarm_and_no_alarm_make_no_reminder() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:b@x\r\nSUMMARY:Mail only\r\nDTSTART:20260110T100000Z\r\n\
         DTEND:20260110T110000Z\r\n\
         BEGIN:VALARM\r\nACTION:EMAIL\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert!(
        triggers_between(&event, "2026-01-10T00:00:00Z", "2026-01-11T00:00:00Z", None).is_empty()
    );
    assert!(
        triggers_between(
            &event,
            "2026-01-10T00:00:00Z",
            "2026-01-11T00:00:00Z",
            Some(10)
        )
        .len()
            == 1,
        "a default reminder applies when the event has no DISPLAY or AUDIO alarm"
    );
}

#[test]
fn an_absolute_trigger_is_kept_as_its_offset_from_the_start() {
    // 07:00Z for a 10:00Z start is three hours before.
    let event = events(
        "BEGIN:VEVENT\r\nUID:c@x\r\nSUMMARY:Flight\r\nDTSTART:20260110T100000Z\r\n\
         DTEND:20260110T110000Z\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;VALUE=DATE-TIME:20260110T070000Z\r\n\
         END:VALARM\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert_eq!(
        triggers_between(&event, "2026-01-10T00:00:00Z", "2026-01-11T00:00:00Z", None),
        vec!["2026-01-10T07:00:00Z"]
    );
}

#[test]
fn an_all_day_event_reminds_on_its_local_day_and_never_takes_the_default() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:d@x\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20260110\r\n\
         DTEND;VALUE=DATE:20260111\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT60M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    let expansion = expand(
        &event,
        utc("2026-01-09T00:00:00Z"),
        utc("2026-01-12T00:00:00Z"),
        10,
    );
    let occurrence = &expansion.occurrences[0];
    let expected =
        day_start(chrono::NaiveDate::from_ymd_opt(2026, 1, 10).unwrap()) - Duration::minutes(60);
    assert_eq!(occurrence_triggers(occurrence, None), vec![expected]);

    let plain = events(
        "BEGIN:VEVENT\r\nUID:e@x\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20260110\r\n\
         DTEND;VALUE=DATE:20260111\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    assert!(occurrence_triggers(&first_occurrence(&plain), Some(15)).is_empty());
}

#[test]
fn a_daily_series_reminds_each_day_across_the_spring_daylight_jump() {
    // 09:00 Berlin is 08:00Z before the 29 March 2026 switch and 07:00Z after.
    let event = events(
        "BEGIN:VEVENT\r\nUID:f@x\r\nSUMMARY:Daily\r\n\
         DTSTART;TZID=Europe/Berlin:20260327T090000\r\n\
         DTEND;TZID=Europe/Berlin:20260327T093000\r\n\
         RRULE:FREQ=DAILY;COUNT=5\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    let got = triggers_between(&event, "2026-03-27T00:00:00Z", "2026-03-31T00:00:00Z", None);
    assert_eq!(
        got,
        vec![
            "2026-03-27T07:50:00Z",
            "2026-03-28T07:50:00Z",
            "2026-03-29T06:50:00Z",
            "2026-03-30T06:50:00Z",
        ]
    );
}

#[test]
fn a_declined_attendee_matches_the_account_address_only() {
    let people = vec![
        Person {
            email: "Ann@Example.com".to_owned(),
            name: None,
            partstat: PartStat::Declined,
        },
        Person {
            email: "bob@example.com".to_owned(),
            name: None,
            partstat: PartStat::Declined,
        },
    ];
    assert!(declined_by(&people, Some("ann@example.com")));
    assert!(!declined_by(&people, Some("carl@example.com")));
    assert!(!declined_by(&people, None));
    let accepted = vec![Person {
        email: "ann@example.com".to_owned(),
        name: None,
        partstat: PartStat::Accepted,
    }];
    assert!(!declined_by(&accepted, Some("ann@example.com")));
}

#[test]
fn a_reminder_key_names_the_calendar_event_occurrence_and_trigger() {
    let event = events(
        "BEGIN:VEVENT\r\nUID:g@x\r\nSUMMARY:Series\r\nDTSTART:20260110T100000Z\r\n\
         DTEND:20260110T110000Z\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEND:VEVENT\r\n",
    )
    .remove(0);
    let expansion = expand(
        &event,
        utc("2026-01-10T00:00:00Z"),
        utc("2026-01-12T00:00:00Z"),
        10,
    );
    let second = &expansion.occurrences[1];
    let instance = second.instance.as_ref().expect("series occurrence");
    let trigger = utc("2026-01-11T09:50:00Z");
    assert_eq!(
        reminder_key("cal-1", "g@x", Some(instance), trigger),
        "calendar|cal-1|g@x|2026-01-11T10:00:00Z|2026-01-11T09:50:00Z"
    );
    assert_ne!(
        reminder_key("cal-1", "g@x", Some(instance), trigger),
        reminder_key("cal-2", "g@x", Some(instance), trigger),
        "the same invitation in two calendars is two reminders"
    );
}

#[test]
fn a_reminder_due_now_is_shown_once() {
    let now = utc("2026-01-10T09:50:00Z");
    let due = reminder("k1", "2026-01-10T09:45:00Z");
    let since = now - Duration::minutes(5);
    let first = plan(now, since, &HashSet::new(), std::slice::from_ref(&due));
    assert_eq!(first.due, vec![due.clone()]);
    assert_eq!(first.missed, 0);
    assert_eq!(first.handled, vec!["k1".to_owned()]);

    let recorded: HashSet<String> = first.handled.into_iter().collect();
    let again = plan(
        now + Duration::seconds(30),
        now,
        &recorded,
        std::slice::from_ref(&due),
    );
    assert!(again.due.is_empty());
    assert_eq!(again.missed, 0);
}

#[test]
fn reminders_missed_during_sleep_are_counted_not_shown() {
    let now = utc("2026-01-10T12:00:00Z");
    // The last pass ran at 08:00; a laptop slept until noon.
    let since = utc("2026-01-10T08:00:00Z");
    let reminders: Vec<Reminder> = ["08:30", "09:00", "10:15", "11:30"]
        .iter()
        .enumerate()
        .map(|(index, time)| reminder(&format!("k{index}"), &format!("2026-01-10T{time}:00Z")))
        .collect();
    let result = plan(now, since, &HashSet::new(), &reminders);
    assert!(result.due.is_empty());
    assert_eq!(result.missed, 4);
    assert_eq!(result.handled.len(), 4);
    assert_eq!(result.next, None);
}

#[test]
fn reminders_from_before_the_last_pass_are_ignored_quietly() {
    let now = utc("2026-01-10T12:00:00Z");
    let since = utc("2026-01-10T11:00:00Z");
    let old = reminder("old", "2026-01-10T09:00:00Z");
    let result = plan(now, since, &HashSet::new(), std::slice::from_ref(&old));
    assert!(result.due.is_empty());
    assert_eq!(result.missed, 0);
    assert!(result.handled.is_empty());
}

#[test]
fn a_burst_of_due_reminders_shows_a_few_and_counts_the_rest() {
    let now = utc("2026-01-10T09:50:00Z");
    let reminders: Vec<Reminder> = (0..7)
        .map(|index| reminder(&format!("k{index}"), &format!("2026-01-10T09:4{index}:00Z")))
        .collect();
    let result = plan(now, now - Duration::minutes(1), &HashSet::new(), &reminders);
    assert_eq!(result.due.len(), MAX_DUE_PER_PASS);
    assert_eq!(result.missed, 2);
    assert_eq!(result.handled.len(), 7);
    assert_eq!(result.due[0].key, "k0", "earliest first");
}

#[test]
fn the_next_future_reminder_is_reported_for_the_scheduler() {
    let now = utc("2026-01-10T09:00:00Z");
    let soon = reminder("soon", "2026-01-10T09:20:00Z");
    let later = reminder("later", "2026-01-10T11:00:00Z");
    let result = plan(now, now, &HashSet::new(), &[later.clone(), soon.clone()]);
    assert!(result.due.is_empty());
    assert_eq!(result.next, Some(soon.trigger));
}

#[test]
fn a_scheduler_driven_by_a_fake_clock_shows_each_reminder_once() {
    let trigger = utc("2026-01-10T09:45:00Z");
    let reminders = vec![reminder("k1", "2026-01-10T09:45:00Z")];
    let mut ledger = Ledger::default();
    // First pass: nothing yet; the next reminder sets the wait.
    let start = utc("2026-01-10T09:00:00Z");
    let first = ledger.pass(start, &reminders);
    assert!(first.due.is_empty());
    assert_eq!(
        Ledger::wait_after(&first, start),
        std::time::Duration::from_secs(60)
    );
    // A wake at the trigger shows it.
    let at_trigger = ledger.pass(trigger, &reminders);
    assert_eq!(at_trigger.due.len(), 1);
    // Later passes, and a restart from the saved text, never show it again.
    let saved = ledger
        .save_if_due(trigger, &at_trigger)
        .expect("handled, so saved");
    let mut restarted = Ledger::load(Some(&saved));
    assert!(restarted
        .pass(trigger + Duration::minutes(1), &reminders)
        .due
        .is_empty());
    assert!(ledger
        .pass(trigger + Duration::minutes(2), &reminders)
        .due
        .is_empty());
}

#[test]
fn a_wake_from_sleep_counts_the_missed_reminders_once() {
    let mut ledger = Ledger::default();
    let before = utc("2026-01-10T08:00:00Z");
    ledger.pass(before, &[]);
    let reminders: Vec<Reminder> = ["08:30", "09:00", "10:15"]
        .iter()
        .enumerate()
        .map(|(index, time)| reminder(&format!("k{index}"), &format!("2026-01-10T{time}:00Z")))
        .collect();
    let woke = ledger.pass(utc("2026-01-10T12:00:00Z"), &reminders);
    assert!(woke.due.is_empty());
    assert_eq!(woke.missed, 3);
    let again = ledger.pass(utc("2026-01-10T12:01:00Z"), &reminders);
    assert_eq!(again.missed, 0, "missed reminders are not counted twice");
}

#[test]
fn the_wait_is_bounded_between_a_second_and_a_minute() {
    let now = utc("2026-01-10T09:00:00Z");
    let far = Plan {
        next: Some(now + Duration::hours(3)),
        ..Plan::default()
    };
    assert_eq!(
        Ledger::wait_after(&far, now),
        std::time::Duration::from_secs(60)
    );
    let soon = Plan {
        next: Some(now + Duration::milliseconds(200)),
        ..Plan::default()
    };
    assert_eq!(
        Ledger::wait_after(&soon, now),
        std::time::Duration::from_secs(1)
    );
    let mid = Plan {
        next: Some(now + Duration::seconds(20)),
        ..Plan::default()
    };
    assert_eq!(
        Ledger::wait_after(&mid, now),
        std::time::Duration::from_secs(20)
    );
    assert_eq!(
        Ledger::wait_after(&Plan::default(), now),
        std::time::Duration::from_secs(60)
    );
}

#[test]
fn a_saved_ledger_is_written_only_when_something_changed_or_five_minutes_passed() {
    let now = utc("2026-01-10T09:00:00Z");
    let mut ledger = Ledger::default();
    let first = ledger.pass(now, &[]);
    assert!(
        ledger.save_if_due(now, &first).is_some(),
        "the first pass saves"
    );
    let quiet = ledger.pass(now + Duration::minutes(1), &[]);
    assert!(ledger
        .save_if_due(now + Duration::minutes(1), &quiet)
        .is_none());
    let later = ledger.pass(now + Duration::minutes(6), &[]);
    assert!(ledger
        .save_if_due(now + Duration::minutes(6), &later)
        .is_some());
}

#[test]
fn unreadable_saved_text_starts_an_empty_ledger_and_fired_keys_are_bounded() {
    assert_eq!(Ledger::load(Some("not json")), Ledger::default());
    assert_eq!(Ledger::load(None), Ledger::default());
    let now = utc("2026-01-10T09:00:00Z");
    let many: Vec<Reminder> = (0..MAX_FIRED + 20)
        .map(|index| Reminder {
            key: format!("k{index}"),
            title: "Bulk".into(),
            start: now,
            all_day: false,
            location: String::new(),
            trigger: now - Duration::minutes(1),
        })
        .collect();
    let mut ledger = Ledger::default();
    ledger.pass(now - Duration::minutes(2), &[]);
    let plan = ledger.pass(now, &many);
    assert_eq!(plan.due.len(), MAX_DUE_PER_PASS);
    let saved = ledger.save_if_due(now, &plan).expect("saved");
    let reloaded = Ledger::load(Some(&saved));
    assert!(reloaded.fired_count() <= MAX_FIRED);
    assert!(reloaded.fired_count() > 0);
}
