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

/// Encodes a form as `application/x-www-form-urlencoded`. The body holds
/// secrets, so it is wiped on drop.
#[must_use]
pub fn encode_form(form: &[(String, String)]) -> zeroize::Zeroizing<String> {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in form {
        serializer.append_pair(key, value);
    }
    zeroize::Zeroizing::new(serializer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_are_percent_encoded() {
        let form = vec![
            ("code".to_owned(), "a b&c=d/é".to_owned()),
            ("x".to_owned(), "1".to_owned()),
        ];
        assert_eq!(&*encode_form(&form), "code=a+b%26c%3Dd%2F%C3%A9&x=1");
    }
}
