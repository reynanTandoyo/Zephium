//! MIME parsing into the safe model (T-0021). Input is a hostile byte string:
//! sizes are checked first, text is capped, file names are sanitised and
//! nothing is executed or fetched.

use std::borrow::Cow;

use mail_parser::{Address as ParsedAddress, HeaderValue, MessageParser, MimeHeaders, PartType};

use crate::error::{MailError, Result};
use crate::limits::{
    MAX_ADDRESSES, MAX_ATTACHMENTS, MAX_BODY_TEXT_BYTES, MAX_HEADER_BYTES, MAX_INLINE_BYTES,
    MAX_MESSAGE_BYTES, MAX_PARTS, MAX_REFERENCES, MAX_SNIPPET_BYTES, MAX_SUBJECT_BYTES,
};
use crate::model::{Address, AttachmentInfo, InlineImage, MessageBody, MessageHeader};

fn cap(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_owned(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

fn clean_line(text: &str, max: usize) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cap(collapsed.trim(), max).0
}

fn addresses(value: Option<&ParsedAddress<'_>>) -> Vec<Address> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .iter()
        .filter_map(|addr| {
            let email = addr.address()?.trim();
            if email.is_empty() || email.len() > 320 || email.chars().any(char::is_control) {
                return None;
            }
            Some(Address {
                name: addr
                    .name()
                    .map(|name| clean_line(name, 256))
                    .filter(|name| !name.is_empty()),
                email: email.to_owned(),
            })
        })
        .take(MAX_ADDRESSES)
        .collect()
}

fn id_list(value: &HeaderValue<'_>) -> Vec<String> {
    let clean = |id: &str| clean_line(id.trim_matches(|c| c == '<' || c == '>'), 998);
    match value {
        HeaderValue::Text(text) => vec![clean(text)],
        HeaderValue::TextList(list) => list.iter().map(|id| clean(id)).collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .filter(|id| !id.is_empty())
    .take(MAX_REFERENCES)
    .collect()
}

/// Reduces markup to its text for a list snippet. This is not sanitising:
/// the output is plain text and is only ever shown as text.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len().min(2048));
    let mut in_tag = false;
    let mut skip_until: Option<&str> = None;
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() && out.len() < 4096 {
        if let Some(close) = skip_until {
            match lower[i..].find(close) {
                Some(at) => {
                    i += at + close.len();
                    skip_until = None;
                    in_tag = true;
                }
                None => break,
            }
            continue;
        }
        let b = bytes[i];
        if in_tag {
            if b == b'>' {
                in_tag = false;
                out.push(' ');
            }
            i += 1;
        } else if b == b'<' {
            if lower[i..].starts_with("<style") {
                skip_until = Some("</style");
            } else if lower[i..].starts_with("<script") {
                skip_until = Some("</script");
            } else {
                in_tag = true;
            }
            i += 1;
        } else {
            let ch = html[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&")
}

fn snippet(text: Option<Cow<'_, str>>, html: Option<Cow<'_, str>>) -> String {
    let source = match (text, html) {
        (Some(text), _) if !text.trim().is_empty() => text.into_owned(),
        (_, Some(html)) => strip_tags(&html),
        _ => String::new(),
    };
    let squeezed = source.split_whitespace().collect::<Vec<_>>().join(" ");
    cap(&squeezed, MAX_SNIPPET_BYTES).0
}

fn header_from(message: &mail_parser::Message<'_>, probe_text: bool) -> MessageHeader {
    let ctype = message.content_type().map(|c| {
        (
            c.ctype().to_ascii_lowercase(),
            c.subtype().map(str::to_ascii_lowercase),
        )
    });
    let has_attachments = match &ctype {
        Some((main, Some(sub))) => {
            main == "multipart" && matches!(sub.as_str(), "mixed" | "signed" | "report")
        }
        _ => false,
    } || message.attachment_count() > 0 && probe_text;
    MessageHeader {
        uid: 0,
        message_id: message
            .message_id()
            .map(|id| clean_line(id, 998))
            .filter(|id| !id.is_empty()),
        subject: clean_line(message.subject().unwrap_or(""), MAX_SUBJECT_BYTES),
        from: addresses(message.from()),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        date: message.date().map(mail_parser::DateTime::to_timestamp),
        flags: crate::model::Flags::default(),
        size: 0,
        has_attachments,
        snippet: String::new(),
        in_reply_to: id_list(message.in_reply_to()).into_iter().next(),
        references: id_list(message.references()),
    }
}

/// Parses the header block fetched for a list row. `text_probe` is the first
/// bytes of the body, used only for the snippet.
pub fn parse_list_header(header: &[u8], text_probe: Option<&[u8]>) -> Result<MessageHeader> {
    let header = &header[..header.len().min(MAX_HEADER_BYTES)];
    let mut joined = header.to_vec();
    if let Some(probe) = text_probe {
        if !joined.ends_with(b"\r\n\r\n") && !joined.ends_with(b"\n\n") {
            joined.extend_from_slice(b"\r\n\r\n");
        }
        joined.extend_from_slice(&probe[..probe.len().min(2048)]);
    }
    let message = MessageParser::default()
        .parse(&joined)
        .ok_or(MailError::Protocol)?;
    let mut parsed = header_from(&message, false);
    if text_probe.is_some() {
        parsed.snippet = snippet(message.body_text(0), message.body_html(0));
    }
    Ok(parsed)
}

/// Strips what a file system or a shell would treat specially.
#[must_use]
pub fn sanitize_filename(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let mut name = cap(trimmed, 120).0;
    if name.is_empty() {
        return "attachment".to_owned();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        name.insert(0, '_');
    }
    name
}

fn is_safe_inline_image(ctype: &str) -> bool {
    matches!(
        ctype,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp"
    )
}

fn part_content_type(part: &mail_parser::MessagePart<'_>) -> String {
    part.content_type()
        .map(|c| match c.subtype() {
            Some(sub) => format!("{}/{}", c.ctype(), sub).to_ascii_lowercase(),
            None => c.ctype().to_ascii_lowercase(),
        })
        .filter(|ct| ct.len() <= 127 && !ct.chars().any(|c| c.is_control() || c.is_whitespace()))
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

fn attachment_info(index: usize, part: &mail_parser::MessagePart<'_>) -> AttachmentInfo {
    let content_type = part_content_type(part);
    let filename = sanitize_filename(part.attachment_name().unwrap_or(""));
    let content_id = part
        .content_id()
        .map(|id| clean_line(id.trim_matches(|c| c == '<' || c == '>'), 256))
        .filter(|id| !id.is_empty());
    let inline = part
        .content_disposition()
        .is_some_and(|d| d.ctype().eq_ignore_ascii_case("inline"))
        || content_id.is_some();
    AttachmentInfo {
        part: u32::try_from(index).unwrap_or(u32::MAX),
        filename,
        content_type,
        size: u32::try_from(part.len()).unwrap_or(u32::MAX),
        content_id,
        inline,
    }
}

fn part_bytes<'a>(part: &'a mail_parser::MessagePart<'_>) -> &'a [u8] {
    match &part.body {
        PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes,
        PartType::Text(text) | PartType::Html(text) => text.as_bytes(),
        PartType::Message(message) => message.raw_message(),
        PartType::Multipart(_) => &[],
    }
}

/// Parses a whole message for reading.
pub fn parse_message(raw: &[u8]) -> Result<MessageBody> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(MailError::TooLarge);
    }
    let message = MessageParser::default()
        .parse(raw)
        .ok_or(MailError::Protocol)?;
    if message.parts.len() > MAX_PARTS {
        return Err(MailError::TooLarge);
    }
    let mut header = header_from(&message, true);
    header.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let mut truncated = false;
    let text = message.body_text(0).map(|body| {
        let (kept, cut) = cap(&body, MAX_BODY_TEXT_BYTES);
        truncated |= cut;
        kept
    });
    let html = message.body_html(0).map(|body| {
        let (kept, cut) = cap(&body, MAX_BODY_TEXT_BYTES);
        truncated |= cut;
        kept
    });
    header.snippet = snippet(message.body_text(0), message.body_html(0));
    let mut attachments = Vec::new();
    let mut inline_images = Vec::new();
    for (index, part) in message.attachments().enumerate().take(MAX_ATTACHMENTS) {
        let info = attachment_info(index, part);
        if let Some(id) = &info.content_id {
            if is_safe_inline_image(&info.content_type) && part.len() <= MAX_INLINE_BYTES {
                inline_images.push(InlineImage {
                    content_id: id.clone(),
                    content_type: info.content_type.clone(),
                    data: part_bytes(part).to_vec(),
                });
            }
        }
        attachments.push(info);
    }
    header.has_attachments = attachments.iter().any(|a| !a.inline) || header.has_attachments;
    Ok(MessageBody {
        header,
        reply_to: addresses(message.reply_to()),
        text,
        html,
        truncated,
        attachments,
        inline_images,
    })
}

/// Extracts one attachment's decoded bytes.
pub fn attachment_bytes(raw: &[u8], part: u32) -> Result<(AttachmentInfo, Vec<u8>)> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(MailError::TooLarge);
    }
    let message = MessageParser::default()
        .parse(raw)
        .ok_or(MailError::Protocol)?;
    if message.parts.len() > MAX_PARTS {
        return Err(MailError::TooLarge);
    }
    let index = usize::try_from(part).map_err(|_| MailError::InvalidInput)?;
    if index >= MAX_ATTACHMENTS {
        return Err(MailError::NotFound);
    }
    let found = message.attachment(part).ok_or(MailError::NotFound)?;
    if found.len() > crate::limits::MAX_ATTACHMENT_BYTES {
        return Err(MailError::TooLarge);
    }
    Ok((attachment_info(index, found), part_bytes(found).to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &[u8] = include_bytes!("../tests/fixtures/plain.eml");
    const MULTIPART: &[u8] = include_bytes!("../tests/fixtures/multipart.eml");
    const HOSTILE: &[u8] = include_bytes!("../tests/fixtures/hostile.eml");

    #[test]
    fn parses_a_plain_message_with_encoded_headers() {
        let body = parse_message(PLAIN).unwrap();
        assert_eq!(body.header.subject, "Grüße aus Köln");
        assert_eq!(body.header.from[0].email, "ann@example.com");
        assert_eq!(body.header.from[0].name.as_deref(), Some("Ann Beispiel"));
        assert_eq!(body.header.to.len(), 2);
        assert_eq!(body.header.cc[0].email, "cc@example.com");
        assert_eq!(body.header.message_id.as_deref(), Some("m1@example.com"));
        assert_eq!(body.header.in_reply_to.as_deref(), Some("m0@example.com"));
        assert_eq!(
            body.header.references,
            ["m-1@example.com", "m0@example.com"]
        );
        assert_eq!(body.header.date, Some(1_731_499_200));
        assert!(body.text.as_deref().unwrap().contains("schöne Grüße"));
        assert!(body.attachments.is_empty());
        assert!(!body.header.has_attachments);
        assert!(body.header.snippet.starts_with("Hallo Bob"));
    }

    #[test]
    fn parses_multipart_with_html_attachment_and_inline_image() {
        let body = parse_message(MULTIPART).unwrap();
        assert!(body.text.as_deref().unwrap().contains("plain version"));
        assert!(body
            .html
            .as_deref()
            .unwrap()
            .contains("cid:logo@example.com"));
        let files: Vec<_> = body.attachments.iter().filter(|a| !a.inline).collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "report.pdf");
        assert_eq!(files[0].content_type, "application/pdf");
        assert_eq!(files[0].size, 8);
        assert!(body.header.has_attachments);
        assert_eq!(body.inline_images.len(), 1);
        assert_eq!(body.inline_images[0].content_id, "logo@example.com");
        assert_eq!(body.inline_images[0].content_type, "image/png");
        let (info, bytes) = attachment_bytes(MULTIPART, files[0].part).unwrap();
        assert_eq!(info.filename, "report.pdf");
        assert_eq!(bytes, b"%PDF-1.4");
        assert_eq!(
            attachment_bytes(MULTIPART, 99).unwrap_err(),
            MailError::NotFound
        );
    }

    #[test]
    fn hostile_headers_and_names_are_neutralised() {
        let body = parse_message(HOSTILE).unwrap();
        assert!(!body.header.subject.chars().any(char::is_control));
        let file = &body.attachments[0];
        assert!(!file.filename.contains(['/', '\\', ':']));
        assert!(!file.filename.starts_with('.'));
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("  ..  "), "attachment");
        assert_eq!(sanitize_filename(&"a".repeat(500)).len(), 120);
    }

    #[test]
    fn enforces_size_and_text_caps() {
        let huge = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        assert_eq!(parse_message(&huge), Err(MailError::TooLarge));
        let mut big = b"Subject: big\r\nContent-Type: text/plain\r\n\r\n".to_vec();
        big.extend(std::iter::repeat_n(b'a', MAX_BODY_TEXT_BYTES + 10));
        let body = parse_message(&big).unwrap();
        assert!(body.truncated);
        assert_eq!(body.text.unwrap().len(), MAX_BODY_TEXT_BYTES);
        assert!(parse_message(b"").is_err());
    }

    #[test]
    fn list_header_gets_a_snippet_from_the_probe_and_ignores_oversize_headers() {
        let header = b"From: Ann <ann@example.com>\r\nSubject: Hi\r\nDate: Wed, 13 Nov 2024 12:00:00 +0000\r\n\r\n";
        let parsed = parse_list_header(header, Some(b"Hello   there\r\nworld")).unwrap();
        assert_eq!(parsed.subject, "Hi");
        assert_eq!(parsed.snippet, "Hello there world");
        let mut oversized = b"Subject: s\r\n".to_vec();
        oversized.extend(std::iter::repeat_n(b'X', MAX_HEADER_BYTES * 2));
        assert!(parse_list_header(&oversized, None).is_ok());
    }

    #[test]
    fn html_only_snippets_drop_tags_scripts_and_styles() {
        let header = b"Subject: h\r\nContent-Type: text/html\r\n\r\n";
        let parsed = parse_list_header(
            header,
            Some(b"<style>p{}</style><p>Hello <b>bold</b></p><script>evil()</script>end"),
        )
        .unwrap();
        assert_eq!(parsed.snippet, "Hello bold end");
    }
}
