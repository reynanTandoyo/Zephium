//! Plain data shared by the engine, the cache and the IPC layer. Nothing here
//! holds a secret: the password lives only in the OS vault ([`crate::vault`]).

use serde::{Deserialize, Serialize};

use crate::auth::AuthKind;
use crate::error::{MailError, Result};
use crate::limits;

/// How a connection is secured. Plain text is never offered.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte (IMAPS 993, SMTPS 465).
    Tls,
    /// Plain greeting upgraded with STARTTLS before any credential is sent.
    StartTls,
}

/// One server endpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Host name (never an IP literal with a scheme).
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Transport security.
    pub security: Security,
}

/// A mail account. Holds no password.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Account {
    /// Stable id: 1 to 40 characters of `[A-Za-z0-9-]`.
    pub id: String,
    /// Name shown in the UI and used as the sender name.
    pub display_name: String,
    /// The address mail is sent from.
    pub email: String,
    /// Incoming server.
    pub imap: Endpoint,
    /// Outgoing server.
    pub smtp: Endpoint,
    /// Login name for both servers.
    pub username: String,
    /// Accent colour as `#rrggbb`.
    pub colour: String,
    /// Plain-text signature appended to new messages.
    pub signature: String,
    /// Append sent mail to the Sent folder after SMTP. On by default: most
    /// servers do not file submissions themselves.
    pub save_sent_copy: bool,
    /// How the account signs in. Absent in accounts saved before OAuth
    /// existed, which are password accounts.
    #[serde(default)]
    pub auth: AuthKind,
}

fn host_ok(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && !host.contains("..")
}

/// True for a plausible address: one `@`, a dotted host, no controls or
/// separators that could inject headers or commands.
#[must_use]
pub fn email_ok(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    email.len() <= 254
        && !local.is_empty()
        && !local.contains('@')
        && local.len() <= 64
        && host_ok(domain)
        && domain.contains('.')
        && !email.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '<' | '>' | '"' | ',' | ';' | '(' | ')')
        })
}

/// True for an account id.
#[must_use]
pub fn id_ok(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn colour_ok(colour: &str) -> bool {
    colour.len() == 7
        && colour.starts_with('#')
        && colour[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn text_ok(text: &str) -> bool {
    text.len() <= limits::MAX_ACCOUNT_TEXT_BYTES && !text.contains('\0')
}

impl Endpoint {
    /// Checks the host and port.
    pub fn validate(&self) -> Result<()> {
        if host_ok(&self.host) && self.port != 0 {
            Ok(())
        } else {
            Err(MailError::InvalidInput)
        }
    }
}

impl Account {
    /// Checks every field against its bound.
    pub fn validate(&self) -> Result<()> {
        let ok = id_ok(&self.id)
            && text_ok(&self.display_name)
            && email_ok(&self.email)
            && !self.username.is_empty()
            && self.username.len() <= 320
            && !self.username.chars().any(char::is_control)
            && colour_ok(&self.colour)
            && text_ok(&self.signature);
        if !ok {
            return Err(MailError::InvalidInput);
        }
        self.auth.validate()?;
        self.imap.validate()?;
        self.smtp.validate()
    }
}

/// What a folder is for, from RFC 6154 SPECIAL-USE or its name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    /// The inbox.
    Inbox,
    /// Sent mail.
    Sent,
    /// Drafts.
    Drafts,
    /// Deleted mail.
    Trash,
    /// Archive.
    Archive,
    /// Spam.
    Junk,
    /// Any other folder.
    Other,
}

impl FolderRole {
    /// Stable text for the cache.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Trash => "trash",
            Self::Archive => "archive",
            Self::Junk => "junk",
            Self::Other => "other",
        }
    }

    /// Inverse of [`Self::as_str`]; unknown text is `Other`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "inbox" => Self::Inbox,
            "sent" => Self::Sent,
            "drafts" => Self::Drafts,
            "trash" => Self::Trash,
            "archive" => Self::Archive,
            "junk" => Self::Junk,
            _ => Self::Other,
        }
    }
}

/// A mailbox on the server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    /// Full decoded path, the key used in every call.
    pub path: String,
    /// Last path segment.
    pub name: String,
    /// Hierarchy delimiter.
    pub delimiter: Option<char>,
    /// What it is for.
    pub role: FolderRole,
    /// Unread messages (STATUS UNSEEN).
    pub unread: u32,
    /// All messages (STATUS MESSAGES).
    pub total: u32,
    /// UIDVALIDITY; a change invalidates cached UIDs.
    pub uid_validity: Option<u32>,
    /// UIDNEXT.
    pub uid_next: Option<u32>,
    /// HIGHESTMODSEQ when the server has CONDSTORE.
    pub highest_modseq: Option<u64>,
    /// False for hierarchy-only folders.
    pub selectable: bool,
}

