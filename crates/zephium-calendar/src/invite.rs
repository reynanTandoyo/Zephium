//! Meeting invitations (T-0045): reads an iTIP message (RFC 5546, carried by
//! e-mail as iMIP, RFC 6047), matches the user's attendee entry, compares it
//! with a stored copy, builds the `METHOD:REPLY` answer and plans the changes
//! to the calendar. Pure logic over [`crate::model`]; nothing here sends mail
//! or touches a server (see [`crate::service`] for the calendar side).

use crate::error::{CalendarError, Result};
use crate::ical::{self, AttendeeRole};
use crate::model::{Event, EventStatus, EventTime, PartStat, Person};
use crate::recur;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Largest iTIP object read; invitations are a few KiB, this is generous.
pub const MAX_INVITATION_BYTES: usize = 1024 * 1024;
/// Longest description kept on an invitation, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 4_000;
/// Most own addresses considered when matching an attendee.
pub const MAX_OWN_ADDRESSES: usize = 50;
/// Content type of a REPLY part.
pub const REPLY_CONTENT_TYPE: &str = "text/calendar; method=REPLY; charset=UTF-8";

const MAX_TITLE_IN_SUBJECT: usize = 200;

/// The iTIP METHOD of an invitation message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ItipMethod {
    /// An invitation or an update of one.
    Request,
    /// The organizer withdrew the event (or one instance).
    Cancel,
    /// An attendee's answer, read by the organizer.
    Reply,
    /// An attendee proposes another time. Read, never applied.
    Counter,
}

impl ItipMethod {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Cancel => "cancel",
            Self::Reply => "reply",
            Self::Counter => "counter",
        }
    }
}

/// An attendee with the attributes the event model does not keep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attendee {
    /// Address, name and status.
    pub person: Person,
    /// `ROLE`.
    pub role: AttendeeRole,
    /// `RSVP=TRUE`: the organizer asks for an answer.
    pub rsvp: bool,
}

/// A parsed iTIP object about one event (all VEVENTs share one UID).
#[derive(Clone, Debug, PartialEq)]
pub struct Invitation {
    /// METHOD.
    pub method: ItipMethod,
    /// UID.
    pub uid: String,
    /// SEQUENCE of the main component.
    pub sequence: u32,
    /// DTSTAMP of the main component.
    pub dtstamp: Option<DateTime<Utc>>,
    /// RECURRENCE-ID when the message is about one instance only.
    pub recurrence_id: Option<EventTime>,
    /// The event: the series master with its overrides, or, for an
    /// instance-only message, the single override. The description is capped.
    pub event: Event,
    /// The message carried a DTSTART (CANCEL and REPLY may omit it).
    pub has_start: bool,
    /// Attendees of the main component.
    pub attendees: Vec<Attendee>,
    /// The message text, so an accepted invitation is stored with every line
    /// the organizer sent, not only the modelled ones.
    pub source: std::sync::Arc<str>,
}

/// Lower-cases an address and strips `mailto:`.
pub fn normalize_address(address: &str) -> String {
    let trimmed = address.trim();
    let bare = trimmed
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("mailto:"))
        .map_or(trimmed, |_| &trimmed[7..]);
    bare.trim().to_lowercase()
}

fn own_set(own: &[String]) -> Vec<String> {
    own.iter()
        .take(MAX_OWN_ADDRESSES)
        .map(|address| normalize_address(address))
        .filter(|address| !address.is_empty())
        .collect()
}

fn cap_text(text: &mut String, limit: usize) {
    if let Some((index, _)) = text.char_indices().nth(limit) {
        text.truncate(index);
    }
}

