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
    ("Central America Standard Time", "America/Guatemala"),
    ("Mexico Standard Time", "America/Mexico_City"),
    ("Canada Central Standard Time", "America/Regina"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Greenland Standard Time", "America/Godthab"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("Cape Verde Standard Time", "Atlantic/Cape_Verde"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Namibia Standard Time", "Africa/Windhoek"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("Nepal Standard Time", "Asia/Katmandu"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("Myanmar Standard Time", "Asia/Rangoon"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("West Pacific Standard Time", "Pacific/Port_Moresby"),
    ("Fiji Standard Time", "Pacific/Fiji"),
    ("Samoa Standard Time", "Pacific/Apia"),
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("Belarus Standard Time", "Europe/Minsk"),
    ("Kaliningrad Standard Time", "Europe/Kaliningrad"),
    ("Central Asia Standard Time", "Asia/Almaty"),
    ("N. Central Asia Standard Time", "Asia/Novosibirsk"),
    ("North Asia Standard Time", "Asia/Krasnoyarsk"),
    ("Yakutsk Standard Time", "Asia/Yakutsk"),
    ("Vladivostok Standard Time", "Asia/Vladivostok"),
];

/// A name like `(UTC+01:00) Amsterdam, Berlin` or `UTC-05:00` or `GMT+1`
/// (a custom zone some clients write): a whole-hour offset becomes the
/// `Etc/GMT` zone with the opposite sign. Half-hour offsets stay unknown.
fn fixed_offset_zone(tzid: &str) -> Option<Tz> {
    let upper = tzid.to_ascii_uppercase();
    let start = upper.find("UTC").or_else(|| upper.find("GMT"))? + 3;
    let rest = &upper[start..];
    let sign = match rest.chars().next()? {
        '+' => -1,
        '-' => 1,
        _ => return None,
    };
    let digits: String = rest[1..].chars().take_while(char::is_ascii_digit).collect();
    let after = &rest[1 + digits.len()..];
    let (hours, minutes) = match (digits.len(), after.strip_prefix(':')) {
        (1 | 2, Some(minutes)) => (digits.parse::<i32>().ok()?, minutes.get(..2)?),
        (1 | 2, None) => (digits.parse::<i32>().ok()?, "00"),
        (4, _) => (digits[..2].parse().ok()?, &digits[2..]),
        _ => return None,
    };
    if minutes != "00" || !(0..=12).contains(&hours) {
        return None;
    }
    let name = if hours == 0 {
        "Etc/GMT".to_owned()
    } else {
        format!("Etc/GMT{:+}", sign * hours)
    };
    name.parse().ok()
}

/// Resolves a `TZID` to an IANA zone, or `None` when it is unknown (the
/// caller then reads the time as floating, that is, in the viewer's local
/// zone). Tried in order: IANA name, Windows name, `/vendor/Region/City`, a
/// whole-hour `UTC+hh:mm` name.
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
    fixed_offset_zone(tzid)
}
