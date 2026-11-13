//! Fork-owned mail engine (T-0020 to T-0032): IMAP reading and flags, MIME
//! parsing, SMTP sending, provider autoconfig and Credential Manager
//! passwords.
//!
//! Everything a server sends is untrusted input: every size is capped here
//! and every failure maps to the closed [`MailError`] without server text.
#![deny(missing_docs)]

pub mod auth;
pub mod autoconfig;
pub mod engine;
pub mod error;
pub mod imap;
pub mod limits;
pub mod mime;
pub mod model;
pub mod remote_images;
pub mod smtp;
pub mod transport;
pub mod utf7;
pub mod vault;

pub use error::{MailError, Result};
pub use futures::future::BoxFuture;
