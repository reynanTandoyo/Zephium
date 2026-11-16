//! Calendar reminders: when an alarm falls due, what is missed while the app
//! was asleep, and what is already shown. Pure: the caller supplies the clock,
//! the expanded occurrences and the record of reminders already handled, so
//! the desktop scheduler can be driven with a fake clock.
//!
//! Only `DISPLAY` and `AUDIO` alarms make a reminder. A timed event with no
//! such alarm gets the person's default reminder, when one is chosen. A
//! declined event (the account's own attendee status is `DECLINED`) gets none.

use crate::model::{EventTime, PartStat};
use crate::recur::Occurrence;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How far either side of now the reminders are looked at.
pub const HORIZON_HOURS: i64 = 24;
/// A reminder older than this is missed, not shown.
pub const MISSED_AFTER_MINUTES: i64 = 10;
/// Most reminders one plan may show at once; the rest count as missed.
pub const MAX_DUE_PER_PASS: usize = 5;
/// Most reminders one scan returns.
pub const MAX_REMINDERS: usize = 200;
/// Most occurrences one scan expands.
pub const MAX_SCANNED_OCCURRENCES: usize = 2_000;
/// The default reminders a person may choose, in minutes before the start.
pub const DEFAULT_CHOICES: [i64; 4] = [5, 10, 15, 30];

/// One reminder that is due at `trigger`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    /// `calendar|<calendar id>|<uid>|<recurrence id or ->|<trigger>`; one
    /// reminder is shown at most once under this key.
    pub key: String,
    /// SUMMARY, possibly empty.
    pub title: String,
    /// Start of the occurrence.
    pub start: DateTime<Utc>,
    /// Whether the occurrence is a whole day.
    pub all_day: bool,
    /// LOCATION, possibly empty.
    pub location: String,
    /// When the reminder is due.
    pub trigger: DateTime<Utc>,
}

/// What one pass decided.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Reminders to show now, earliest first; at most [`MAX_DUE_PER_PASS`].
    pub due: Vec<Reminder>,
    /// Reminders that came due since the last pass but are too old to show.
    pub missed: usize,
    /// Keys of every reminder this pass consumed (shown or missed). Record
    /// them so the next pass does not count them again.
    pub handled: Vec<String>,
    /// The next reminder still in the future, if any.
    pub next: Option<DateTime<Utc>>,
}

/// Midnight of `date` in the viewer's local zone, as an instant. Falls back to
/// UTC midnight when local midnight does not exist.
pub fn day_start(date: NaiveDate) -> DateTime<Utc> {
    let midnight = date.and_time(NaiveTime::MIN);
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map_or_else(|| midnight.and_utc(), |local| local.with_timezone(&Utc))
}

/// The instant an occurrence time denotes. A whole-day value is the viewer's
/// local midnight of that day, so an all-day reminder fires on the local day.
fn anchor(time: &EventTime) -> DateTime<Utc> {
    if time.is_date() {
        day_start(time.local.date())
    } else {
        time.to_utc()
    }
}