/// Reads an iTIP message. Refuses input over [`MAX_INVITATION_BYTES`], a
/// missing or unsupported METHOD, and a REQUEST without DTSTART.
pub fn parse(bytes: &[u8]) -> Result<Invitation> {
    if bytes.len() > MAX_INVITATION_BYTES {
        return Err(CalendarError::TooLarge);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    let (method, components) = ical::parse_itip(&text)?;
    let method = match method.as_deref() {
        Some("REQUEST") => ItipMethod::Request,
        Some("CANCEL") => ItipMethod::Cancel,
        Some("REPLY") => ItipMethod::Reply,
        Some("COUNTER") => ItipMethod::Counter,
        _ => return Err(CalendarError::Unsupported),
    };
    let uid = components
        .first()
        .map(|component| component.event.uid.clone())
        .ok_or(CalendarError::Parse)?;
    let mut same: Vec<_> = components
        .into_iter()
        .filter(|component| component.event.uid == uid)
        .collect();
    let main_index = same
        .iter()
        .position(|component| component.event.recurrence_id.is_none())
        .unwrap_or(0);
    let main = same.remove(main_index);
    let has_master = main.event.recurrence_id.is_none();
    let mut event = main.event;
    if has_master {
        event.overrides = same
            .into_iter()
            .take(ical::MAX_LIST)
            .map(|component| component.event)
            .collect();
    }
    if method == ItipMethod::Request && !main.has_start {
        return Err(CalendarError::Parse);
    }
    cap_text(&mut event.description, MAX_DESCRIPTION_CHARS);
    for item in &mut event.overrides {
        cap_text(&mut item.description, MAX_DESCRIPTION_CHARS);
    }
    let attendees = event
        .attendees
        .iter()
        .cloned()
        .zip(main.attendee_meta)
        .map(|(person, (role, rsvp))| Attendee { person, role, rsvp })
        .collect();
    Ok(Invitation {
        method,
        uid,
        sequence: event.sequence,
        dtstamp: main.dtstamp,
        recurrence_id: event.recurrence_id.clone(),
        has_start: main.has_start,
        event,
        attendees,
        source: std::sync::Arc::from(&*text),
    })
}

impl Invitation {
    /// Index in [`Self::attendees`] of the attendee entry whose address is one
    /// of `own` (case-insensitive, `mailto:` ignored).
    pub fn own_attendee(&self, own: &[String]) -> Option<usize> {
        let own = own_set(own);
        self.attendees
            .iter()
            .position(|item| own.contains(&normalize_address(&item.person.email)))
    }

    /// Whether the message that carried this object may be believed: its From
    /// address(es) `from` must all be the ORGANIZER (REQUEST, CANCEL) or the
    /// attendee that answers (REPLY, COUNTER). A mail header is no proof, but a
    /// message whose sender is not who the object speaks for is certainly not
    /// what it claims. No sender, no organizer or no attendee never matches.
    pub fn sender_matches(&self, from: &[String]) -> bool {
        let senders: Vec<String> = from
            .iter()
            .take(MAX_OWN_ADDRESSES)
            .map(|address| normalize_address(address))
            .collect();
        if senders.is_empty() || senders.iter().any(String::is_empty) {
            return false;
        }
        let all_are = |wanted: &str| senders.iter().all(|sender| sender == wanted);
        match self.method {
            ItipMethod::Request | ItipMethod::Cancel => self
                .event
                .organizer
                .as_ref()
                .is_some_and(|organizer| all_are(&normalize_address(&organizer.email))),
            ItipMethod::Reply | ItipMethod::Counter => {
                !self.attendees.is_empty()
                    && self
                        .attendees
                        .iter()
                        .all(|item| all_are(&normalize_address(&item.person.email)))
            }
        }
    }

    /// `true` when the organizer is one of `own`.
    pub fn is_organizer(&self, own: &[String]) -> bool {
        organizer_is(&self.event, &own_set(own))
    }
}

fn organizer_is(event: &Event, own: &[String]) -> bool {
    event
        .organizer
        .as_ref()
        .is_some_and(|organizer| own.contains(&normalize_address(&organizer.email)))
}

/// `true` when both name the same organizer; an event without an organizer
/// matches nothing, so a message can never change an event the user made.
pub fn same_organizer(stored: &Event, invitation: &Invitation) -> bool {
    match (&stored.organizer, &invitation.event.organizer) {
        (Some(a), Some(b)) => normalize_address(&a.email) == normalize_address(&b.email),
        _ => false,
    }
}

/// How an invitation compares with a stored copy of the same event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// The invitation is an update of the stored copy.
    Newer,
    /// Same version.
    Same,
    /// The stored copy is newer; the invitation is outdated.
    Older,
}

impl Freshness {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Newer => "newer",
            Self::Same => "same",
            Self::Older => "older",
        }
    }
}

/// RFC 5546 §2.1.4: the higher SEQUENCE wins; at equal SEQUENCE the later
/// DTSTAMP does. Without both stamps equal sequences count as the same.
pub fn compare(
    sequence: u32,
    stamp: Option<DateTime<Utc>>,
    stored_sequence: u32,
    stored_stamp: Option<DateTime<Utc>>,
) -> Freshness {
    use std::cmp::Ordering;
    let order = sequence
        .cmp(&stored_sequence)
        .then_with(|| match (stamp, stored_stamp) {
            (Some(new), Some(old)) => new.cmp(&old),
            _ => Ordering::Equal,
        });
    match order {
        Ordering::Greater => Freshness::Newer,
        Ordering::Equal => Freshness::Same,
        Ordering::Less => Freshness::Older,
    }
}

