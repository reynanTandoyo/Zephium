//! VTIMEZONE components for the zones an event uses, generated from the IANA
//! database, so servers and clients that do not know a `TZID` (Outlook,
//! Exchange, older CalDAV servers) still place the event correctly.
//!
//! The observances are derived from the transitions of one reference year.
//! A zone with exactly one daylight-saving pair gets the usual yearly rule
//! (`FREQ=YEARLY;BYMONTH=..;BYDAY=..`); any other zone gets a fixed offset
//! (no transitions) or one dated observance per transition.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::{OffsetComponents, OffsetName, Tz};

const MIN_YEAR: i32 = 1970;
const MAX_YEAR: i32 = 2100;

#[derive(Clone, Debug)]
struct Transition {
    /// Wall-clock time just before the change.
    local: NaiveDateTime,
    from: i32,
    to: i32,
    daylight: bool,
    abbreviation: String,
}

fn offset_seconds(zone: Tz, at: DateTime<Utc>) -> i32 {
    zone.offset_from_utc_datetime(&at.naive_utc())
        .fix()
        .local_minus_utc()
}

fn describe(zone: Tz, at: DateTime<Utc>) -> (bool, String) {
    let offset = zone.offset_from_utc_datetime(&at.naive_utc());
    let daylight = !offset.dst_offset().is_zero();
    let name = offset.abbreviation().map(str::to_owned).unwrap_or_default();
    (daylight, name)
}

fn transitions_in(zone: Tz, year: i32) -> Vec<Transition> {
    let Some(first) = NaiveDate::from_ymd_opt(year, 1, 1) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut day = first.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc();
    let end = day + Duration::days(366);
    while day < end && day.year() == year {
        let next = day + Duration::days(1);
        let (before, after) = (offset_seconds(zone, day), offset_seconds(zone, next));
        if before != after {
            // First minute of the day that already has the new offset.
            let (mut low, mut high) = (0i64, 1_440i64);
            while low + 1 < high {
                let mid = (low + high) / 2;
                if offset_seconds(zone, day + Duration::minutes(mid)) == before {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            let at = day + Duration::minutes(high);
            let (daylight, abbreviation) = describe(zone, at);
            out.push(Transition {
                local: (at + Duration::seconds(i64::from(before))).naive_utc(),
                from: before,
                to: after,
                daylight,
                abbreviation,
            });
        }
        day = next;
    }
    out
}

fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.abs();
    format!("{sign}{:02}{:02}", seconds / 3600, (seconds % 3600) / 60)
}

fn days_in_month(date: NaiveDate) -> u32 {
    let (year, month) = (date.year(), date.month());
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    next.and_then(|next| next.pred_opt())
        .map_or(30, |last| last.day())
}

fn yearly_rule(local: NaiveDateTime) -> String {
    let date = local.date();
    let weekday =
        ["MO", "TU", "WE", "TH", "FR", "SA", "SU"][date.weekday().num_days_from_monday() as usize];
    let nth = if date.day() + 7 > days_in_month(date) {
        -1
    } else {
        i64::from((date.day() - 1) / 7 + 1)
    };
    format!("FREQ=YEARLY;BYMONTH={};BYDAY={nth}{weekday}", date.month())
}

fn observance(out: &mut Vec<String>, transition: &Transition, rule: bool) {
    let kind = if transition.daylight {
        "DAYLIGHT"
    } else {
        "STANDARD"
    };
    out.push(format!("BEGIN:{kind}"));
    out.push(format!(
        "DTSTART:{}",
        transition.local.format("%Y%m%dT%H%M%S")
    ));
    out.push(format!("TZOFFSETFROM:{}", format_offset(transition.from)));
    out.push(format!("TZOFFSETTO:{}", format_offset(transition.to)));
    if !transition.abbreviation.is_empty() {
        out.push(format!("TZNAME:{}", transition.abbreviation));
    }
    if rule {
        out.push(format!("RRULE:{}", yearly_rule(transition.local)));
    }
    out.push(format!("END:{kind}"));
}

/// Most years a VTIMEZONE reaches back from the last year in use.
const MAX_SPAN_YEARS: i32 = 30;

fn same_pattern(a: &[Transition], b: &[Transition]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.daylight == y.daylight
                && x.from == y.from
                && x.to == y.to
                && x.local.time() == y.local.time()
                && yearly_rule(x.local) == yearly_rule(y.local)
        })
}

/// The lines (unfolded, without line ends) of the VTIMEZONE for `zone`,
/// anchored at `year` (clamped to 1970-2100).
pub fn component(zone: Tz, year: i32) -> Vec<String> {
    component_span(zone, year, year)
}

/// The VTIMEZONE for a zone used from `first` to `last` (the later of the
/// last year in use and the present; each clamped to 1970-2100, at most 30
/// years apart). The rule of `last` becomes a yearly rule that starts in the
/// earliest year it has held without change, so later instances keep the
/// current rule; years before that, back to `first`, get one dated observance
/// per transition, so a series that began before a rule change is placed
/// correctly too.
pub fn component_span(zone: Tz, first: i32, last: i32) -> Vec<String> {
    let last = last.clamp(MIN_YEAR, MAX_YEAR);
    let first = first.clamp(MIN_YEAR, last).max(last - MAX_SPAN_YEARS);
    let mut out = vec![
        "BEGIN:VTIMEZONE".to_owned(),
        format!("TZID:{}", zone.name()),
    ];
    let current = transitions_in(zone, last);
    match current.as_slice() {
        [a, b] if a.daylight != b.daylight => {
            let mut stable = last;
            while stable > first && same_pattern(&transitions_in(zone, stable - 1), &current) {
                stable -= 1;
            }
            for year in first..stable {
                for transition in transitions_in(zone, year) {
                    observance(&mut out, &transition, false);
                }
            }
            let anchored = transitions_in(zone, stable);
            if let [a, b] = anchored.as_slice() {
                observance(&mut out, a, true);
                observance(&mut out, b, true);
            }
        }
        _ => {
            let start = NaiveDate::from_ymd_opt(first, 1, 1)
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .unwrap_or_default();
            let at = start.and_utc();
            let offset = offset_seconds(zone, at);
            let (daylight, abbreviation) = describe(zone, at);
            // The offset in force when the span begins; changes follow dated.
            let dated: Vec<Transition> = (first..=last)
                .flat_map(|year| transitions_in(zone, year))
                .collect();
            if dated.is_empty() || current.is_empty() {
                observance(
                    &mut out,
                    &Transition {
                        local: start,
                        from: offset,
                        to: offset,
                        daylight,
                        abbreviation,
                    },
                    false,
                );
            }
            for transition in &dated {
                observance(&mut out, transition, false);
            }
        }
    }
    out.push("END:VTIMEZONE".to_owned());
    out
}
