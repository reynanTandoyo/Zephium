//! IMAP modified UTF-7 (RFC 3501 section 5.1.3) for folder names.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Decodes a mailbox name; `None` when it is malformed.
#[must_use]
pub fn decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'&' {
            if !(0x20..=0x7e).contains(&b) {
                return None;
            }
            out.push(char::from(b));
            i += 1;
            continue;
        }
        let end = bytes[i + 1..].iter().position(|&c| c == b'-')? + i + 1;
        let run = &bytes[i + 1..end];
        if run.is_empty() {
            out.push('&');
        } else {
            let mut bits: u32 = 0;
            let mut nbits = 0;
            let mut units: Vec<u16> = Vec::new();
            for &c in run {
                let v = B64.iter().position(|&x| x == c)? as u32;
                bits = (bits << 6) | v;
                nbits += 6;
                if nbits >= 16 {
                    nbits -= 16;
                    units.push(u16::try_from((bits >> nbits) & 0xffff).ok()?);
                    bits &= (1 << nbits) - 1;
                }
            }
            if bits != 0 {
                return None;
            }
            out.push_str(&String::from_utf16(&units).ok()?);
        }
        i = end + 1;
    }
    Some(out)
}

/// Encodes a name for the wire.
#[must_use]
pub fn encode(input: &str) -> String {
    fn flush(pending: &mut Vec<u16>, out: &mut String) {
        if pending.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(pending.len() * 2);
        for unit in pending.drain(..) {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        out.push('&');
        let mut bits: u32 = 0;
        let mut nbits = 0;
        for byte in bytes {
            bits = (bits << 8) | u32::from(byte);
            nbits += 8;
            while nbits >= 6 {
                nbits -= 6;
                out.push(char::from(B64[((bits >> nbits) & 63) as usize]));
            }
            bits &= (1 << nbits) - 1;
        }
        if nbits > 0 {
            out.push(char::from(B64[((bits << (6 - nbits)) & 63) as usize]));
        }
        out.push('-');
    }
    let mut out = String::with_capacity(input.len());
    let mut pending: Vec<u16> = Vec::new();
    for ch in input.chars() {
        if ('\u{20}'..='\u{7e}').contains(&ch) {
            flush(&mut pending, &mut out);
            if ch == '&' {
                out.push_str("&-");
            } else {
                out.push(ch);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_non_ascii_and_ampersand() {
        for name in ["INBOX", "Entwürfe", "R&D", "日本語", "A/Ünder/😀"] {
            assert_eq!(decode(&encode(name)).as_deref(), Some(name), "{name}");
        }
        assert_eq!(encode("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(decode("&-").as_deref(), Some("&"));
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(decode("&AP"), None);
        assert_eq!(decode("a\u{0}b"), None);
        assert_eq!(decode("&A$-"), None);
    }
}