/// The user's answer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Response {
    /// Accept.
    Accept,
    /// Tentative.
    Tentative,
    /// Decline.
    Decline,
}

impl Response {
    /// The PARTSTAT this answer sets.
    pub fn partstat(self) -> PartStat {
        match self {
            Self::Accept => PartStat::Accepted,
            Self::Tentative => PartStat::Tentative,
            Self::Decline => PartStat::Declined,
        }
    }
}

/// A `METHOD:REPLY` ready to be sent by the mail side. Never sent here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    /// The organizer's address.
    pub to: String,
    /// `Accepted: <title>`, `Tentative: <title>` or `Declined: <title>`.
    pub subject: String,
    /// Plain-text body for the message.
    pub body_text: String,
    /// The iCalendar object (UTF-8, CRLF lines), sent as
    /// [`REPLY_CONTENT_TYPE`].
    pub ics: Vec<u8>,
}

fn one_line(text: &str, limit: usize) -> String {
    let flat: String = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn when_text(event: &Event) -> String {
    if event.start.is_date() {
        event.start.local.format("%Y-%m-%d").to_string()
    } else {
        event
            .start
            .to_utc()
            .format("%Y-%m-%d %H:%M UTC")
            .to_string()
    }
}

/// Builds the REPLY to a REQUEST for the attendee entry matching `own`: the
/// same UID, SEQUENCE and RECURRENCE-ID, the ORGANIZER, only the user's
/// ATTENDEE with the new PARTSTAT, and `stamp` as DTSTAMP. Refuses a message
/// that is not a REQUEST, has no organizer, or does not list the user.
pub fn build_reply(
    invitation: &Invitation,
    own: &[String],
    response: Response,
    stamp: DateTime<Utc>,
) -> Result<Reply> {
    if invitation.method != ItipMethod::Request {
        return Err(CalendarError::InvalidInput);
    }
    let organizer = invitation
        .event
        .organizer
        .as_ref()
        .ok_or(CalendarError::InvalidInput)?;
    let index = invitation
        .own_attendee(own)
        .ok_or(CalendarError::NotFound)?;
    let mut me = invitation.attendees[index].person.clone();
    me.partstat = response.partstat();
    let title = one_line(&invitation.event.summary, MAX_TITLE_IN_SUBJECT);

    let mut out = String::new();
    let mut line = |text: &str| ical::fold(text, &mut out);
    line("BEGIN:VCALENDAR");
    line("PRODID:-//Zephium//Calendar//EN");
    line("VERSION:2.0");
    line("METHOD:REPLY");
    line("BEGIN:VEVENT");
    line(&format!("UID:{}", ical::escape(&invitation.uid)));
    if let Some(id) = &invitation.recurrence_id {
        line(&ical::time_property("RECURRENCE-ID", id));
    }
    line(&format!("SEQUENCE:{}", invitation.sequence));
    line(&format!("DTSTAMP:{}Z", stamp.format("%Y%m%dT%H%M%S")));
    line(&ical::person_property("ORGANIZER", organizer, false));
    line(&ical::person_property("ATTENDEE", &me, true));
    line(&format!("SUMMARY:{}", ical::escape(&title)));
    line("END:VEVENT");
    line("END:VCALENDAR");

    let (prefix, verb) = match response {
        Response::Accept => ("Accepted", "accepted"),
        Response::Tentative => ("Tentative", "tentatively accepted"),
        Response::Decline => ("Declined", "declined"),
    };
    let who = one_line(me.name.as_deref().unwrap_or(&me.email), 200);
    let mut body_text = format!("{who} has {verb} this meeting.\r\n\r\n{title}\r\n");
    body_text.push_str(&format!("When: {}\r\n", when_text(&invitation.event)));
    let location = one_line(&invitation.event.location, 300);
    if !location.is_empty() {
        body_text.push_str(&format!("Where: {location}\r\n"));
    }
    Ok(Reply {
        to: organizer.email.clone(),
        subject: format!("{prefix}: {title}"),
        body_text,
        ics: out.into_bytes(),
    })
}

fn set_partstat(event: &mut Event, own: &[String], stat: PartStat) -> bool {
    let mut changed = false;
    for attendee in &mut event.attendees {
        if own.contains(&normalize_address(&attendee.email)) {
            attendee.partstat = stat;
            changed = true;
        }
    }
    changed
}

fn same_instance(item: &Event, id: &EventTime) -> bool {
    item.recurrence_id
        .as_ref()
        .is_some_and(|existing| recur::same_time(existing, id))
}

/// The event to store when the user answers a REQUEST: the invitation's event
/// with the user's PARTSTAT set. `existing` is the stored copy, if any: its
/// reminders are kept, and an instance-only REQUEST is merged into it as an
/// override.
pub fn event_to_store(
    invitation: &Invitation,
    existing: Option<&Event>,
    own: &[String],
    response: Response,
) -> Result<Event> {
    if invitation.method != ItipMethod::Request || !invitation.has_start {
        return Err(CalendarError::InvalidInput);
    }
    let own = own_set(own);
    let stat = response.partstat();
    let mut event = invitation.event.clone();
    if let Some(id) = invitation.recurrence_id.clone() {
        // One instance of a series.
        set_partstat(&mut event, &own, stat);
        let Some(master) = existing.filter(|stored| stored.recurrence_id.is_none()) else {
            return Ok(event);
        };
        let mut merged = master.clone();
        merged.overrides.retain(|item| !same_instance(item, &id));
        if merged.overrides.len() >= ical::MAX_LIST {
            return Err(CalendarError::Limit);
        }
        merged.exdates.retain(|date| !recur::same_time(date, &id));
        merged.overrides.push(event);
        return Ok(merged);
    }
    set_partstat(&mut event, &own, stat);
    for item in &mut event.overrides {
        set_partstat(item, &own, stat);
    }
    if let Some(stored) = existing {
        event.alarms = stored.alarms.clone();
    }
    Ok(event)
}

/// What a CANCEL does to a stored event.
#[derive(Clone, Debug, PartialEq)]
pub enum CancelPlan {
    /// Delete the whole event.
    DeleteEvent,
    /// Delete one instance of a series.
    DeleteInstance(EventTime),
    /// Replace the stored event (marked cancelled).
    Store(Box<Event>),
    /// The message does not apply to the stored event.
    Nothing,
}

/// Whether a cancelled event is removed or kept with `STATUS:CANCELLED`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelMode {
    /// Delete it.
    Remove,
    /// Keep it, marked cancelled.
    MarkCancelled,
}

