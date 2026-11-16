use super::service::signed_in;
use super::{response, utc};
use crate::error::CalendarError;
use crate::http::Method;
use crate::invite::{
    self, apply_reply, build_reply, compare, event_to_store, normalize_address, parse, plan_cancel,
    CancelMode, CancelPlan, DeclineMode, Freshness, ItipMethod, Response, MAX_INVITATION_BYTES,
};
use crate::model::PartStat;
use crate::service::CalendarInvitationActionV1;

const OWN: &str = "Ann@Example.COM";

fn own() -> Vec<String> {
    vec!["other@example.org".to_owned(), OWN.to_owned()]
}

/// Outlook / Exchange style: Windows zone name, long UID, RSVP and roles.
fn outlook(sequence: u32, stamp: &str, method: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nMETHOD:{method}\r\nPRODID:Microsoft Exchange Server 2010\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nORGANIZER;CN=Bob Boss:MAILTO:bob@corp.example\r\n\
ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Ann Smith:MAILTO:ann@example.com\r\n\
ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=ACCEPTED;CN=Cy:mailto:cy@corp.example\r\n\
DESCRIPTION;LANGUAGE=en-US:Agenda\\nitems\r\nUID:040000008200E00074C5B7101A82E00800000000AB@corp\r\n\
SUMMARY;LANGUAGE=en-US:Quarterly planning\r\nDTSTART;TZID=W. Europe Standard Time:20260120T100000\r\n\
DTEND;TZID=W. Europe Standard Time:20260120T110000\r\nCLASS:PUBLIC\r\nPRIORITY:5\r\n\
DTSTAMP:{stamp}\r\nTRANSP:OPAQUE\r\nSTATUS:CONFIRMED\r\nSEQUENCE:{sequence}\r\nLOCATION;LANGUAGE=en-US:Room 4\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:REMINDER\r\nTRIGGER;RELATED=START:-PT15M\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

/// Google style: UTC times, `mailto:` attendees with `X-NUM-GUESTS`, no VTIMEZONE.
fn google(method: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\nVERSION:2.0\r\n\
CALSCALE:GREGORIAN\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nDTSTART:20260121T090000Z\r\n\
DTEND:20260121T093000Z\r\nDTSTAMP:20260110T080000Z\r\nORGANIZER;CN=Dana:mailto:dana@gmail.example\r\n\
UID:gcal123@google.com\r\n\
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=ann@example.com;X-NUM-GUESTS=0:mailto:ann@example.com\r\n\
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE;CN=Dana;X-NUM-GUESTS=0:mailto:dana@gmail.example\r\n\
CREATED:20260101T000000Z\r\nDESCRIPTION:Join at https://meet.example/abc\r\nLAST-MODIFIED:20260110T080000Z\r\n\
LOCATION:\r\nSEQUENCE:0\r\nSTATUS:CONFIRMED\r\nSUMMARY:Sync\r\nTRANSP:OPAQUE\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

/// Thunderbird / Lightning style: TZID with a VTIMEZONE and a recurring series
/// with one moved instance.
const THUNDERBIRD_SERIES: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Mozilla.org/NONSGML Mozilla Calendar V1.1//EN\r\n\
VERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\n\
TZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nTZNAME:CET\r\nDTSTART:19701025T030000\r\n\
RRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nCREATED:20260105T100000Z\r\nLAST-MODIFIED:20260105T100100Z\r\nDTSTAMP:20260105T100100Z\r\n\
UID:tb-series@example.org\r\nSUMMARY:Standup\r\nORGANIZER;CN=Eve:mailto:eve@example.org\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION;ROLE=CHAIR;RSVP=TRUE;CN=Ann:mailto:ann@example.com\r\n\
RRULE:FREQ=DAILY;COUNT=5\r\nDTSTART;TZID=Europe/Berlin:20260112T090000\r\n\
DTEND;TZID=Europe/Berlin:20260112T091500\r\nSEQUENCE:2\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTAMP:20260105T100100Z\r\nUID:tb-series@example.org\r\nSUMMARY:Standup (moved)\r\n\
ORGANIZER;CN=Eve:mailto:eve@example.org\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Ann:mailto:ann@example.com\r\n\
RECURRENCE-ID;TZID=Europe/Berlin:20260113T090000\r\nDTSTART;TZID=Europe/Berlin:20260113T100000\r\n\
DTEND;TZID=Europe/Berlin:20260113T101500\r\nSEQUENCE:2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

const THUNDERBIRD_CANCEL_INSTANCE: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Mozilla.org/NONSGML Mozilla Calendar V1.1//EN\r\n\
VERSION:2.0\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nDTSTAMP:20260106T100100Z\r\nUID:tb-series@example.org\r\n\
ORGANIZER;CN=Eve:mailto:eve@example.org\r\nATTENDEE;CN=Ann:mailto:ann@example.com\r\n\
RECURRENCE-ID;TZID=Europe/Berlin:20260114T090000\r\nSEQUENCE:3\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

const GOOGLE_REPLY: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\n\
VERSION:2.0\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nDTSTAMP:20260111T080000Z\r\nUID:own-event@zephium\r\n\
ORGANIZER:mailto:ann@example.com\r\n\
ATTENDEE;PARTSTAT=ACCEPTED;CN=Zed:mailto:ZED@example.net\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn parsed(text: &str) -> invite::Invitation {
    parse(text.as_bytes()).expect("parses")
}

#[test]
fn outlook_request_is_read_in_full() {
    let inv = parsed(&outlook(0, "20260110T080000Z", "REQUEST"));
    assert_eq!(inv.method, ItipMethod::Request);
    assert_eq!(inv.uid, "040000008200E00074C5B7101A82E00800000000AB@corp");
    assert_eq!(inv.sequence, 0);
    assert_eq!(inv.event.summary, "Quarterly planning");
    assert_eq!(inv.event.description, "Agenda\nitems");
    assert_eq!(inv.event.location, "Room 4");
    assert_eq!(
        inv.event.organizer.as_ref().expect("org").email,
        "bob@corp.example"
    );
    assert_eq!(inv.attendees.len(), 2);
    assert!(inv.attendees[0].rsvp);
    assert_eq!(inv.attendees[0].person.partstat, PartStat::NeedsAction);
    assert_eq!(inv.attendees[0].role.as_str(), "required");
    assert_eq!(inv.attendees[1].role.as_str(), "optional");
    assert_eq!(inv.attendees[1].person.partstat, PartStat::Accepted);
    assert_eq!(inv.dtstamp, Some(utc("2026-01-10T08:00:00Z")));
    assert!(inv.has_start);
}

#[test]
fn google_request_and_cancel_methods() {
    let request = parsed(&google("REQUEST"));
    assert_eq!(request.event.summary, "Sync");
    assert_eq!(request.event.location, "");
    assert_eq!(request.attendees.len(), 2);
    let cancel = parsed(&google("CANCEL"));
    assert_eq!(cancel.method, ItipMethod::Cancel);
}

#[test]
fn counter_is_read_but_never_answered_or_applied() {
    let counter = parsed(&google("COUNTER"));
    assert_eq!(counter.method, ItipMethod::Counter);
    assert_eq!(
        build_reply(
            &counter,
            &own(),
            Response::Accept,
            utc("2026-01-11T00:00:00Z")
        )
        .err(),
        Some(CalendarError::InvalidInput)
    );
    assert_eq!(
        event_to_store(&counter, None, &own(), Response::Accept).err(),
        Some(CalendarError::InvalidInput)
    );
}

#[test]
fn unsupported_or_missing_method_and_garbage_are_refused() {
    assert_eq!(
        parse(google("PUBLISH").as_bytes()).err(),
        Some(CalendarError::Unsupported)
    );
    let no_method = google("REQUEST").replace("METHOD:REQUEST\r\n", "");
    assert_eq!(
        parse(no_method.as_bytes()).err(),
        Some(CalendarError::Unsupported)
    );
    assert_eq!(parse(b"hello").err(), Some(CalendarError::Parse));
    assert_eq!(parse(b"").err(), Some(CalendarError::Parse));
    // Unbalanced components.
    assert_eq!(
        parse(b"BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n").err(),
        Some(CalendarError::Parse)
    );
    // A REQUEST needs a start.
    let no_start = google("REQUEST").replace("DTSTART:20260121T090000Z\r\n", "");
    assert_eq!(parse(no_start.as_bytes()).err(), Some(CalendarError::Parse));
}

#[test]
fn oversized_and_hostile_input_is_bounded() {
    let big = vec![b'a'; MAX_INVITATION_BYTES + 1];
    assert_eq!(parse(&big).err(), Some(CalendarError::TooLarge));
    // Invalid UTF-8 and a BOM do not panic.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(google("REQUEST").as_bytes());
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    assert!(parse(&bytes).is_ok());
    // A huge description is capped.
    let long = "x".repeat(200_000);
    let text = google("REQUEST").replace(
        "DESCRIPTION:Join at https://meet.example/abc",
        &format!("DESCRIPTION:{long}"),
    );
    let inv = parse(text.as_bytes()).expect("parses");
    assert_eq!(
        inv.event.description.chars().count(),
        invite::MAX_DESCRIPTION_CHARS
    );
    // Thousands of attendees are bounded.
    let mut many = google("REQUEST");
    let extra: String = (0..2_000)
        .map(|i| format!("ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:p{i}@example.com\r\n"))
        .collect();
    many = many.replace("CREATED:", &format!("{extra}CREATED:"));
    let inv = parse(many.as_bytes()).expect("parses");
    assert!(inv.attendees.len() <= crate::ical::MAX_ATTENDEES);
    assert_eq!(inv.attendees.len(), inv.event.attendees.len());
    // Deep nesting is refused.
    let nested = format!("BEGIN:VCALENDAR\r\n{}", "BEGIN:X\r\n".repeat(20));
    assert_eq!(parse(nested.as_bytes()).err(), Some(CalendarError::Parse));
}

#[test]
fn attendee_matching_ignores_case_and_mailto() {
    let inv = parsed(&outlook(0, "20260110T080000Z", "REQUEST"));
    assert_eq!(inv.own_attendee(&own()), Some(0));
    assert_eq!(
        inv.own_attendee(&["MAILTO:CY@corp.example".to_owned()]),
        Some(1)
    );
    assert_eq!(inv.own_attendee(&["nobody@example.com".to_owned()]), None);
    assert_eq!(inv.own_attendee(&[]), None);
    assert_eq!(inv.own_attendee(&[String::new(), "  ".to_owned()]), None);
    assert!(inv.is_organizer(&["BOB@corp.example".to_owned()]));
    assert!(!inv.is_organizer(&own()));
    assert_eq!(normalize_address(" MailTo:A@B.c "), "a@b.c");
}

#[test]
fn sequence_then_stamp_decide_which_is_newer() {
    let t1 = Some(utc("2026-01-10T08:00:00Z"));
    let t2 = Some(utc("2026-01-11T08:00:00Z"));
    assert_eq!(compare(1, t1, 0, t2), Freshness::Newer);
    assert_eq!(compare(0, t2, 1, t1), Freshness::Older);
    assert_eq!(compare(1, t2, 1, t1), Freshness::Newer);
    assert_eq!(compare(1, t1, 1, t2), Freshness::Older);
    assert_eq!(compare(1, t1, 1, t1), Freshness::Same);
    assert_eq!(compare(1, t1, 1, None), Freshness::Same);
    assert_eq!(compare(1, None, 1, None), Freshness::Same);
}

#[test]
fn reply_is_minimal_and_addressed_to_the_organizer() {
    let inv = parsed(&outlook(3, "20260110T080000Z", "REQUEST"));
    let now = utc("2026-01-11T09:30:00Z");
    for (response, subject, stat) in [
        (Response::Accept, "Accepted: Quarterly planning", "ACCEPTED"),
        (
            Response::Tentative,
            "Tentative: Quarterly planning",
            "TENTATIVE",
        ),
        (
            Response::Decline,
            "Declined: Quarterly planning",
            "DECLINED",
        ),
    ] {
        let reply = build_reply(&inv, &own(), response, now).expect("reply");
        assert_eq!(reply.to, "bob@corp.example");
        assert_eq!(reply.subject, subject);
        assert!(reply.body_text.contains("Ann Smith has"));
        assert!(reply.body_text.contains("Quarterly planning"));
        let ics = String::from_utf8(reply.ics).expect("utf8");
        assert!(ics.contains("METHOD:REPLY\r\n"));
        assert!(ics.contains("UID:040000008200E00074C5B7101A82E00800000000AB@corp\r\n"));
        assert!(ics.contains("SEQUENCE:3\r\n"));
        assert!(ics.contains("DTSTAMP:20260111T093000Z\r\n"));
        assert!(ics.contains("ORGANIZER;CN=\"Bob Boss\":mailto:bob@corp.example\r\n"));
        assert!(ics.contains(&format!(
            "ATTENDEE;CN=\"Ann Smith\";PARTSTAT={stat}:mailto:ann@example.com\r\n"
        )));
        // Only the user's attendee line, no description or other attendees.
        assert_eq!(ics.matches("ATTENDEE").count(), 1);
        assert!(!ics.contains("cy@corp.example") && !ics.contains("Agenda"));
        // The output reads back as a REPLY from the user.
        let back = parse(ics.as_bytes()).expect("reads back");
        assert_eq!(back.method, ItipMethod::Reply);
        assert_eq!(back.sequence, 3);
        assert_eq!(back.attendees[0].person.partstat, inv_stat(response));
    }
}

fn inv_stat(response: Response) -> PartStat {
    response.partstat()
}

#[test]
fn reply_keeps_the_recurrence_id_and_refuses_strangers() {
    let inv = parsed(THUNDERBIRD_SERIES);
    // The master is the main component; build a per-instance REQUEST instead.
    assert!(inv.recurrence_id.is_none());
    assert_eq!(inv.event.overrides.len(), 1);
    let instance = parsed(
        &THUNDERBIRD_SERIES
            .replacen("BEGIN:VEVENT", "BEGIN:VEVENTX", 1)
            .replacen("END:VEVENT", "END:VEVENTX", 1),
    );
    // Without the master VEVENT only the override is left: an instance update.
    assert!(instance.recurrence_id.is_some());
    let reply = build_reply(
        &instance,
        &own(),
        Response::Accept,
        utc("2026-01-11T09:30:00Z"),
    )
    .expect("reply");
    let ics = String::from_utf8(reply.ics).expect("utf8");
    assert!(ics.contains("RECURRENCE-ID;TZID=Europe/Berlin:20260113T090000\r\n"));
    // Not listed → NotFound; no organizer → InvalidInput.
    assert_eq!(
        build_reply(
            &inv,
            &["z@example.com".to_owned()],
            Response::Accept,
            utc("2026-01-11T09:30:00Z")
        )
        .err(),
        Some(CalendarError::NotFound)
    );
    let no_org = google("REQUEST").replace("ORGANIZER;CN=Dana:mailto:dana@gmail.example\r\n", "");
    assert_eq!(
        build_reply(
            &parsed(&no_org),
            &own(),
            Response::Accept,
            utc("2026-01-11T09:30:00Z")
        )
        .err(),
        Some(CalendarError::InvalidInput)
    );
}

#[test]
fn reply_header_injection_through_the_title_is_flattened() {
    let text = google("REQUEST").replace("SUMMARY:Sync", "SUMMARY:Hi\\nBcc: evil@example.com");
    let inv = parsed(&text);
    let reply =
        build_reply(&inv, &own(), Response::Accept, utc("2026-01-11T09:30:00Z")).expect("reply");
    assert!(!reply.subject.contains('\n') && !reply.subject.contains('\r'));
    assert_eq!(reply.subject, "Accepted: Hi Bcc: evil@example.com");
}

#[test]
fn accepting_sets_the_users_status_and_keeps_existing_reminders() {
    let inv = parsed(&outlook(0, "20260110T080000Z", "REQUEST"));
    let event = event_to_store(&inv, None, &own(), Response::Tentative).expect("event");
    assert_eq!(event.uid, inv.uid);
    assert_eq!(event.attendees[0].partstat, PartStat::Tentative);
    assert_eq!(event.attendees[1].partstat, PartStat::Accepted);
    assert_eq!(event.alarms.len(), 1);
    let mut stored = event.clone();
    stored.alarms.clear();
    let again = event_to_store(&inv, Some(&stored), &own(), Response::Accept).expect("event");
    assert!(again.alarms.is_empty());
    assert_eq!(again.attendees[0].partstat, PartStat::Accepted);
}

#[test]
fn instance_update_merges_into_the_stored_series() {
    let series = parsed(THUNDERBIRD_SERIES);
    let stored = event_to_store(&series, None, &own(), Response::Accept).expect("stored");
    assert_eq!(stored.overrides.len(), 1);
    assert_eq!(
        stored.overrides[0].attendees[0].partstat,
        PartStat::Accepted
    );
    // A later instance-only REQUEST (here: the override alone) replaces it.
    let instance_text = THUNDERBIRD_SERIES
        .replacen("BEGIN:VEVENT", "BEGIN:VEVENTX", 1)
        .replacen("END:VEVENT", "END:VEVENTX", 1);
    let instance = parsed(&instance_text);
    let merged =
        event_to_store(&instance, Some(&stored), &own(), Response::Decline).expect("merged");
    assert_eq!(merged.overrides.len(), 1);
    assert_eq!(
        merged.overrides[0].attendees[0].partstat,
        PartStat::Declined
    );
    assert_eq!(merged.attendees[0].partstat, PartStat::Accepted);
    assert!(merged.rrule.is_some());
}

#[test]
fn cancel_plans_follow_mode_and_instance() {
    let series = parsed(THUNDERBIRD_SERIES);
    let stored = event_to_store(&series, None, &own(), Response::Accept).expect("stored");
    let cancel = parsed(THUNDERBIRD_CANCEL_INSTANCE);
    assert_eq!(cancel.method, ItipMethod::Cancel);
    assert!(!cancel.has_start);
    assert!(matches!(
        plan_cancel(&stored, &cancel, CancelMode::Remove),
        CancelPlan::DeleteInstance(_)
    ));
    let whole = parsed(&google("CANCEL"));
    let single =
        event_to_store(&parsed(&google("REQUEST")), None, &own(), Response::Accept).expect("e");
    assert_eq!(
        plan_cancel(&single, &whole, CancelMode::Remove),
        CancelPlan::DeleteEvent
    );
    match plan_cancel(&single, &whole, CancelMode::MarkCancelled) {
        CancelPlan::Store(event) => assert_eq!(event.status, crate::model::EventStatus::Cancelled),
        other => panic!("{other:?}"),
    }
    // An instance cancel for a non-repeating event does nothing.
    assert_eq!(
        plan_cancel(&single, &cancel, CancelMode::Remove),
        CancelPlan::Nothing
    );
}

#[test]
fn attendee_reply_updates_the_stored_attendee() {
    let reply = parsed(GOOGLE_REPLY);
    assert_eq!(reply.method, ItipMethod::Reply);
    assert!(!reply.has_start);
    let mut stored = parsed(&google("REQUEST")).event;
    stored.uid = "own-event@zephium".to_owned();
    stored.attendees.push(crate::model::Person {
        email: "zed@example.net".to_owned(),
        name: None,
        partstat: PartStat::NeedsAction,
    });
    assert_eq!(apply_reply(&mut stored, &reply), 1);
    assert_eq!(
        stored.attendees.last().expect("zed").partstat,
        PartStat::Accepted
    );
    // Applying the same reply again changes nothing; strangers are ignored.
    assert_eq!(apply_reply(&mut stored, &reply), 0);
    let mut other = parsed(&google("REQUEST")).event;
    other.attendees.clear();
    assert_eq!(apply_reply(&mut other, &reply), 0);
}

// -- service ------------------------------------------------------------------

fn put_requests(h: &super::service::Harness) -> Vec<crate::http::HttpRequest> {
    h.fake
        .requests()
        .into_iter()
        .filter(|r| r.method == Method::Put)
        .collect()
}

#[tokio::test]
async fn respond_stores_the_event_under_the_organizers_uid_and_returns_the_reply() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(201, &[("etag", "\"i1\"")], ""),
    );
    let inv = parsed(&outlook(0, "20260110T080000Z", "REQUEST"));

    let described = h.service.describe_invitation(&inv, &own());
    assert!(described.can_respond && described.existing.is_none());
    assert_eq!(described.suggested_calendar.as_deref(), Some(work.as_str()));
    assert_eq!(described.own_address.as_deref(), Some("ann@example.com"));
    assert!(described.attendees[0].is_you && !described.attendees[1].is_you);
    assert_eq!(described.start, "2026-01-20T09:00:00Z");

    let reply = h
        .service
        .respond_invitation(
            &inv,
            &own(),
            Response::Accept,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("respond");
    assert_eq!(reply.calendar_action, CalendarInvitationActionV1::Created);
    assert_eq!(reply.to, "bob@corp.example");
    assert_eq!(reply.subject, "Accepted: Quarterly planning");
    assert_eq!(
        reply.content_type,
        "text/calendar; method=REPLY; charset=UTF-8"
    );
    assert!(reply.ics.contains("PARTSTAT=ACCEPTED"));
    let puts = put_requests(&h);
    let body = String::from_utf8_lossy(&puts.last().expect("put").body).into_owned();
    assert!(body.contains("UID:040000008200E00074C5B7101A82E00800000000AB@corp"));
    assert!(body.contains("PARTSTAT=ACCEPTED"));
    assert!(body.contains("DTSTAMP:20260110T080000Z"));
    assert!(puts
        .last()
        .expect("put")
        .headers
        .contains(&("if-none-match", "*".to_owned())));

    // The stored copy is now found, same version, with the user's status.
    let again = h.service.describe_invitation(&inv, &own());
    let existing = again.existing.expect("existing");
    assert_eq!(existing.partstat.as_deref(), Some("accepted"));
    assert_eq!(existing.freshness, Freshness::Same);
    assert!(existing.same_organizer);

    // An update (higher SEQUENCE) replaces it, guarded by the ETag.
    let update = parsed(&outlook(1, "20260112T080000Z", "REQUEST"));
    assert_eq!(
        h.service
            .describe_invitation(&update, &own())
            .existing
            .expect("e")
            .freshness,
        Freshness::Newer
    );
    let reply = h
        .service
        .respond_invitation(
            &update,
            &own(),
            Response::Tentative,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("update");
    assert_eq!(reply.calendar_action, CalendarInvitationActionV1::Updated);
    let last = put_requests(&h).pop().expect("put");
    assert!(last.headers.contains(&("if-match", "\"i1\"".to_owned())));

    // The outdated message is refused and nothing is written.
    let before = put_requests(&h).len();
    let old = parsed(&outlook(0, "20260110T080000Z", "REQUEST"));
    assert_eq!(
        h.service
            .respond_invitation(
                &old,
                &own(),
                Response::Accept,
                Some(&work),
                DeclineMode::Remove
            )
            .await
            .err(),
        Some(CalendarError::Conflict)
    );
    assert_eq!(put_requests(&h).len(), before);
}

#[tokio::test]
async fn respond_refuses_a_stranger_and_a_changed_organizer_without_writing() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(201, &[("etag", "\"i1\"")], ""),
    );
    let inv = parsed(&google("REQUEST"));
    let stranger = vec!["nobody@example.com".to_owned()];
    assert_eq!(
        h.service
            .respond_invitation(
                &inv,
                &stranger,
                Response::Accept,
                Some(&work),
                DeclineMode::Remove
            )
            .await
            .err(),
        Some(CalendarError::NotFound)
    );
    assert!(put_requests(&h).is_empty());
    // Accept needs a calendar.
    assert_eq!(
        h.service
            .respond_invitation(&inv, &own(), Response::Accept, None, DeclineMode::Remove)
            .await
            .err(),
        Some(CalendarError::InvalidInput)
    );
    h.service
        .respond_invitation(
            &inv,
            &own(),
            Response::Accept,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("first");
    // The same UID from another organizer must not overwrite it.
    let forged = parsed(
        &google("REQUEST")
            .replace("dana@gmail.example\r\nUID", "mallory@evil.example\r\nUID")
            .replace("SEQUENCE:0", "SEQUENCE:9"),
    );
    assert_eq!(
        h.service
            .respond_invitation(
                &forged,
                &own(),
                Response::Accept,
                Some(&work),
                DeclineMode::Remove
            )
            .await
            .err(),
        Some(CalendarError::Forbidden)
    );
    let forged_cancel = parsed(
        &google("CANCEL").replace("dana@gmail.example\r\nUID", "mallory@evil.example\r\nUID"),
    );
    assert_eq!(
        h.service
            .apply_cancel(&forged_cancel, CancelMode::Remove)
            .await
            .err(),
        Some(CalendarError::NotFound)
    );
}

#[tokio::test]
async fn decline_removes_or_keeps_and_cancel_removes() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(201, &[("etag", "\"i1\"")], ""),
    );
    h.fake
        .route(Method::Delete, "*", "", response(204, &[], ""));
    let inv = parsed(&google("REQUEST"));

    // Declining something never accepted writes nothing but still replies.
    let reply = h
        .service
        .respond_invitation(
            &inv,
            &own(),
            Response::Decline,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("decline");
    assert_eq!(reply.calendar_action, CalendarInvitationActionV1::None);
    assert!(reply.subject.starts_with("Declined: "));
    assert!(put_requests(&h).is_empty());

    // Accept, then decline in "keep" mode: stored as declined.
    h.service
        .respond_invitation(
            &inv,
            &own(),
            Response::Accept,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("accept");
    let kept = h
        .service
        .respond_invitation(
            &inv,
            &own(),
            Response::Decline,
            Some(&work),
            DeclineMode::KeepDeclined,
        )
        .await
        .expect("keep");
    assert_eq!(kept.calendar_action, CalendarInvitationActionV1::Updated);
    let body = String::from_utf8_lossy(&put_requests(&h).pop().expect("put").body).into_owned();
    assert!(body.contains("PARTSTAT=DECLINED"));

    // Decline in "remove" mode deletes it.
    let removed = h
        .service
        .respond_invitation(
            &inv,
            &own(),
            Response::Decline,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("remove");
    assert_eq!(removed.calendar_action, CalendarInvitationActionV1::Removed);
    assert!(h.service.find_by_uid(&inv.uid).is_empty());

    // Accept again, then the organizer cancels it.
    h.service
        .respond_invitation(
            &inv,
            &own(),
            Response::Accept,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("accept");
    let cancel = parsed(&google("CANCEL").replace("SEQUENCE:0", "SEQUENCE:1"));
    let done = h
        .service
        .apply_cancel(&cancel, CancelMode::MarkCancelled)
        .await
        .expect("cancel");
    assert_eq!(done, CalendarInvitationActionV1::Cancelled);
    assert_eq!(
        h.service.find_by_uid(&inv.uid)[0].event.status,
        crate::model::EventStatus::Cancelled
    );
    let done = h
        .service
        .apply_cancel(&cancel, CancelMode::Remove)
        .await
        .expect("cancel");
    assert_eq!(done, CalendarInvitationActionV1::Removed);
    assert!(h.service.find_by_uid(&inv.uid).is_empty());
    assert_eq!(
        h.service
            .apply_cancel(&cancel, CancelMode::Remove)
            .await
            .err(),
        Some(CalendarError::NotFound)
    );
    // A reply is not a cancel.
    assert_eq!(
        h.service.apply_cancel(&inv, CancelMode::Remove).await.err(),
        Some(CalendarError::InvalidInput)
    );
}

#[tokio::test]
async fn replies_to_an_own_event_update_the_attendee() {
    let h = signed_in().await;
    h.service.sync_all().await;
    let work = h.service.calendars()[0].id.clone();
    h.fake.route(
        Method::Put,
        "*",
        "",
        response(201, &[("etag", "\"o1\"")], ""),
    );
    // The user's own event: create it through the invitation path with the
    // user as organizer.
    let mine = parsed(
        &google("REQUEST")
            .replace("dana@gmail.example\r\nUID:gcal123@google.com", "ann@example.com\r\nUID:own-event@zephium")
            .replace("ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE;CN=Dana;X-NUM-GUESTS=0:mailto:dana@gmail.example", "ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:zed@example.net"),
    );
    assert!(mine.is_organizer(&own()));
    h.service
        .respond_invitation(
            &mine,
            &own(),
            Response::Accept,
            Some(&work),
            DeclineMode::Remove,
        )
        .await
        .expect("store");
    let reply = parsed(GOOGLE_REPLY);
    let done = h.service.apply_reply(&reply, &own()).await.expect("apply");
    assert_eq!(done, CalendarInvitationActionV1::Updated);
    let stored = &h.service.find_by_uid("own-event@zephium")[0].event;
    let zed = stored
        .attendees
        .iter()
        .find(|a| a.email == "zed@example.net")
        .expect("zed");
    assert_eq!(zed.partstat, PartStat::Accepted);
    // Same reply twice: nothing to change. Someone else's event: not found.
    assert_eq!(
        h.service.apply_reply(&reply, &own()).await.expect("again"),
        CalendarInvitationActionV1::None
    );
    assert_eq!(
        h.service
            .apply_reply(&reply, &["bob@corp.example".to_owned()])
            .await
            .err(),
        Some(CalendarError::NotFound)
    );
}
