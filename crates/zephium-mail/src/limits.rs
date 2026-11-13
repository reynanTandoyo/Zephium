//! Hard bounds on everything a server or a caller can make large.

use std::time::Duration;

/// Time allowed to establish TCP and TLS.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Time one IMAP command may take.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
/// Time one SMTP submission may take.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(90);
/// Longest IDLE before it is renewed (servers drop it near 30 minutes).
pub const IDLE_RENEW: Duration = Duration::from_secs(25 * 60);

/// Folders kept per account.
pub const MAX_FOLDERS: usize = 512;
/// Bytes of one folder name (decoded).
pub const MAX_FOLDER_NAME_BYTES: usize = 512;
/// Headers returned per page.
pub const MAX_PAGE: u32 = 200;
/// Bytes of the header block parsed for a list row.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;
/// Bytes of one raw message fetched for reading.
pub const MAX_MESSAGE_BYTES: usize = 40 * 1024 * 1024;
/// Bytes of the text/plain and text/html bodies, each.
pub const MAX_BODY_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// Parts walked in one message.
pub const MAX_PARTS: usize = 256;
/// Attachments listed for one message.
pub const MAX_ATTACHMENTS: usize = 128;
/// Bytes of one attachment handed out.
pub const MAX_ATTACHMENT_BYTES: usize = 35 * 1024 * 1024;
/// Bytes of one decoded inline image kept in memory.
pub const MAX_INLINE_BYTES: usize = 1024 * 1024;
/// Addresses kept in one header field.
pub const MAX_ADDRESSES: usize = 200;
/// Bytes of a subject.
pub const MAX_SUBJECT_BYTES: usize = 1024;
/// Bytes of a snippet.
pub const MAX_SNIPPET_BYTES: usize = 240;
/// References kept per message.
pub const MAX_REFERENCES: usize = 32;
/// Bytes of an autoconfig XML document.
pub const MAX_AUTOCONFIG_BYTES: usize = 64 * 1024;
/// Bytes of a search query.
pub const MAX_QUERY_BYTES: usize = 256;
/// Messages returned from one search.
pub const MAX_SEARCH_HITS: usize = 500;
/// Recipients on one outgoing message.
pub const MAX_RECIPIENTS: usize = 200;
/// Total bytes of an outgoing message.
pub const MAX_OUTGOING_BYTES: usize = 35 * 1024 * 1024;
/// Bytes of a text field in an account (name, signature, host).
pub const MAX_ACCOUNT_TEXT_BYTES: usize = 8 * 1024;
/// Accounts kept.
pub const MAX_ACCOUNTS: usize = 16;
/// Bytes of message bodies the local cache keeps in total.
pub const MAX_CACHED_BODY_BYTES: usize = 256 * 1024 * 1024;
