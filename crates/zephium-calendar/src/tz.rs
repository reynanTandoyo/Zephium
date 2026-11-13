//! Time-zone names as calendar servers send them: IANA names, Windows names
//! (Outlook/Exchange) and the `/vendor/Region/City` form.

use chrono_tz::Tz;

const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("UTC", "UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("Romance Standard Time", "Europe/Paris"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("SA Pacific Standard Time", "America/Bogota"),
];

/// Resolves a `TZID` to an IANA zone, or `None` when it is unknown (the
/// caller then treats the time as floating).
pub fn resolve(tzid: &str) -> Option<Tz> {
    let tzid = tzid.trim().trim_matches('"');
    if tzid.is_empty() || tzid.len() > 128 {
        return None;
    }
    if let Ok(zone) = tzid.parse::<Tz>() {
        return Some(zone);
    }
    if let Some((_, iana)) = WINDOWS_ZONES
        .iter()
        .find(|(windows, _)| windows.eq_ignore_ascii_case(tzid))
    {
        return iana.parse().ok();
    }
    // "/freeassociation.sourceforge.net/Europe/Berlin" and similar.
    let parts: Vec<&str> = tzid.split('/').filter(|part| !part.is_empty()).collect();
    for width in [2usize, 1] {
        if parts.len() >= width {
            let tail = parts[parts.len() - width..].join("/");
            if let Ok(zone) = tail.parse::<Tz>() {
                return Some(zone);
            }
        }
    }
    None
}
