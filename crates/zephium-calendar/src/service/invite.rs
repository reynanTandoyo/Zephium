//! The calendar side of meeting invitations (T-0045): finding the stored copy
//! of an invited event, describing an invitation for the UI, and applying a
//! response, cancellation or reply. Writing goes through CalDAV only: Google
//! and Microsoft accounts keep their own event ids and track invitations
//! themselves, so storing an invited event there is [`CalendarError::Unsupported`].

use super::{CachedResource, CalendarService};
use crate::api::CalendarAccountKindV1;
use crate::api::{format_time, CalendarEventRefV1, CalendarPersonV1};
use crate::caldav::CaldavClient;
use crate::error::{CalendarError, Result};
use crate::ical;
use crate::invite::{
    self, CancelMode, CancelPlan, DeclineMode, Freshness, Invitation, ItipMethod, Reply, Response,
    REPLY_CONTENT_TYPE,
};
use crate::model::{Event, TimeKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// The stored copy of an invited event.
#[derive(Clone, Debug)]
pub struct StoredInvitedEvent {
    /// Calendar holding it.
    pub calendar_id: String,
    /// The calendar cannot be changed.
    pub read_only: bool,
    /// The event (master with overrides).
    pub event: Event,
    /// DTSTAMP of the stored text, when known.
    pub dtstamp: Option<DateTime<Utc>>,
}

/// What an operation did to the calendar.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CalendarInvitationActionV1 {
    /// A new event was stored.
    Created,
    /// A stored event was changed.
    Updated,
    /// A stored event or instance was deleted.
    Removed,
    /// A stored event was kept and marked cancelled.
    Cancelled,
    /// Nothing changed.
    None,
}

/// The stored copy of an invitation's event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarInvitationExistingV1 {
    /// Calendar holding the event.
    pub calendar_id: String,
    /// That calendar cannot be changed.
    pub read_only: bool,
    /// `none`, `confirmed`, `tentative` or `cancelled`.
    pub status: String,
    /// The user's stored status, when the user is an attendee.
    pub partstat: Option<String>,
    /// `newer` (the message updates the stored copy), `same` or `older`.
    pub freshness: invite::Freshness,
    /// The stored event names the same organizer as the message.
    pub same_organizer: bool,
}

/// One attendee of an invitation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarInvitationAttendeeV1 {
    /// Address, name and `partstat` as on any event.
    pub person: CalendarPersonV1,
    /// `chair`, `required`, `optional` or `non_participant`.
    pub role: String,
    /// The organizer asks for an answer.
    pub rsvp: bool,
    /// This entry is the user's.
    pub is_you: bool,
}

/// An invitation as the Mail card shows it.
///
/// `start`/`end` use the formats of `CalendarOccurrenceV1`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarInvitationV1 {
    /// Opaque handle for the other `calendar_invitation_*` commands; set by
    /// the desktop.
    pub id: String,
    /// `request`, `cancel`, `reply` or `counter`.
    pub method: ItipMethod,
    /// Event UID.
    pub uid: String,
    /// SEQUENCE.
    pub sequence: u32,
    /// The instance this message is about, when only one.
    pub recurrence_id: Option<String>,
    /// Title.
    pub summary: String,
    /// Notes, at most 4000 characters.
    pub description: String,
    /// Place.
    pub location: String,
    /// Start.
    pub start: String,
    /// End.
    pub end: String,
    /// Whole-day event.
    pub all_day: bool,
    /// IANA zone, when the event has one.
    pub tzid: Option<String>,
    /// The event repeats.
    pub recurring: bool,
    /// Organizer.
    pub organizer: Option<CalendarPersonV1>,
    /// Attendees with their status.
    pub attendees: Vec<CalendarInvitationAttendeeV1>,
    /// The address the user was invited at, when listed.
    pub own_address: Option<String>,
    /// The user is the organizer (the message is about the user's own event).
    pub is_organizer: bool,
    /// The user can answer: a REQUEST that lists the user and has an organizer.
    pub can_respond: bool,
    /// The stored copy, when one exists.
    pub existing: Option<CalendarInvitationExistingV1>,
    /// A writable calendar to store the event in: the one holding the
    /// existing copy, else the first writable CalDAV calendar.
    pub suggested_calendar: Option<String>,
}

