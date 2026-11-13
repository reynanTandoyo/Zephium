//! Fork-owned OAuth 2.0 sign-in (T-0034, prepares T-0040): the authorization
//! code flow with PKCE for native public clients and a loopback redirect
//! (RFC 8252), token exchange and refresh for Microsoft and Google.
//!
//! The person supplies the client ID; nothing is committed here. Refresh
//! tokens live only in the OS vault under `app.zephium.oauth|<account id>`;
//! access tokens live only in memory. Everything a server or a browser sends
//! is untrusted input: every size is capped and every failure maps to the
//! closed [`OAuthError`] without server text.
#![deny(missing_docs)]

pub mod error;
pub mod flow;
pub mod http;
pub mod loopback;
pub mod manager;
pub mod pkce;
pub mod provider;
pub mod token;
pub mod vault;

pub use error::{OAuthError, Result};
pub use futures::future::BoxFuture;
