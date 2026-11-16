//! R2-B2: writing an event back keeps every line the model does not own.
//! Rich Outlook, Google and Thunderbird resources go through edit, move,
//! occurrence edit and invitation answer; each unmodelled line must survive.

use super::service::{draft, harness, signed_in_on};
use super::{response, utc};
use crate::api::CalendarEventDraftV1;
use crate::http::Method;
use crate::ical::{parse_calendar, serialize_event_over};
use crate::invite::{self, DeclineMode, Response};

const OUTLOOK: &str = "BEGIN:VCALENDAR
PRODID:Microsoft Exchange Server 2010
VERSION:2.0
BEGIN:VTIMEZONE
TZID:W. Europe Standard Time
BEGIN:STANDARD
DTSTART:16010101T030000
TZOFFSETFROM:+0200
TZOFFSETTO:+0100
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=10
END:STANDARD
BEGIN:DAYLIGHT
DTSTART:16010101T020000
TZOFFSETFROM:+0100
TZOFFSETTO:+0200
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=-1SU;BYMONTH=3
END:DAYLIGHT
END:VTIMEZONE
BEGIN:VEVENT
UID:out@x
DTSTAMP:20260101T000000Z
SUMMARY:Planning
DTSTART;TZID=W. Europe Standard Time:20260112T090000
DTEND;TZID=W. Europe Standard Time:20260112T100000
CLASS:PRIVATE
TRANSP:TRANSPARENT
CATEGORIES:Blue category,Red category
X-MICROSOFT-CDO-BUSYSTATUS:FREE
X-MICROSOFT-CDO-IMPORTANCE:2
ATTACH;FMTTYPE=application/pdf:https://files.example/agenda.pdf
ORGANIZER;CN=Bob Boss:MAILTO:bob@corp.example
ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CUTYPE=INDIVIDUAL;CN=Ann Smith;SCHEDULE-STATUS=1.2:MAILTO:ann@example.com
ATTENDEE;ROLE=OPT-PARTICIPANT;CUTYPE=RESOURCE;CN=Room 4:mailto:room4@corp.example
SEQUENCE:0
BEGIN:VALARM
ACTION:EMAIL
TRIGGER:-PT30M
SUMMARY:Mail reminder
DESCRIPTION:Mail me
ATTENDEE:mailto:ann@example.com
END:VALARM
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER;RELATED=END:PT0S
DESCRIPTION:End reminder
END:VALARM
END:VEVENT
END:VCALENDAR
";

const GOOGLE: &str = "BEGIN:VCALENDAR
PRODID:-//Google Inc//Google Calendar 70.9054//EN
VERSION:2.0
CALSCALE:GREGORIAN
BEGIN:VEVENT
DTSTART:20260121T090000Z
DTEND:20260121T093000Z
DTSTAMP:20260110T080000Z
ORGANIZER;CN=Dana:mailto:dana@gmail.example
UID:gcal123@google.com
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=ann@example.com;X-NUM-GUESTS=0:mailto:ann@example.com
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE;CN=Dana;X-NUM-GUESTS=0:mailto:dana@gmail.example
CREATED:20260101T000000Z
DESCRIPTION:Join at https://meet.example/abc
GEO:52.52;13.405
LAST-MODIFIED:20260110T080000Z
LOCATION:Berlin
SEQUENCE:0
STATUS:CONFIRMED
SUMMARY:Sync
TRANSP:OPAQUE
CONFERENCE;VALUE=URI;FEATURE=VIDEO:https://meet.example/abc
X-GOOGLE-CONFERENCE:https://meet.example/abc
ATTACH;FILENAME=notes.txt:https://files.example/notes.txt
BEGIN:VALARM
ACTION:EMAIL
TRIGGER:-P1D
DESCRIPTION:Mail me
SUMMARY:Reminder
ATTENDEE:mailto:ann@example.com
END:VALARM
BEGIN:VALARM
ACTION:DISPLAY
TRIGGER:-PT10M
DESCRIPTION:This is an event reminder
END:VALARM
END:VEVENT
END:VCALENDAR
";

