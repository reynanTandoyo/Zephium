//! Calendar providers that sign in with OAuth: Google Calendar API v3 and
//! Microsoft Graph. This module holds what both share: the authenticated
//! request path (bearer token from `zephium-oauth`, one refresh after a 401),
//! the neutral [`Item`] a sync produces, and the bounds every response is
//! held to. Mapping to the iCalendar model lives in [`google`] and [`graph`].
//!
//! Every response is untrusted: bodies are capped, counts are capped, links
//! the server sends are checked against the provider's own host before they
//! are followed, and failures map to the closed [`CalendarError`] without
//! server text.

pub mod google;
pub mod graph;

use crate::api::CalendarAccountV1;
use crate::error::{CalendarError, Result};
use crate::http::{send, HttpClient, HttpRequest, HttpResponse, Secret};
use crate::model::{Event, EventTime};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use zephium_oauth::manager::TokenSessions;
use zephium_oauth::provider::{ClientConfig, Provider, Purpose};
use zephium_oauth::OAuthError;

/// Largest page asked of a provider.
pub const PAGE_SIZE: usize = 250;
/// Most pages read by one sync.
pub const MAX_PAGES: usize = 200;
/// Most calendars one account may bring.
pub const MAX_CALENDARS_PER_ACCOUNT: usize = 200;
/// Longest sync token or delta link kept (the store keeps 4096 bytes).
pub const MAX_TOKEN_BYTES: usize = 4_000;
/// Most text kept from one title, place or note.
pub(crate) const MAX_SUMMARY_CHARS: usize = 1_000;
pub(crate) const MAX_LOCATION_BYTES: usize = 2_000;
pub(crate) const MAX_DESCRIPTION_BYTES: usize = 100_000;

/// One provider answer, independent of the provider.
#[derive(Clone, Debug)]
pub enum Item {
    /// A whole resource: a single event, or a series master that carries no
    /// overrides of its own (they arrive as [`Item::Occurrence`]).
    Event {
        /// Provider id of the event (also the resource address in the cache).
        href: String,
        /// Version tag for conditional writes.
        etag: Option<String>,
        /// The event.
        event: Event,
    },
    /// One changed or cancelled occurrence of a series.
    Occurrence {
        /// Resource address (id) of the series master.
        series: String,
        /// The occurrence's start in the series.
        original: EventTime,
        /// The changed occurrence, or `None` when it was cancelled.
        event: Option<Event>,
    },
    /// The resource no longer exists.
    Gone {
        /// Resource address.
        href: String,
    },
}

/// What a sync read.
#[derive(Debug, Default)]
pub struct Delta {
    /// Changes in the order the provider sent them.
    pub items: Vec<Item>,
    /// Token or link for the next incremental sync.
    pub next_token: Option<String>,
    /// The items are the whole calendar, not a change on top of the cache
    /// (a first sync, or the saved token expired).
    pub full: bool,
}

/// A calendar a provider lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteCalendar {
    /// Collection address (an API URL built from fixed parts and the
    /// percent-encoded provider id).
    pub url: String,
    /// Display name.
    pub name: String,
    /// `#RRGGBB` when the provider names one.
    pub color: Option<String>,
    /// The person cannot change events here.
    pub read_only: bool,
    /// Address of the calendar's owner, when known.
    pub owner: Option<String>,
}

/// Maps a token failure to the closed calendar error set.
pub fn map_oauth(error: OAuthError) -> CalendarError {
    match error {
        OAuthError::SignInRequired | OAuthError::Rejected | OAuthError::Denied => {
            CalendarError::Unauthorized
        }
        OAuthError::Offline | OAuthError::Busy => CalendarError::Network,
        OAuthError::Timeout => CalendarError::Timeout,
        OAuthError::Credentials | OAuthError::Unsupported => CalendarError::Credentials,
        OAuthError::InvalidInput => CalendarError::InvalidInput,
        OAuthError::Cancelled
        | OAuthError::StateMismatch
        | OAuthError::Protocol
        | OAuthError::Internal => CalendarError::Server,
    }
}

/// The OAuth client of an account, as the token manager wants it.
pub fn client_config(account: &CalendarAccountV1) -> Result<ClientConfig> {
    let oauth = account.oauth.as_ref().ok_or(CalendarError::InvalidInput)?;
    let provider = match account.kind {
        crate::api::CalendarAccountKindV1::Google => Provider::Google,
        crate::api::CalendarAccountKindV1::Microsoft => Provider::Microsoft,
        _ => return Err(CalendarError::InvalidInput),
    };
    let config = ClientConfig {
        provider,
        client_id: oauth.client_id.clone(),
        tenant: oauth.tenant.clone(),
    };
    config.validate().map_err(map_oauth)?;
    Ok(config)
}

/// Authenticated access to one account's provider API.
pub struct Api<'a> {
    /// Network port.
    pub http: &'a dyn HttpClient,
    /// Token port.
    pub tokens: &'a Arc<dyn TokenSessions>,
    /// The account id (the vault key).
    pub account_id: &'a str,
    /// The account's OAuth client.
    pub config: &'a ClientConfig,
}

impl Api<'_> {
    /// Sends `request` with a bearer token for [`Purpose::Calendar`]. After a
    /// 401 the cached token is dropped and the request is sent once more with
    /// a refreshed one; a second 401 is returned as it is.
    pub async fn call(&self, request: HttpRequest) -> Result<HttpResponse> {
        let mut retried = false;
        loop {
            let token = self
                .tokens
                .access_token(self.account_id, self.config, Purpose::Calendar)
                .await
                .map_err(map_oauth)?;
            let authorized = request
                .clone()
                .bearer(Some(Secret::new(token.as_str().to_owned())));
            let response = send(self.http, authorized).await?;
            if response.status == 401 && !retried {
                retried = true;
                self.tokens.invalidate(self.account_id).await;
                continue;
            }
            return Ok(response);
        }
    }
}

/// Parses a JSON body into `T`.
pub fn parse_json<T: DeserializeOwned>(response: &HttpResponse) -> Result<T> {
    serde_json::from_slice(&response.body).map_err(|_| CalendarError::Parse)
}

/// Percent-encodes one URL path segment.
pub fn encode_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A provider id is data from the network: it must be short and printable
/// before it becomes a cache key or a URL part.
pub fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 1_024 && id.chars().all(|c| !c.is_control())
}

/// Cuts `text` to at most `max` bytes on a character boundary.
pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// `#RRGGBB` for a provider colour, or `None`.
pub(crate) fn hex_color(text: &str) -> Option<String> {
    let digits = text.strip_prefix('#')?;
    (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", digits.to_ascii_uppercase()))
}
