//! The HTTP port. The desktop supplies a `reqwest` client; tests supply a
//! fake. Only form POSTs to the provider's token endpoint go through it.

use crate::error::Result;
use crate::BoxFuture;

/// Longest token endpoint body read.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// One response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Body, at most [`MAX_BODY_BYTES`].
    pub body: Vec<u8>,
}

/// Posts `application/x-www-form-urlencoded` forms over HTTPS.
pub trait HttpClient: Send + Sync {
    /// Posts `form` to `url`. Implementations refuse non-HTTPS URLs, follow
    /// no redirects, bound time and body size, and fail with
    /// [`crate::OAuthError::Offline`] or [`crate::OAuthError::Timeout`].
    fn post_form<'a>(
        &'a self,
        url: &'a str,
        form: Vec<(String, String)>,
    ) -> BoxFuture<'a, Result<HttpResponse>>;
}
