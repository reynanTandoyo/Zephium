//! Fork-owned calendar backend: iCalendar parsing and recurrence expansion,
//! CalDAV accounts, read-only ICS subscriptions and an event cache that is
//! kept in memory and persisted through [`store::EventCache`].
//!
//! Network access goes through [`http::HttpClient`], durable bookkeeping
//! through [`store::CalendarStorage`] and passwords through
//! [`store::SecretStore`] (Windows Credential Manager / macOS Keychain via
//! `zephium-credentials`), so tests run on fixtures only.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod api;
pub mod caldav;
pub mod error;
pub mod http;
pub mod ical;
pub mod model;
pub mod persist;
pub mod recur;
pub mod service;
pub mod store;
pub mod tz;
pub mod vtimezone;
pub mod xml;

pub use error::{CalendarError, Result};
pub use service::CalendarService;

#[cfg(test)]
mod tests;