/// The REPLY for the mail side to send, and what the answer did to the
/// calendar. Send `ics` as `content_type`, to `to`; the mail side must not
/// send it without the user's action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
pub struct CalendarItipReplyV1 {
    /// The organizer's address.
    pub to: String,
    /// `Accepted: <title>`, `Tentative: <title>` or `Declined: <title>`.
    pub subject: String,
    /// Plain-text body.
    pub body_text: String,
    /// The `METHOD:REPLY` iCalendar text (UTF-8).
    pub ics: String,
    /// `text/calendar; method=REPLY; charset=UTF-8`.
    pub content_type: String,
    /// What the answer did to the calendar.
    pub calendar_action: CalendarInvitationActionV1,
}

impl CalendarItipReplyV1 {
    fn from_reply(reply: Reply, action: CalendarInvitationActionV1) -> Self {
        Self {
            to: reply.to,
            subject: reply.subject,
            body_text: reply.body_text,
            ics: String::from_utf8_lossy(&reply.ics).into_owned(),
            content_type: REPLY_CONTENT_TYPE.to_owned(),
            calendar_action: action,
        }
    }
}

fn person_wire(person: &crate::model::Person) -> CalendarPersonV1 {
    CalendarPersonV1 {
        email: person.email.clone(),
        name: person.name.clone(),
        partstat: person.partstat.as_str().to_owned(),
    }
}

impl CalendarService {
    fn writable_kind(&self, calendar_id: &str) -> Result<()> {
        let (_, account) = self.writable(calendar_id)?;
        if account.kind.is_oauth() {
            return Err(CalendarError::Unsupported);
        }
        Ok(())
    }

    /// Every stored copy of `uid` (a series master, or a lone instance), in
    /// any calendar.
    pub fn find_by_uid(&self, uid: &str) -> Vec<StoredInvitedEvent> {
        let inner = self.lock();
        let mut found = Vec::new();
        for calendar in &inner.state.calendars {
            let Some(cache) = inner.caches.get(&calendar.id) else {
                continue;
            };
            let read_only = calendar.read_only
                || inner.state.accounts.iter().any(|account| {
                    account.id == calendar.account_id
                        && account.kind == CalendarAccountKindV1::Subscription
                });
            for resource in cache.resources.values() {
                let Some(event) = resource
                    .events
                    .iter()
                    .find(|event| event.uid == uid && event.recurrence_id.is_none())
                    .or_else(|| resource.events.iter().find(|event| event.uid == uid))
                else {
                    continue;
                };
                found.push(StoredInvitedEvent {
                    calendar_id: calendar.id.clone(),
                    read_only,
                    event: event.clone(),
                    dtstamp: stored_stamp(resource, uid),
                });
            }
        }
        found
    }