/// What a declined event does to an earlier accepted copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclineMode {
    /// Delete the copy.
    Remove,
    /// Keep it with the user's PARTSTAT set to declined.
    KeepDeclined,
}

/// Plans a CANCEL against the stored copy. The caller has already checked the
/// organizer ([`same_organizer`]) and freshness.
pub fn plan_cancel(stored: &Event, invitation: &Invitation, mode: CancelMode) -> CancelPlan {
    if let Some(id) = &invitation.recurrence_id {
        if stored.is_recurring() && recur::is_instance(stored, id) {
            return CancelPlan::DeleteInstance(id.clone());
        }
        return CancelPlan::Nothing;
    }
    match mode {
        CancelMode::Remove => CancelPlan::DeleteEvent,
        CancelMode::MarkCancelled => {
            let mut event = stored.clone();
            event.status = EventStatus::Cancelled;
            event.sequence = event.sequence.max(invitation.sequence);
            CancelPlan::Store(Box::new(event))
        }
    }
}

/// Applies an attendee's REPLY to `stored` (the user is the organizer):
/// every attendee of the REPLY that the event lists gets its new PARTSTAT.
/// Unknown addresses are ignored. A REPLY for one instance updates that
/// instance's override only. Returns how many attendees changed.
pub fn apply_reply(stored: &mut Event, reply: &Invitation) -> usize {
    let target: &mut Event = match &reply.recurrence_id {
        Some(id) => match stored
            .overrides
            .iter_mut()
            .find(|item| same_instance(item, id))
        {
            Some(item) => item,
            None => return 0,
        },
        None => stored,
    };
    let mut changed = 0;
    for attendee in &reply.attendees {
        let wanted = normalize_address(&attendee.person.email);
        for known in &mut target.attendees {
            if normalize_address(&known.email) == wanted
                && known.partstat != attendee.person.partstat
            {
                known.partstat = attendee.person.partstat;
                changed += 1;
            }
        }
    }
    changed
}