/// One address.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct Address {
    /// Display name, when present.
    pub name: Option<String>,
    /// The address.
    pub email: String,
}

/// Message flags the UI cares about.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct Flags {
    /// `\Seen`.
    pub seen: bool,
    /// `\Flagged`.
    pub flagged: bool,
    /// `\Answered`.
    pub answered: bool,
    /// `\Draft`.
    pub draft: bool,
    /// `\Deleted`.
    pub deleted: bool,
}

/// A flag a caller may change.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    /// `\Seen`.
    Seen,
    /// `\Flagged`.
    Flagged,
    /// `\Deleted`.
    Deleted,
}

impl FlagKind {
    /// The wire name.
    #[must_use]
    pub fn wire(self) -> &'static str {
        match self {
            Self::Seen => "\\Seen",
            Self::Flagged => "\\Flagged",
            Self::Deleted => "\\Deleted",
        }
    }
}

/// A row of the message list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct MessageHeader {
    /// UID within the folder (valid for the folder's UIDVALIDITY).
    pub uid: u32,
    /// Message-ID without angle brackets.
    pub message_id: Option<String>,
    /// Subject (decoded, capped).
    pub subject: String,
    /// From.
    pub from: Vec<Address>,
    /// To.
    pub to: Vec<Address>,
    /// Cc.
    pub cc: Vec<Address>,
    /// Date header as Unix seconds, or the server's internal date.
    pub date: Option<i64>,
    /// Flags.
    pub flags: Flags,
    /// RFC822 size in bytes.
    pub size: u32,
    /// True when the structure suggests an attachment.
    pub has_attachments: bool,
    /// First words of the text.
    pub snippet: String,
    /// In-Reply-To without brackets.
    pub in_reply_to: Option<String>,
    /// References without brackets, oldest first.
    pub references: Vec<String>,
}

/// One page of headers, newest first.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct HeaderPage {
    /// Rows, highest UID first.
    pub messages: Vec<MessageHeader>,
    /// Pass this as `before_uid` for the next page; `None` at the end.
    pub next_before: Option<u32>,
    /// Folder UIDVALIDITY when the page was read.
    pub uid_validity: Option<u32>,
    /// Folder message count when the page was read.
    pub total: u32,
}

/// Attachment metadata; the bytes come on demand.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// Stable index among the message's attachment parts.
    pub part: u32,
    /// File name, sanitised.
    pub filename: String,
    /// MIME type.
    pub content_type: String,
    /// Decoded size in bytes.
    pub size: u32,
    /// Content-ID without brackets, for inline parts.
    pub content_id: Option<String>,
    /// True when it is shown inline in the HTML body.
    pub inline: bool,
    /// The `method` parameter of a calendar part (`REQUEST`, `CANCEL`, `REPLY`...),
    /// upper-cased; `None` for other parts or when the part names none.
    #[serde(default)]
    pub method: Option<String>,
}

impl AttachmentInfo {
    /// A calendar invitation: a `text/calendar` (or legacy `application/ics`)
    /// part, or a file named `*.ics`, small enough to be read.
    #[must_use]
    pub fn is_calendar(&self) -> bool {
        let named = self.filename.to_ascii_lowercase().ends_with(".ics");
        let typed = matches!(
            self.content_type.as_str(),
            "text/calendar" | "application/ics" | "text/x-vcalendar"
        );
        (typed || named) && (self.size as usize) <= limits::MAX_INVITATION_BYTES
    }
}

impl MessageBody {
    /// The calendar parts of the message, at most [`limits::MAX_INVITATION_PARTS`].
    #[must_use]
    pub fn invitation_parts(&self) -> Vec<&AttachmentInfo> {
        self.attachments
            .iter()
            .filter(|part| part.is_calendar())
            .take(limits::MAX_INVITATION_PARTS)
            .collect()
    }
}

/// An inline image referenced from the HTML as `cid:`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InlineImage {
    /// Content-ID without brackets.
    pub content_id: String,
    /// MIME type (`image/*` only).
    pub content_type: String,
    /// Decoded bytes, at most [`limits::MAX_INLINE_BYTES`].
    pub data: Vec<u8>,
}

