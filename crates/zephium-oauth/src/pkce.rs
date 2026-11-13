//! PKCE (RFC 7636) and the random `state` value.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{OAuthError, Result};

/// Random bytes behind a verifier: 32 bytes give a 43 character verifier,
/// the shortest RFC 7636 allows, with 256 bits of entropy.
const VERIFIER_BYTES: usize = 32;
/// Random bytes behind the `state` value.
const STATE_BYTES: usize = 24;

fn random_text(bytes: usize) -> Result<String> {
    let mut buffer = Zeroizing::new(vec![0_u8; bytes]);
    getrandom::fill(&mut buffer).map_err(|_| OAuthError::Internal)?;
    Ok(URL_SAFE_NO_PAD.encode(&*buffer))
}

/// A fresh code verifier.
pub fn verifier() -> Result<Zeroizing<String>> {
    random_text(VERIFIER_BYTES).map(Zeroizing::new)
}

/// The `S256` challenge for a verifier: `BASE64URL(SHA256(verifier))`.
#[must_use]
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A fresh unguessable `state` value.
pub fn state() -> Result<String> {
    random_text(STATE_BYTES)
}

/// A random opaque handle of `bytes` random bytes, URL-safe.
pub fn handle(bytes: usize) -> Result<String> {
    random_text(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_the_rfc_7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_is_43_unreserved_characters_and_fresh() {
        let first = verifier().unwrap();
        let second = verifier().unwrap();
        assert_eq!(first.len(), 43);
        assert!(first
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_ne!(*first, *second);
    }

    #[test]
    fn state_is_long_and_fresh() {
        let first = state().unwrap();
        assert_eq!(first.len(), 32);
        assert_ne!(first, state().unwrap());
    }
}