    /// Describes `invitation` for the UI: the user's entry, the stored copy
    /// and a suggested calendar. `id` is left empty for the host to fill.
    pub fn describe_invitation(
        &self,
        invitation: &Invitation,
        own: &[String],
    ) -> CalendarInvitationV1 {
        let own_index = invitation.own_attendee(own);
        let stored = self.find_by_uid(&invitation.uid);
        let own_norm: Vec<String> = own.iter().map(|a| invite::normalize_address(a)).collect();
        let existing = stored
            .iter()
            .find(|item| !item.read_only)
            .or_else(|| stored.first())
            .map(|item| CalendarInvitationExistingV1 {
                calendar_id: item.calendar_id.clone(),
                read_only: item.read_only,
                status: item.event.status.as_str().to_owned(),
                partstat: item
                    .event
                    .attendees
                    .iter()
                    .find(|a| own_norm.contains(&invite::normalize_address(&a.email)))
                    .map(|a| a.partstat.as_str().to_owned()),
                freshness: invite::compare(
                    invitation.sequence,
                    invitation.dtstamp,
                    item.event.sequence,
                    item.dtstamp,
                ),
                same_organizer: invite::same_organizer(&item.event, invitation),
            });
        let suggested_calendar = stored
            .iter()
            .find(|item| !item.read_only && self.writable_kind(&item.calendar_id).is_ok())
            .map(|item| item.calendar_id.clone())
            .or_else(|| self.first_invitation_calendar());
        let event = &invitation.event;
        CalendarInvitationV1 {
            id: String::new(),
            method: invitation.method,
            uid: invitation.uid.clone(),
            sequence: invitation.sequence,
            recurrence_id: invitation.recurrence_id.as_ref().map(format_time),
            summary: event.summary.clone(),
            description: event.description.clone(),
            location: event.location.clone(),
            start: format_time(&event.start),
            end: format_time(&event.end),
            all_day: event.start.is_date(),
            tzid: match &event.start.kind {
                TimeKind::Zoned(zone) => Some(zone.name().to_owned()),
                _ => None,
            },
            recurring: event.is_recurring(),
            organizer: event.organizer.as_ref().map(person_wire),
            attendees: invitation
                .attendees
                .iter()
                .enumerate()
                .map(|(index, item)| CalendarInvitationAttendeeV1 {
                    person: person_wire(&item.person),
                    role: item.role.as_str().to_owned(),
                    rsvp: item.rsvp,
                    is_you: own_index == Some(index),
                })
                .collect(),
            own_address: own_index.map(|index| invitation.attendees[index].person.email.clone()),
            is_organizer: invitation.is_organizer(own),
            can_respond: invitation.method == ItipMethod::Request
                && event.organizer.is_some()
                && own_index.is_some(),
            existing,
            suggested_calendar,
        }
    }

    /// The first visible writable CalDAV calendar.
    fn first_invitation_calendar(&self) -> Option<String> {
        self.calendars()
            .into_iter()
            .filter(|calendar| calendar.visible && !calendar.read_only)
            .find(|calendar| self.writable_kind(&calendar.id).is_ok())
            .map(|calendar| calendar.id)
    }

    /// Writes `event` over the stored copy in `calendar_id`, or creates it
    /// under the organizer's UID. `stamp` becomes its DTSTAMP.
    ///
    /// An existing copy is patched, so the user's own properties stay. A new
    /// one is written from `message` (the invitation's text) when given, so
    /// the organizer's unmodelled properties stay too.
    async fn put_invited(
        &self,
        calendar_id: &str,
        event: &Event,
        stamp: DateTime<Utc>,
        message: Option<&str>,
    ) -> Result<(CalendarEventRefV1, bool)> {
        let (calendar, account) = self.writable(calendar_id)?;
        if account.kind.is_oauth() {
            return Err(CalendarError::Unsupported);
        }
        let existing = self.locate_any(calendar_id, &event.uid);
        let ics = match (&existing, message) {
            (Some((_, resource)), _) => {
                ical::serialize_event_over(event, stamp, resource.raw.as_deref())
            }
            (None, Some(message)) => ical::serialize_event_from_itip(event, stamp, message),
            (None, None) => ical::serialize_event(event, stamp),
        };
        let url = Url::parse(&calendar.url).map_err(|_| CalendarError::InvalidInput)?;
        let client = CaldavClient::new(self.http.as_ref(), self.credentials_for(&account)?);
        let (href, etag) = match &existing {
            Some((href, resource)) => {
                let href = Url::parse(href).map_err(|_| CalendarError::InvalidInput)?;
                client
                    .put_event(
                        &url,
                        &event.uid,
                        &ics,
                        Some((&href, resource.etag.as_deref())),
                    )
                    .await?
            }
            None => client.put_event(&url, &event.uid, &ics, None).await?,
        };
        self.store_resource(calendar_id, &href, etag, vec![event.clone()], ics);
        Ok((
            CalendarEventRefV1 {
                calendar_id: calendar_id.to_owned(),
                uid: event.uid.clone(),
            },
            existing.is_some(),
        ))
    }

