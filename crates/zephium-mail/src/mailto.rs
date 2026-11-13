//! Fork-owned (T-0036): `mailto:` links per RFC 6068. A link becomes a prefilled
//! compose and nothing more: only `to`, `cc`, `bcc`, `subject` and `body` are
//! read (`from`, `attachment`, `in-reply-to` and any other hfield are
//! ignored). A link that is too long, has an address that is not a plain
//! `addr-spec`, or smuggles a line break into a header field is refused whole,
//! so a page can never start a message that differs from what the person sees.

use crate::error::{MailError, Result};
use crate::limits::MAX_SUBJECT_BYTES;
use crate::model::email_ok;

/// Bytes of the whole link.
pub const MAX_MAILTO_BYTES: usize = 16 * 1024;
/// Addresses accepted per field.
pub const MAX_MAILTO_RECIPIENTS: usize = 50;
/// Bytes of the decoded body.
pub const MAX_MAILTO_BODY_BYTES: usize = 8 * 1024;

/// A parsed link.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Mailto {
    /// To addresses (the path and every `to` field).
    pub to: Vec<String>,
    /// Cc addresses.
    pub cc: Vec<String>,
    /// Bcc addresses.
    pub bcc: Vec<String>,
    /// Subject, one line.
    pub subject: String,
    /// Body with `\n` line breaks.
    pub body: String,
}

/// True when the text starts like a `mailto:` link (any case).
pub fn is_mailto(link: &str) -> bool {
    link.get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes into UTF-8. A bad escape or invalid UTF-8 is an error.
fn decode(text: &str) -> Result<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let high = bytes.get(at + 1).copied().and_then(hex);
            let low = bytes.get(at + 2).copied().and_then(hex);
            match (high, low) {
                (Some(high), Some(low)) => out.push(high << 4 | low),
                _ => return Err(MailError::InvalidInput),
            }
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).map_err(|_| MailError::InvalidInput)
}

/// Splits a decoded address list; every entry must be a plain address.
fn addresses(decoded: &str, into: &mut Vec<String>) -> Result<()> {
    for part in decoded.split(',') {
        let address = part.trim();
        if address.is_empty() {
            continue;
        }
        if address.chars().any(|c| c.is_control() || c.is_whitespace())
            || address.contains(['<', '>', '"', ';'])
            || !email_ok(address)
        {
            return Err(MailError::InvalidInput);
        }
        if !into.iter().any(|known| known.eq_ignore_ascii_case(address)) {
            into.push(address.to_owned());
        }
        if into.len() > MAX_MAILTO_RECIPIENTS {
            return Err(MailError::TooLarge);
        }
    }
    Ok(())
}