const THUNDERBIRD: &str = "BEGIN:VCALENDAR
PRODID:-//Mozilla.org/NONSGML Mozilla Calendar V1.1//EN
VERSION:2.0
X-WR-CALNAME:Team
BEGIN:VTIMEZONE
TZID:Europe/Berlin
BEGIN:STANDARD
TZOFFSETFROM:+0200
TZOFFSETTO:+0100
TZNAME:CET
DTSTART:19701025T030000
RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10
END:STANDARD
BEGIN:DAYLIGHT
TZOFFSETFROM:+0100
TZOFFSETTO:+0200
TZNAME:CEST
DTSTART:19700329T020000
RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3
END:DAYLIGHT
END:VTIMEZONE
BEGIN:VEVENT
UID:tb@x
DTSTAMP:20260101T000000Z
SUMMARY:Standup
DTSTART;TZID=Europe/Berlin:20260112T090000
DTEND;TZID=Europe/Berlin:20260112T091500
RRULE:FREQ=DAILY;COUNT=5
EXDATE;TZID=Europe/Berlin:20260113T090000
CATEGORIES:Work
TRANSP:OPAQUE
X-MOZ-GENERATION:3
X-MOZ-SNOOZE-TIME-1768200000000000:20260112T080000Z
SEQUENCE:2
BEGIN:VALARM
ACTION:AUDIO
TRIGGER:-PT5M
X-MOZ-LASTACK:20260111T000000Z
END:VALARM
END:VEVENT
BEGIN:VEVENT
UID:tb@x
RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000
DTSTAMP:20260101T000000Z
SUMMARY:Standup (moved)
DTSTART;TZID=Europe/Berlin:20260114T100000
DTEND;TZID=Europe/Berlin:20260114T101500
X-MOZ-OVERRIDE:keep-me
CLASS:CONFIDENTIAL
SEQUENCE:2
END:VEVENT
END:VCALENDAR
";