    fn locate_any(&self, calendar_id: &str, uid: &str) -> Option<(String, CachedResource)> {
        let inner = self.lock();
        inner
            .caches
            .get(calendar_id)?
            .resources
            .iter()
            .find_map(|(href, resource)| {
                resource
                    .events
                    .iter()
                    .any(|event| event.uid == uid)
                    .then(|| (href.clone(), resource.clone()))
            })
    }

    /// Answers a REQUEST: builds the REPLY first (nothing is written when the
    /// user is not listed), then applies the answer to `calendar_id`.
    /// Accept/tentative store or update the event under the organizer's UID
    /// keeping the user's reminders; decline deletes an earlier copy or keeps
    /// it as declined (`decline`). An outdated REQUEST is refused with
    /// [`CalendarError::Conflict`]; a stored copy of another organizer with
    /// [`CalendarError::Forbidden`]. The REPLY is returned, never sent.
    pub async fn respond_invitation(
        &self,
        invitation: &Invitation,
        own: &[String],
        response: Response,
        calendar_id: Option<&str>,
        decline: DeclineMode,
    ) -> Result<CalendarItipReplyV1> {
        let reply = invite::build_reply(invitation, own, response, self.now())?;
        let stored = self.find_by_uid(&invitation.uid);
        let chosen = calendar_id.and_then(|id| stored.iter().find(|item| item.calendar_id == id));
        if let Some(item) = chosen {
            if !invite::same_organizer(&item.event, invitation) {
                return Err(CalendarError::Forbidden);
            }
            if invite::compare(
                invitation.sequence,
                invitation.dtstamp,
                item.event.sequence,
                item.dtstamp,
            ) == Freshness::Older
            {
                return Err(CalendarError::Conflict);
            }
        }
        let action = if response == Response::Decline {
            self.apply_decline(invitation, own, chosen, decline).await?
        } else {
            let target = calendar_id.ok_or(CalendarError::InvalidInput)?;
            self.writable_kind(target)?;
            let event =
                invite::event_to_store(invitation, chosen.map(|item| &item.event), own, response)?;
            let stamp = invitation.dtstamp.unwrap_or_else(|| self.now());
            let (_, existed) = self
                .put_invited(target, &event, stamp, Some(&*invitation.source))
                .await?;
            if existed {
                CalendarInvitationActionV1::Updated
            } else {
                CalendarInvitationActionV1::Created
            }
        };
        Ok(CalendarItipReplyV1::from_reply(reply, action))
    }

    async fn apply_decline(
        &self,
        invitation: &Invitation,
        own: &[String],
        chosen: Option<&StoredInvitedEvent>,
        mode: DeclineMode,
    ) -> Result<CalendarInvitationActionV1> {
        let Some(item) = chosen else {
            return Ok(CalendarInvitationActionV1::None);
        };
        match (mode, &invitation.recurrence_id) {
            (DeclineMode::Remove, Some(id)) if item.event.is_recurring() => {
                self.delete_occurrence(&item.calendar_id, &invitation.uid, &format_time(id))
                    .await?;
                Ok(CalendarInvitationActionV1::Removed)
            }
            (DeclineMode::Remove, _) => {
                self.delete_event(&item.calendar_id, &invitation.uid)
                    .await?;
                Ok(CalendarInvitationActionV1::Removed)
            }
            (DeclineMode::KeepDeclined, _) => {
                let event =
                    invite::event_to_store(invitation, Some(&item.event), own, Response::Decline)?;
                let stamp = invitation.dtstamp.unwrap_or_else(|| self.now());
                self.put_invited(&item.calendar_id, &event, stamp, None)
                    .await?;
                Ok(CalendarInvitationActionV1::Updated)
            }
        }
    }