/// Parses a `mailto:` link.
pub fn parse(link: &str) -> Result<Mailto> {
    if link.len() > MAX_MAILTO_BYTES {
        return Err(MailError::TooLarge);
    }
    if !is_mailto(link) {
        return Err(MailError::InvalidInput);
    }
    let rest = &link[7..];
    // `mailto://host` is a URL-shaped lookalike, not RFC 6068.
    if rest.starts_with("//") {
        return Err(MailError::InvalidInput);
    }
    let rest = rest.split('#').next().unwrap_or("");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut mailto = Mailto::default();
    addresses(&decode(path)?, &mut mailto.to)?;
    let mut subject_seen = false;
    let mut body_seen = false;
    for field in query.split('&').filter(|field| !field.is_empty()) {
        let (name, value) = field.split_once('=').unwrap_or((field, ""));
        let name = decode(name)?.to_ascii_lowercase();
        match name.as_str() {
            "to" => addresses(&decode(value)?, &mut mailto.to)?,
            "cc" => addresses(&decode(value)?, &mut mailto.cc)?,
            "bcc" => addresses(&decode(value)?, &mut mailto.bcc)?,
            "subject" if !subject_seen => {
                let subject = decode(value)?;
                // A line break in a header value is how headers get injected.
                if subject.chars().any(char::is_control) || subject.len() > MAX_SUBJECT_BYTES {
                    return Err(MailError::InvalidInput);
                }
                mailto.subject = subject;
                subject_seen = true;
            }
            "body" if !body_seen => {
                let body = decode(value)?.replace("\r\n", "\n").replace('\r', "\n");
                if body.len() > MAX_MAILTO_BODY_BYTES
                    || body.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
                {
                    return Err(MailError::InvalidInput);
                }
                mailto.body = body;
                body_seen = true;
            }
            // Other hfields, and a repeated subject or body, are ignored.
            _ => {}
        }
    }
    Ok(mailto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_link_and_every_field_are_read() {
        let m = parse("mailto:ann@example.com").unwrap();
        assert_eq!(m.to, ["ann@example.com"]);
        let m = parse(
            "MAILTO:ann@example.com,bob@example.com?cc=cy@example.com&bcc=dee@example.com\
             &subject=Hello%20W%C3%B6rld&body=Line%201%0D%0ALine%202%25",
        )
        .unwrap();
        assert_eq!(m.to, ["ann@example.com", "bob@example.com"]);
        assert_eq!(m.cc, ["cy@example.com"]);
        assert_eq!(m.bcc, ["dee@example.com"]);
        assert_eq!(m.subject, "Hello Wörld");
        assert_eq!(m.body, "Line 1\nLine 2%");
    }

    #[test]
    fn to_fields_merge_with_the_path_without_duplicates() {
        let m = parse("mailto:a@example.com?to=A@example.com,b@example.com&TO=c@example.com").unwrap();
        assert_eq!(m.to, ["a@example.com", "b@example.com", "c@example.com"]);
    }

    #[test]
    fn other_fields_are_ignored_and_the_fragment_is_dropped() {
        let m = parse(
            "mailto:a@example.com?from=evil@example.com&attachment=C:%5Csecret.txt\
             &in-reply-to=%3Cx%3E&x-custom=1&subject=Hi#frag",
        )
        .unwrap();
        assert_eq!(m, Mailto {
            to: vec!["a@example.com".into()],
            subject: "Hi".into(),
            ..Mailto::default()
        });
    }

    #[test]
    fn header_injection_is_refused() {
        for link in [
            "mailto:a@example.com?subject=Hi%0D%0ABcc:%20evil@example.com",
            "mailto:a@example.com?subject=Hi%0AX:y",
            "mailto:a@example.com?subject=Hi%00",
            "mailto:a@example.com%0D%0ABcc:evil@example.com",
            "mailto:a@example.com?to=b@example.com%0D%0ABcc:evil@example.com",
            "mailto:a@example.com?cc=b@example.com%0Ax@example.com",
            "mailto:%22a%22@example.com",
            "mailto:Ann%20%3Cann@example.com%3E",
            "mailto:a@example.com?body=x%00y",
        ] {
            assert_eq!(parse(link), Err(MailError::InvalidInput), "{link}");
        }
    }

    #[test]
    fn malformed_links_are_refused() {
        for link in [
            "http://example.com",
            "mailto://a@example.com",
            "mailto:a@example.com?subject=%ZZ",
            "mailto:a@example.com?subject=%E9",
            "mailto:not-an-address",
            "mail",
            "",
        ] {
            assert_eq!(parse(link), Err(MailError::InvalidInput), "{link}");
        }
    }

    #[test]
    fn oversized_links_are_refused() {
        let long = format!("mailto:a@example.com?body={}", "x".repeat(MAX_MAILTO_BYTES));
        assert_eq!(parse(&long), Err(MailError::TooLarge));
        let body = format!("mailto:a@example.com?body={}", "x".repeat(MAX_MAILTO_BODY_BYTES + 1));
        assert_eq!(parse(&body), Err(MailError::InvalidInput));
        let subject = format!(
            "mailto:a@example.com?subject={}",
            "s".repeat(MAX_SUBJECT_BYTES + 1)
        );
        assert_eq!(parse(&subject), Err(MailError::InvalidInput));
        let many: Vec<String> = (0..=MAX_MAILTO_RECIPIENTS)
            .map(|i| format!("u{i}@example.com"))
            .collect();
        assert_eq!(
            parse(&format!("mailto:{}", many.join(","))),
            Err(MailError::TooLarge)
        );
    }

    #[test]
    fn is_mailto_is_case_insensitive_and_char_boundary_safe() {
        assert!(is_mailto("MailTo:a@b.c"));
        assert!(!is_mailto("mailt"));
        assert!(!is_mailto("mailtö:x"));
    }
}