/// A fully parsed message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct MessageBody {
    /// Header fields.
    pub header: MessageHeader,
    /// Reply-To addresses.
    pub reply_to: Vec<Address>,
    /// Plain text part, capped.
    pub text: Option<String>,
    /// Raw HTML part, capped. Not sanitised: the UI must do that.
    pub html: Option<String>,
    /// True when a text or HTML part was cut at its cap.
    pub truncated: bool,
    /// Attachments (including inline parts).
    pub attachments: Vec<AttachmentInfo>,
    /// Inline images with a Content-ID.
    pub inline_images: Vec<InlineImage>,
}

/// An attachment on an outgoing message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutgoingAttachment {
    /// File name.
    pub filename: String,
    /// MIME type.
    pub content_type: String,
    /// Bytes.
    pub data: Vec<u8>,
}

/// A message to send or save as a draft.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct Draft {
    /// To.
    pub to: Vec<Address>,
    /// Cc.
    pub cc: Vec<Address>,
    /// Bcc (never written into headers).
    pub bcc: Vec<Address>,
    /// Subject.
    pub subject: String,
    /// Plain text body.
    pub text: String,
    /// Optional HTML body.
    pub html: Option<String>,
    /// Attachments.
    pub attachments: Vec<OutgoingAttachment>,
    /// Message-ID being replied to, without brackets.
    pub in_reply_to: Option<String>,
    /// Thread references, oldest first, without brackets.
    pub references: Vec<String>,
    /// An iMIP reply: the text body plus a `text/calendar; method=...` alternative.
    #[serde(default)]
    pub calendar: Option<CalendarBody>,
}

/// The `text/calendar` alternative of an outgoing iMIP message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CalendarBody {
    /// `REPLY` is the only method this app sends.
    pub method: String,
    /// The iCalendar text (UTF-8).
    pub ics: String,
}

/// A change the server pushed while watching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchOutcome {
    /// The server reported new or changed mail.
    Changed,
    /// The renewal interval passed with nothing new.
    Idle,
    /// The caller stopped the watch.
    Stopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        let endpoint = |port| Endpoint {
            host: "mail.example.com".into(),
            port,
            security: Security::Tls,
        };
        Account {
            id: "acc-1".into(),
            display_name: "Ann".into(),
            email: "ann@example.com".into(),
            imap: endpoint(993),
            smtp: endpoint(465),
            username: "ann@example.com".into(),
            colour: "#336699".into(),
            signature: String::new(),
            save_sent_copy: true,
            auth: AuthKind::Password,
        }
    }

    #[test]
    fn accounts_saved_before_oauth_still_load_as_password_accounts() {
        let mut value = serde_json::to_value(account()).unwrap();
        value.as_object_mut().unwrap().remove("auth");
        let loaded: Account = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.auth, AuthKind::Password);
        assert_eq!(loaded, account());
    }

    #[test]
    fn oauth_accounts_round_trip_and_validate_their_client() {
        let mut oauth = account();
        oauth.auth = AuthKind::Oauth {
            provider: zephium_oauth::provider::Provider::Google,
            client_id: "123-abc.apps.googleusercontent.com".into(),
            tenant: None,
        };
        let json = serde_json::to_string(&oauth).unwrap();
        assert_eq!(serde_json::from_str::<Account>(&json).unwrap(), oauth);
        assert_eq!(oauth.validate(), Ok(()));
        oauth.auth = AuthKind::Oauth {
            provider: zephium_oauth::provider::Provider::Microsoft,
            client_id: "bad id".into(),
            tenant: Some("x/y".into()),
        };
        assert_eq!(oauth.validate(), Err(MailError::InvalidInput));
    }

    #[test]
    fn a_valid_account_passes_and_bad_fields_fail() {
        assert_eq!(account().validate(), Ok(()));
        let mut bad = account();
        bad.id = "../x".into();
        assert_eq!(bad.validate(), Err(MailError::InvalidInput));
        let mut bad = account();
        bad.imap.host = "evil host".into();
        assert_eq!(bad.validate(), Err(MailError::InvalidInput));
        let mut bad = account();
        bad.colour = "red".into();
        assert_eq!(bad.validate(), Err(MailError::InvalidInput));
        let mut bad = account();
        bad.smtp.port = 0;
        assert_eq!(bad.validate(), Err(MailError::InvalidInput));
    }

    #[test]
    fn email_rejects_header_injection() {
        assert!(email_ok("a.b@example.com"));
        for bad in [
            "a@b",
            "a@@b.com",
            "a b@x.com",
            "a@x.com\r\nBcc: z@x.com",
            "<a@x.com>",
            "",
        ] {
            assert!(!email_ok(bad), "{bad:?}");
        }
    }
}