/// The logical (unfolded) lines of a document.
fn lines(text: &str) -> Vec<String> {
    text.replace("\r\n ", "")
        .replace('\r', "")
        .split('\n')
        .map(|line| line.trim_end().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Every line of `original` is still in `output`, except those starting with
/// one of `changed` (properties the edit meant to change).
fn assert_kept(original: &str, output: &str, changed: &[&str]) {
    let written = lines(output);
    for line in lines(original) {
        if changed.iter().any(|prefix| line.starts_with(prefix)) {
            continue;
        }
        assert!(
            written.contains(&line),
            "line lost: {line}\n--- output ---\n{output}"
        );
    }
}

/// A draft that restates `event` with a new summary.
fn restate(text: &str, summary: &str) -> CalendarEventDraftV1 {
    let event = &parse_calendar(text).expect("parses")[0];
    let local = |time: &crate::model::EventTime| {
        let text = time.local.format("%Y-%m-%dT%H:%M:%S").to_string();
        if time.kind == crate::model::TimeKind::Utc {
            format!("{text}Z")
        } else {
            text
        }
    };
    CalendarEventDraftV1 {
        summary: summary.to_owned(),
        description: event.description.clone(),
        location: event.location.clone(),
        start: local(&event.start),
        end: local(&event.end),
        all_day: false,
        tzid: match &event.start.kind {
            crate::model::TimeKind::Zoned(zone) => Some(zone.name().to_owned()),
            _ => None,
        },
        rrule: event.rrule.clone(),
        reminders: event
            .alarms
            .iter()
            .filter(|alarm| !alarm.from_end)
            .map(|alarm| u32::try_from(alarm.minutes_before).expect("small"))
            .collect(),
        url: event.url.clone(),
        recurrence: None,
    }
}

fn stamp() -> chrono::DateTime<chrono::Utc> {
    utc("2026-01-10T12:00:00Z")
}

#[test]
fn an_edit_keeps_every_unmodelled_line_of_each_vendor_resource() {
    for (text, summary_line) in [
        (OUTLOOK, "SUMMARY:Planning"),
        (GOOGLE, "SUMMARY:Sync"),
        (THUNDERBIRD, "SUMMARY:Standup"),
    ] {
        let base = &parse_calendar(text).expect("parses")[0];
        let draft = restate(text, "Renamed");
        let updated = crate::api::apply_draft(&base.uid, &draft, Some(base)).expect("applies");
        let out = serialize_event_over(&updated, stamp(), Some(text));
        assert_kept(text, &out, &["DTSTAMP", "SEQUENCE", summary_line]);
        assert!(out.contains("SUMMARY:Renamed"), "{out}");
        assert!(lines(&out).contains(&format!("SEQUENCE:{}", base.sequence + 1)));
        assert!(out.contains("DTSTAMP:20260110T120000Z"));
        // Unchanged times keep the vendor's own zone name and VTIMEZONE.
        let reread = parse_calendar(&out).expect("reads back");
        assert_eq!(reread[0].summary, "Renamed");
        assert_eq!(reread[0].start, base.start);
        assert_eq!(reread[0].end, base.end);
    }
}

#[test]
fn a_move_replaces_the_times_and_keeps_everything_else() {
    let base = &parse_calendar(OUTLOOK).expect("parses")[0];
    let mut draft = restate(OUTLOOK, "Planning");
    draft.start = "2026-01-14T10:00:00".to_owned();
    draft.end = "2026-01-14T11:30:00".to_owned();
    let moved = crate::api::apply_draft(&base.uid, &draft, Some(base)).expect("applies");
    let out = serialize_event_over(&moved, stamp(), Some(OUTLOOK));
    assert_kept(
        OUTLOOK,
        &out,
        &[
            "DTSTAMP",
            "SEQUENCE",
            "DTSTART;TZID=W. Europe Standard Time:20260112",
            "DTEND;TZID=W. Europe",
        ],
    );
    let written = lines(&out);
    assert!(written.contains(&"DTSTART;TZID=Europe/Berlin:20260114T100000".to_owned()));
    assert!(written.contains(&"DTEND;TZID=Europe/Berlin:20260114T113000".to_owned()));
    // The vendor's zone stays, and the new zone is described for the server.
    assert!(out.contains("TZID:W. Europe Standard Time"));
    assert!(out.contains("TZID:Europe/Berlin"));
    let reread = parse_calendar(&out).expect("reads back");
    assert_eq!(reread[0].start.to_utc(), utc("2026-01-14T09:00:00Z"));
}

#[test]
fn reminders_are_kept_as_they_are_and_only_new_ones_are_added_or_removed() {
    let base = &parse_calendar(OUTLOOK).expect("parses")[0];
    // The editor shows the 30-minute EMAIL alarm; adding a 10-minute one
    // keeps the EMAIL alarm and the end-relative alarm unchanged.
    let mut draft = restate(OUTLOOK, "Planning");
    draft.reminders = vec![30, 10];
    let event = crate::api::apply_draft(&base.uid, &draft, Some(base)).expect("applies");
    let out = serialize_event_over(&event, stamp(), Some(OUTLOOK));
    assert_kept(OUTLOOK, &out, &["DTSTAMP", "SEQUENCE"]);
    assert!(out.contains("TRIGGER:-PT10M"));
    // Removing the 30-minute reminder drops that alarm only.
    draft.reminders = Vec::new();
    let event = crate::api::apply_draft(&base.uid, &draft, Some(base)).expect("applies");
    let out = serialize_event_over(&event, stamp(), Some(OUTLOOK));
    assert!(!out.contains("Mail reminder"));
    assert!(out.contains("TRIGGER;RELATED=END:PT0S"));
}

#[test]
fn attendee_changes_patch_only_the_changed_parameters() {
    let mut event = parse_calendar(GOOGLE).expect("parses").remove(0);
    event.attendees[0].partstat = crate::model::PartStat::Accepted;
    event.attendees.remove(1);
    event.attendees.push(crate::model::Person {
        email: "new@example.org".to_owned(),
        name: Some("New".to_owned()),
        partstat: crate::model::PartStat::NeedsAction,
    });
    let out = serialize_event_over(&event, stamp(), Some(GOOGLE));
    let written = lines(&out);
    let ann = written
        .iter()
        .find(|line| line.ends_with(":mailto:ann@example.com") && line.starts_with("ATTENDEE"))
        .expect("ann");
    for part in [
        "PARTSTAT=ACCEPTED",
        "CUTYPE=INDIVIDUAL",
        "ROLE=REQ-PARTICIPANT",
        "RSVP=TRUE",
        "X-NUM-GUESTS=0",
    ] {
        assert!(ann.contains(part), "{ann}");
    }
    assert!(!written
        .iter()
        .any(|l| l.starts_with("ATTENDEE") && l.contains("dana@gmail.example")));
    assert!(written
        .iter()
        .any(|l| l.starts_with("ATTENDEE") && l.ends_with(":mailto:new@example.org")));
    // Everything else, including the organizer's own line and both alarms.
    assert_kept(
        GOOGLE,
        &out,
        &[
            "DTSTAMP",
            "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION",
            "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED",
        ],
    );
}

#[test]
fn an_unreadable_original_falls_back_to_writing_the_model() {
    let event = parse_calendar(OUTLOOK).expect("parses").remove(0);
    for original in [
        None,
        Some("not a calendar"),
        Some("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:other\nEND:VEVENT\nEND:VCALENDAR\n"),
    ] {
        let out = serialize_event_over(&event, stamp(), original);
        assert!(
            out.contains("UID:out@x") && out.contains("BEGIN:VCALENDAR"),
            "{out}"
        );
        assert_eq!(parse_calendar(&out).expect("reads")[0].summary, "Planning");
    }
}

fn report(data: &str) -> String {
    super::caldav::multistatus(&format!(
        "<d:response><d:href>/dav/calendars/ann/work/one.ics</d:href><d:propstat><d:prop>\
         <d:getetag>\"e1\"</d:getetag><c:calendar-data>{data}</c:calendar-data>\
         </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    ))
}

fn last_put(h: &super::service::Harness) -> String {
    let puts: Vec<_> = h
        .fake
        .requests()
        .into_iter()
        .filter(|r| r.method == Method::Put)
        .collect();
    String::from_utf8_lossy(&puts.last().expect("a PUT").body).into_owned()
}

#[tokio::test]
async fn update_event_over_a_server_copy_keeps_its_lines() {
    let h = signed_in_on(harness(), report(OUTLOOK)).await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(204, &[("etag", "\"e2\"")], ""),
    );
    let mut edit = restate(OUTLOOK, "Planning (final)");
    edit.location = "Room 9".to_owned();
    h.service
        .update_event(&work, "out@x", &edit)
        .await
        .expect("updated");
    let body = last_put(&h);
    assert_kept(OUTLOOK, &body, &["DTSTAMP", "SEQUENCE", "SUMMARY:Planning"]);
    assert!(body.contains("SUMMARY:Planning (final)") && body.contains("LOCATION:Room 9"));
    // A second edit patches the text the first one stored.
    let mut again = restate(&body, "Planning (final 2)");
    again.location = "Room 9".to_owned();
    h.service
        .update_event(&work, "out@x", &again)
        .await
        .expect("updated again");
    let body = last_put(&h);
    assert!(body.contains("CATEGORIES:Blue category,Red category"));
    assert!(body.contains("SUMMARY:Planning (final 2)"));
}

#[tokio::test]
async fn occurrence_edits_keep_the_series_and_its_exceptions_intact() {
    let h = signed_in_on(harness(), report(THUNDERBIRD)).await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(204, &[("etag", "\"e2\"")], ""),
    );

    // A new exception for the 15th.
    let mut new_one = draft("Standup (late)");
    new_one.location = String::new();
    new_one.start = "2026-01-15T09:30:00".to_owned();
    new_one.end = "2026-01-15T09:45:00".to_owned();
    new_one.reminders = vec![5];
    h.service
        .update_occurrence(&work, "tb@x", "2026-01-15T09:00:00", &new_one)
        .await
        .expect("new exception");
    let body = last_put(&h);
    assert_kept(THUNDERBIRD, &body, &["DTSTAMP", "SEQUENCE"]);
    assert!(body.contains("RECURRENCE-ID;TZID=Europe/Berlin:20260115T090000"));
    assert!(body.contains("SUMMARY:Standup (late)"));

    // Editing the existing exception keeps its own unmodelled lines.
    let mut again = draft("Standup (moved again)");
    again.location = String::new();
    again.start = "2026-01-14T10:00:00".to_owned();
    again.end = "2026-01-14T10:15:00".to_owned();
    again.reminders = vec![5];
    h.service
        .update_occurrence(&work, "tb@x", "2026-01-14T09:00:00", &again)
        .await
        .expect("edited exception");
    let body = last_put(&h);
    for kept in [
        "X-MOZ-OVERRIDE:keep-me",
        "CLASS:CONFIDENTIAL",
        "X-MOZ-GENERATION:3",
        "CATEGORIES:Work",
        "EXDATE;TZID=Europe/Berlin:20260113T090000",
        "X-MOZ-LASTACK:20260111T000000Z",
    ] {
        assert!(lines(&body).contains(&kept.to_owned()), "{kept}\n{body}");
    }
    assert!(body.contains("SUMMARY:Standup (moved again)"));
    assert!(!body.contains("SUMMARY:Standup (moved)"));

    // Deleting an occurrence adds an EXDATE and keeps the rest.
    h.service
        .delete_occurrence(&work, "tb@x", "2026-01-16T09:00:00")
        .await
        .expect("deleted");
    let body = last_put(&h);
    assert!(body.contains("EXDATE;TZID=Europe/Berlin:20260116T090000"));
    assert!(body.contains("X-MOZ-GENERATION:3") && body.contains("CLASS:CONFIDENTIAL"));
}

fn itip(text: &str, method: &str) -> invite::Invitation {
    let text = text.replacen(
        "VERSION:2.0\n",
        &format!("VERSION:2.0\nMETHOD:{method}\n"),
        1,
    );
    invite::parse(text.replace('\n', "\r\n").as_bytes()).expect("parses")
}

#[tokio::test]
async fn answering_an_invitation_stores_every_line_the_organizer_sent() {
    for (text, own) in [(OUTLOOK, "ann@example.com"), (GOOGLE, "ann@example.com")] {
        let h = signed_in_on(
            harness(),
            report("BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR\n"),
        )
        .await;
        h.service.sync_all().await;
        let work = h.service.calendars()[0].id.clone();
        h.fake.route(
            Method::Put,
            "*",
            "",
            response(201, &[("etag", "\"i1\"")], ""),
        );
        let inv = itip(text, "REQUEST");
        h.service
            .respond_invitation(
                &inv,
                &[own.to_owned()],
                Response::Accept,
                Some(&work),
                DeclineMode::Remove,
            )
            .await
            .expect("responds");
        let body = last_put(&h);
        assert!(!body.contains("METHOD:"), "{body}");
        assert_kept(
            text,
            &body,
            &[
                "DTSTAMP",
                "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION",
                "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION",
            ],
        );
        let mine = lines(&body)
            .into_iter()
            .find(|line| {
                line.starts_with("ATTENDEE")
                    && line
                        .to_ascii_lowercase()
                        .ends_with(":mailto:ann@example.com")
                    && !line.contains("PARTSTAT=NEEDS")
                    && line.contains("RSVP")
            })
            .expect("the user's attendee line");
        assert!(mine.contains("PARTSTAT=ACCEPTED"), "{mine}");
        assert!(mine.contains("RSVP=TRUE"), "{mine}");
    }
}

#[tokio::test]
async fn answering_over_a_stored_copy_keeps_the_users_own_lines() {
    let stored = OUTLOOK.replace(
        "X-MICROSOFT-CDO-IMPORTANCE:2",
        "X-MICROSOFT-CDO-IMPORTANCE:2\nX-MY-NOTE:local",
    );
    let h = signed_in_on(harness(), report(&stored)).await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(204, &[("etag", "\"e2\"")], ""),
    );
    let inv = itip(OUTLOOK, "REQUEST");
    h.service
        .respond_invitation(
            &inv,
            &["ann@example.com".to_owned()],
            Response::Tentative,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("responds");
    let body = last_put(&h);
    assert_kept(
        &stored,
        &body,
        &[
            "DTSTAMP",
            "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION",
        ],
    );
    assert!(body.contains("PARTSTAT=TENTATIVE"));
}