    /// Applies a CANCEL to every writable stored copy from the same organizer
    /// (the message is ignored for other organizers and for events without
    /// one). A CANCEL older than a stored copy is [`CalendarError::Conflict`].
    pub async fn apply_cancel(
        &self,
        invitation: &Invitation,
        mode: CancelMode,
    ) -> Result<CalendarInvitationActionV1> {
        if invitation.method != ItipMethod::Cancel {
            return Err(CalendarError::InvalidInput);
        }
        let stored: Vec<_> = self
            .find_by_uid(&invitation.uid)
            .into_iter()
            .filter(|item| !item.read_only && invite::same_organizer(&item.event, invitation))
            .collect();
        if stored.is_empty() {
            return Err(CalendarError::NotFound);
        }
        if stored.iter().any(|item| {
            invite::compare(
                invitation.sequence,
                invitation.dtstamp,
                item.event.sequence,
                item.dtstamp,
            ) == Freshness::Older
        }) {
            return Err(CalendarError::Conflict);
        }
        let mut result = CalendarInvitationActionV1::None;
        for item in stored {
            match invite::plan_cancel(&item.event, invitation, mode) {
                CancelPlan::DeleteEvent => {
                    self.delete_event(&item.calendar_id, &invitation.uid)
                        .await?;
                    result = CalendarInvitationActionV1::Removed;
                }
                CancelPlan::DeleteInstance(id) => {
                    self.delete_occurrence(&item.calendar_id, &invitation.uid, &format_time(&id))
                        .await?;
                    result = CalendarInvitationActionV1::Removed;
                }
                CancelPlan::Store(event) => {
                    let event = *event;
                    let stamp = invitation.dtstamp.unwrap_or_else(|| self.now());
                    self.put_invited(&item.calendar_id, &event, stamp, None)
                        .await?;
                    result = CalendarInvitationActionV1::Cancelled;
                }
                CancelPlan::Nothing => {}
            }
        }
        Ok(result)
    }

    /// Applies an attendee's REPLY to the stored event the user organizes
    /// (the stored organizer must be one of `own`). Unknown attendees are
    /// ignored; [`CalendarInvitationActionV1::None`] means nothing changed.
    pub async fn apply_reply(
        &self,
        reply: &Invitation,
        own: &[String],
    ) -> Result<CalendarInvitationActionV1> {
        if reply.method != ItipMethod::Reply {
            return Err(CalendarError::InvalidInput);
        }
        let own_norm: Vec<String> = own.iter().map(|a| invite::normalize_address(a)).collect();
        let stored: Vec<_> =
            self.find_by_uid(&reply.uid)
                .into_iter()
                .filter(|item| {
                    !item.read_only
                        && item.event.organizer.as_ref().is_some_and(|o| {
                            own_norm.contains(&invite::normalize_address(&o.email))
                        })
                })
                .collect();
        if stored.is_empty() {
            return Err(CalendarError::NotFound);
        }
        let mut result = CalendarInvitationActionV1::None;
        for item in stored {
            let mut event = item.event.clone();
            if invite::apply_reply(&mut event, reply) > 0 {
                self.put_invited(&item.calendar_id, &event, self.now(), None)
                    .await?;
                result = CalendarInvitationActionV1::Updated;
            }
        }
        Ok(result)
    }
}

/// DTSTAMP of the VEVENT of `uid` in a stored resource's text.
fn stored_stamp(resource: &CachedResource, uid: &str) -> Option<DateTime<Utc>> {
    let (_, components) = ical::parse_itip(resource.raw.as_deref()?).ok()?;
    components
        .iter()
        .find(|component| component.event.uid == uid && component.event.recurrence_id.is_none())
        .or_else(|| {
            components
                .iter()
                .find(|component| component.event.uid == uid)
        })
        .and_then(|component| component.dtstamp)
}
