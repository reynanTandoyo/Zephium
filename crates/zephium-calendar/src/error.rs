//! Closed set of calendar failures. No variant carries server text, URLs or
//! credentials, so an error is always safe to log or send over IPC.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a calendar operation failed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Error, PartialEq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CalendarError {
    /// The caller passed a value that is malformed or out of range.
    #[error("invalid input")]
    InvalidInput,
    /// A document or response exceeds the hard size limit.
    #[error("too large")]
    TooLarge,
    /// A limit on counts (events, occurrences, calendars) was reached.
    #[error("limit reached")]
    Limit,
    /// iCalendar or XML content could not be understood.
    #[error("unreadable calendar data")]
    Parse,
    /// The server could not be reached.
    #[error("network error")]
    Network,
    /// The server did not answer in time.
    #[error("timeout")]
    Timeout,
    /// The address is not `https` (or loopback `http`).
    #[error("insecure address")]
    Insecure,
    /// The server rejected the user name or password.
    #[error("not signed in")]
    Unauthorized,
    /// The server refuses this operation.
    #[error("forbidden")]
    Forbidden,
    /// The server or the item does not exist.
    #[error("not found")]
    NotFound,
    /// Someone else changed the event first (ETag mismatch).
    #[error("conflict")]
    Conflict,
    /// The server answered with an error or an unexpected shape.
    #[error("server error")]
    Server,
    /// The server does not speak CalDAV.
    #[error("not a calendar server")]
    Unsupported,
    /// The calendar is read-only.
    #[error("read-only calendar")]
    ReadOnly,
    /// Reading or writing the local calendar file failed.
    #[error("storage error")]
    Storage,
    /// The OS credential vault refused or is unavailable.
    #[error("credential vault unavailable")]
    Credentials,
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, CalendarError>;