/// The reminder instants one occurrence produces: one per `DISPLAY` or
/// `AUDIO` alarm, relative to the start or the end. With no such alarm, the
/// default reminder (`default_minutes`) applies to timed occurrences only.
/// Sorted and without repeats.
pub fn occurrence_triggers(
    occurrence: &Occurrence<'_>,
    default_minutes: Option<i64>,
) -> Vec<DateTime<Utc>> {
    let mut out: Vec<DateTime<Utc>> = occurrence
        .event
        .alarms
        .iter()
        .filter(|alarm| matches!(alarm.action.as_str(), "DISPLAY" | "AUDIO"))
        .filter_map(|alarm| {
            let base = if alarm.from_end {
                anchor(&occurrence.end)
            } else {
                anchor(&occurrence.start)
            };
            // An offset that leaves the range of dates makes no reminder.
            base.checked_sub_signed(Duration::try_minutes(alarm.minutes_before)?)
        })
        .collect();
    if out.is_empty() && !occurrence.start.is_date() {
        if let Some(trigger) = default_minutes
            .and_then(Duration::try_minutes)
            .and_then(|before| anchor(&occurrence.start).checked_sub_signed(before))
        {
            out.push(trigger);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `true` when the account's own attendee entry (matched by address) declined.
pub fn declined_by(attendees: &[crate::model::Person], account_address: Option<&str>) -> bool {
    let Some(address) = account_address else {
        return false;
    };
    attendees.iter().any(|person| {
        person.partstat == PartStat::Declined && person.email.eq_ignore_ascii_case(address)
    })
}

/// A short start label for a toast: `HH:MM` on the local day of `now`, else
/// `YYYY-MM-DD HH:MM`.
pub fn start_label(start: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let local = start.with_timezone(&Local);
    if local.date_naive() == now.with_timezone(&Local).date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// The key of one reminder; see [`Reminder::key`].
pub fn reminder_key(
    calendar_id: &str,
    uid: &str,
    recurrence: Option<&EventTime>,
    trigger: DateTime<Utc>,
) -> String {
    let recurrence = recurrence.map_or_else(
        || "-".to_owned(),
        |id| id.to_utc().to_rfc3339_opts(SecondsFormat::Secs, true),
    );
    format!(
        "calendar|{calendar_id}|{uid}|{recurrence}|{}",
        trigger.to_rfc3339_opts(SecondsFormat::Secs, true)
    )
}

/// Decides one pass at `now`. `since` is when the previous pass ran (the
/// first pass passes `now`): a reminder that came due after `since` but more
/// than [`MISSED_AFTER_MINUTES`] ago is missed, and one that came due before
/// `since` was handled long ago and is ignored. `fired` holds the keys already
/// shown or counted.
pub fn plan(
    now: DateTime<Utc>,
    since: DateTime<Utc>,
    fired: &HashSet<String>,
    reminders: &[Reminder],
) -> Plan {
    let grace = now - Duration::minutes(MISSED_AFTER_MINUTES);
    let mut out = Plan::default();
    let mut due: Vec<&Reminder> = Vec::new();
    let mut missed = 0_usize;
    for reminder in reminders {
        if fired.contains(&reminder.key) {
            continue;
        }
        if reminder.trigger > now {
            out.next = Some(
                out.next
                    .map_or(reminder.trigger, |next| next.min(reminder.trigger)),
            );
        } else if reminder.trigger < grace {
            if reminder.trigger > since {
                missed += 1;
                out.handled.push(reminder.key.clone());
            }
        } else {
            due.push(reminder);
        }
    }
    due.sort_by(|a, b| a.trigger.cmp(&b.trigger).then_with(|| a.key.cmp(&b.key)));
    for reminder in due {
        if out.due.len() < MAX_DUE_PER_PASS {
            out.due.push(reminder.clone());
            out.handled.push(reminder.key.clone());
        } else {
            missed += 1;
            out.handled.push(reminder.key.clone());
        }
    }
    out.missed = missed;
    out
}

/// The longest a scheduler sleeps before it looks again, so a wake from sleep
/// or a changed setting is noticed within a minute.
pub const MAX_WAIT_SECONDS: i64 = 60;
/// Most fired keys kept; older ones are dropped first.
pub const MAX_FIRED: usize = 500;
/// The ledger is written when a reminder was handled or this long has passed
/// since the last write, so the "since" mark survives a restart.
pub const SAVE_EVERY_MINUTES: i64 = 5;

#[derive(Serialize, Deserialize, Default)]
struct Wire {
    since: Option<i64>,
    #[serde(default)]
    fired: Vec<(String, i64)>,
}

/// What the scheduler remembers between passes: when the last pass ran and
/// which reminders were already shown or counted. Persisted as a small JSON
/// text so a restart does not show them again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ledger {
    since: Option<DateTime<Utc>>,
    fired: Vec<(String, DateTime<Utc>)>,
    saved_at: Option<DateTime<Utc>>,
}

impl Ledger {
    /// Reads the saved text; anything unreadable starts an empty ledger.
    pub fn load(text: Option<&str>) -> Self {
        let Some(wire) = text.and_then(|text| serde_json::from_str::<Wire>(text).ok()) else {
            return Self::default();
        };
        let at = |secs: i64| DateTime::from_timestamp(secs, 0);
        let mut fired: Vec<(String, DateTime<Utc>)> = wire
            .fired
            .into_iter()
            .filter(|(key, _)| key.len() <= 512)
            .filter_map(|(key, secs)| at(secs).map(|when| (key, when)))
            .collect();
        fired.sort_by_key(|(_, when)| *when);
        let excess = fired.len().saturating_sub(MAX_FIRED);
        fired.drain(..excess);
        Self {
            since: wire.since.and_then(at),
            fired,
            saved_at: None,
        }
    }

    /// The text to save, when a save is due: a reminder was handled, or the
    /// last save is [`SAVE_EVERY_MINUTES`] old. Records the save time.
    pub fn save_if_due(&mut self, now: DateTime<Utc>, plan: &Plan) -> Option<String> {
        let stale = self
            .saved_at
            .is_none_or(|at| now - at >= Duration::minutes(SAVE_EVERY_MINUTES));
        if plan.handled.is_empty() && !stale {
            return None;
        }
        self.saved_at = Some(now);
        let wire = Wire {
            since: self.since.map(|at| at.timestamp()),
            fired: self
                .fired
                .iter()
                .map(|(key, when)| (key.clone(), when.timestamp()))
                .collect(),
        };
        serde_json::to_string(&wire).ok()
    }

    /// Runs one pass at `now` over the reminders found now. Reminders that are
    /// shown or counted are recorded; the pass becomes the new "since" mark.
    /// The mark never reaches back more than [`HORIZON_HOURS`].
    pub fn pass(&mut self, now: DateTime<Utc>, reminders: &[Reminder]) -> Plan {
        let horizon = now - Duration::hours(HORIZON_HOURS);
        let since = self.since.map_or(now, |at| at.max(horizon));
        let fired: HashSet<String> = self.fired.iter().map(|(key, _)| key.clone()).collect();
        let plan = plan(now, since, &fired, reminders);
        for reminder in plan.due.iter() {
            self.fired.push((reminder.key.clone(), reminder.trigger));
        }
        for key in &plan.handled {
            if !self.fired.iter().any(|(known, _)| known == key) {
                let trigger = reminders
                    .iter()
                    .find(|reminder| &reminder.key == key)
                    .map_or(now, |reminder| reminder.trigger);
                self.fired.push((key.clone(), trigger));
            }
        }
        self.fired
            .retain(|(_, when)| *when >= now - Duration::hours(2 * HORIZON_HOURS));
        self.fired.sort_by_key(|(_, when)| *when);
        let excess = self.fired.len().saturating_sub(MAX_FIRED);
        self.fired.drain(..excess);
        self.since = Some(now);
        plan
    }

    /// Keys currently remembered as shown or counted.
    pub fn fired_count(&self) -> usize {
        self.fired.len()
    }

    /// How long a scheduler sleeps after `plan`: until the next reminder, but
    /// never more than [`MAX_WAIT_SECONDS`] and never less than a second.
    pub fn wait_after(plan: &Plan, now: DateTime<Utc>) -> std::time::Duration {
        let cap = std::time::Duration::from_secs(MAX_WAIT_SECONDS.unsigned_abs());
        let floor = std::time::Duration::from_secs(1);
        plan.next
            .and_then(|next| (next - now).to_std().ok())
            .map_or(cap, |until| until.clamp(floor, cap))
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_key_names_calendar_event_occurrence_and_trigger() {
        let trigger = Utc.with_ymd_and_hms(2026, 11, 13, 9, 45, 0).unwrap();
        assert_eq!(
            reminder_key("cal", "uid@x", None, trigger),
            "calendar|cal|uid@x|-|2026-11-13T09:45:00Z"
        );
    }

    #[test]
    fn a_plan_with_nothing_due_names_the_next_trigger() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap();
        let later = now + Duration::minutes(30);
        let reminder = Reminder {
            key: "k".into(),
            title: "Standup".into(),
            start: later,
            all_day: false,
            location: String::new(),
            trigger: later,
        };
        let result = plan(now, now, &HashSet::new(), std::slice::from_ref(&reminder));
        assert!(result.due.is_empty());
        assert_eq!(result.next, Some(later));
        assert!(result.handled.is_empty());
    }

    #[test]
    fn a_label_is_clock_only_on_the_same_local_day() {
        let now = Utc::now();
        let label = start_label(now, now);
        assert_eq!(label.len(), 5);
        assert_eq!(label.as_bytes()[2], b':');
    }
}
