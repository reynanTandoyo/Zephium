//! Refresh tokens live only in the OS vault (Windows Credential Manager,
//! macOS Keychain) under `app.zephium.oauth|<account id>`. They never reach
//! the store, a log, a diagnostic or IPC output.
//!
//! One vault entry holds at most 2,560 bytes on Windows (4,096 on macOS), and
//! a Microsoft work-account record can be longer. A record that fits one chunk
//! is stored as before (the JSON in the account entry, so older builds and
//! older records still read). A longer one is split: the account entry holds a
//! small header (`ZCH2`, generation, chunk count, total length) and the bytes live
//! in `app.zephium.oauth|<account id>|<generation>|0` to `|<n-1>`. A new record
//! goes to the other generation and the header switches to it with one entry
//! write, so a failure at any point leaves one whole record (the old or the new).
//! Everything is written, read and removed within one vault turn. Records of
//! the first layout (`ZCH1`, chunks at `|<n>`) and single-entry records still read.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::{OAuthError, Result};

/// Prefix of every vault target this crate writes.
pub const SERVICE: &str = "app.zephium.oauth";
/// Longest vault record accepted.
pub const MAX_RECORD_BYTES: usize = 8 * 1024;
/// Bytes kept in one vault entry: below the smallest native blob limit
/// (Windows, 2,560), so one entry is always accepted.
pub const CHUNK_BYTES: usize = 2048;
/// Most chunk entries a record may use.
pub const MAX_CHUNKS: usize = MAX_RECORD_BYTES.div_ceil(CHUNK_BYTES);
/// Header of the first chunked layout: chunks at `<target>|<i>`.
const HEADER_V1: &[u8; 4] = b"ZCH1";
/// Header of the current layout: `ZCH2`, generation (0 or 1), chunk count,
/// total length; chunks at `<target>|<generation>|<i>`.
const HEADER_V2: &[u8; 4] = b"ZCH2";
const HEADER_V1_BYTES: usize = 4 + 1 + 4;
const HEADER_V2_BYTES: usize = 4 + 1 + 1 + 4;

/// What is kept per account: the refresh token and, for Google desktop
/// clients that need one, the client secret the person pasted.
#[derive(Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct StoredSecret {
    /// The refresh token.
    pub refresh_token: String,
    /// The client secret, when the provider requires one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

impl fmt::Debug for StoredSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StoredSecret(..)")
    }
}

impl Drop for StoredSecret {
    fn drop(&mut self) {
        self.refresh_token.zeroize();
        self.client_secret.zeroize();
    }
}

/// True for an account id: 1 to 40 characters of `[A-Za-z0-9-]`.
#[must_use]
pub fn id_ok(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The vault target for an account.
#[must_use]
pub fn target(account_id: &str) -> Option<String> {
    id_ok(account_id).then(|| format!("{SERVICE}|{account_id}"))
}

fn encode(secret: &StoredSecret) -> Result<Vec<u8>> {
    if secret.refresh_token.is_empty() {
        return Err(OAuthError::InvalidInput);
    }
    let bytes = serde_json::to_vec(secret).map_err(|_| OAuthError::Internal)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(OAuthError::InvalidInput);
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<StoredSecret> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(OAuthError::Credentials);
    }
    let secret: StoredSecret =
        serde_json::from_slice(bytes).map_err(|_| OAuthError::Credentials)?;
    if secret.refresh_token.is_empty() {
        return Err(OAuthError::Credentials);
    }
    Ok(secret)
}

/// One flat vault entry by target name: the part of the OS vault the chunking
/// needs. `delete` of a missing entry is success.
pub(crate) trait Entries {
    fn write(&self, target: &str, bytes: &[u8]) -> Result<()>;
    fn read(&self, target: &str) -> Result<zeroize::Zeroizing<Vec<u8>>>;
    fn delete(&self, target: &str) -> Result<()>;
}

fn chunk_target(account_id: &str, generation: u8, index: usize) -> Option<String> {
    target(account_id).map(|head| format!("{head}|{generation}|{index}"))
}

/// Chunk names of the first layout, still read and cleaned up.
fn chunk_target_v1(account_id: &str, index: usize) -> Option<String> {
    target(account_id).map(|head| format!("{head}|{index}"))
}

/// The generation the stored record uses, when it is chunked in the current
/// layout; `None` for a single-entry record, the first layout or no record.
fn stored_generation(entries: &dyn Entries, account_id: &str) -> Option<u8> {
    let head = target(account_id)?;
    let first = entries.read(&head).ok()?;
    (first.starts_with(HEADER_V2) && first.len() == HEADER_V2_BYTES).then(|| first[4] & 1)
}

/// Writes `bytes` (an encoded record) under `account_id`: one entry when it
/// fits, else a header plus chunks. A chunked record never overwrites the one
/// it replaces: the new chunks go to the other generation, then the header
/// switches to them in one entry write, and only then are the old chunks
/// removed. A failure before the switch leaves the old record whole; one
/// after it leaves the new record whole and some stale chunks that the next
/// write or the sign-out removes.
pub(crate) fn put_record(entries: &dyn Entries, account_id: &str, bytes: &[u8]) -> Result<()> {
    let head = target(account_id).ok_or(OAuthError::InvalidInput)?;
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(OAuthError::InvalidInput);
    }
    let previous = stored_generation(entries, account_id);
    let keep = if bytes.len() <= CHUNK_BYTES {
        entries.write(&head, bytes)?;
        None
    } else {
        let generation = previous.map_or(0, |old| old ^ 1);
        let chunks: Vec<&[u8]> = bytes.chunks(CHUNK_BYTES).collect();
        for (index, chunk) in chunks.iter().enumerate() {
            let name =
                chunk_target(account_id, generation, index).ok_or(OAuthError::InvalidInput)?;
            entries.write(&name, chunk)?;
        }
        let mut header = zeroize::Zeroizing::new(Vec::with_capacity(HEADER_V2_BYTES));
        header.extend_from_slice(HEADER_V2);
        header.push(generation);
        header.push(u8::try_from(chunks.len()).map_err(|_| OAuthError::InvalidInput)?);
        header.extend_from_slice(
            &u32::try_from(bytes.len())
                .map_err(|_| OAuthError::InvalidInput)?
                .to_be_bytes(),
        );
        entries.write(&head, &header)?;
        Some(generation)
    };
    // The switch is done: whatever the new record does not use is stale.
    let mut result = Ok(());
    for generation in 0..2_u8 {
        if keep == Some(generation) {
            continue;
        }
        for index in 0..MAX_CHUNKS {
            let name =
                chunk_target(account_id, generation, index).ok_or(OAuthError::InvalidInput)?;
            if let Err(error) = entries.delete(&name) {
                result = result.and(Err(error));
            }
        }
    }
    for index in 0..MAX_CHUNKS {
        let name = chunk_target_v1(account_id, index).ok_or(OAuthError::InvalidInput)?;
        if let Err(error) = entries.delete(&name) {
            result = result.and(Err(error));
        }
    }
    // A failed cleanup does not undo a record that was written whole.
    let _ = result;
    Ok(())
}

/// Reads the record bytes written by [`put_record`] or by an older build.
pub(crate) fn get_record(
    entries: &dyn Entries,
    account_id: &str,
) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    let head = target(account_id).ok_or(OAuthError::InvalidInput)?;
    let first = entries.read(&head)?;
    let (generation, count, total) = if first.starts_with(HEADER_V2) {
        if first.len() != HEADER_V2_BYTES || first[4] > 1 {
            return Err(OAuthError::Credentials);
        }
        (
            Some(first[4]),
            usize::from(first[5]),
            u32::from_be_bytes([first[6], first[7], first[8], first[9]]) as usize,
        )
    } else if first.starts_with(HEADER_V1) {
        if first.len() != HEADER_V1_BYTES {
            return Err(OAuthError::Credentials);
        }
        (
            None,
            usize::from(first[4]),
            u32::from_be_bytes([first[5], first[6], first[7], first[8]]) as usize,
        )
    } else {
        return Ok(first);
    };
    if count == 0 || count > MAX_CHUNKS || total == 0 || total > MAX_RECORD_BYTES {
        return Err(OAuthError::Credentials);
    }
    let mut all = zeroize::Zeroizing::new(Vec::with_capacity(total));
    for index in 0..count {
        let name = match generation {
            Some(generation) => chunk_target(account_id, generation, index),
            None => chunk_target_v1(account_id, index),
        }
        .ok_or(OAuthError::InvalidInput)?;
        let chunk = entries.read(&name)?;
        if chunk.len() > CHUNK_BYTES || all.len() + chunk.len() > total {
            return Err(OAuthError::Credentials);
        }
        all.extend_from_slice(&chunk);
    }
    if all.len() != total {
        return Err(OAuthError::Credentials);
    }
    Ok(all)
}

/// Removes the record and every chunk of every layout. Tries all entries;
/// reports the first failure.
pub(crate) fn remove_record(entries: &dyn Entries, account_id: &str) -> Result<()> {
    let head = target(account_id).ok_or(OAuthError::InvalidInput)?;
    let mut result = Ok(());
    let mut delete = |name: Option<String>| match name {
        Some(name) => {
            if let Err(error) = entries.delete(&name) {
                result = result.clone().and(Err(error));
            }
        }
        None => result = result.clone().and(Err(OAuthError::InvalidInput)),
    };
    for index in 0..MAX_CHUNKS {
        delete(chunk_target(account_id, 0, index));
        delete(chunk_target(account_id, 1, index));
        delete(chunk_target_v1(account_id, index));
    }
    delete(Some(head));
    result
}

/// The OS vault as flat entries.
#[cfg(any(windows, target_os = "macos"))]
struct OsEntries;

#[cfg(any(windows, target_os = "macos"))]
impl Entries for OsEntries {
    fn write(&self, target: &str, bytes: &[u8]) -> Result<()> {
        zephium_credentials::write(target, bytes)?;
        Ok(())
    }

    fn read(&self, target: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
        Ok(zephium_credentials::read(target)?)
    }

    fn delete(&self, target: &str) -> Result<()> {
        match zephium_credentials::delete(target) {
            Ok(()) | Err(zephium_credentials::VaultError::Missing) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// Where refresh tokens are kept.
pub trait RefreshVault: Send + Sync {
    /// Stores or replaces the secret.
    fn set(&self, account_id: &str, secret: &StoredSecret) -> Result<()>;
    /// Reads the secret.
    fn get(&self, account_id: &str) -> Result<StoredSecret>;
    /// Removes the secret; a missing one is success.
    fn remove(&self, account_id: &str) -> Result<()>;
}

/// The real vault.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsRefreshVault;

#[cfg(any(windows, target_os = "macos"))]
impl RefreshVault for OsRefreshVault {
    fn set(&self, account_id: &str, secret: &StoredSecret) -> Result<()> {
        target(account_id).ok_or(OAuthError::InvalidInput)?;
        let bytes = zeroize::Zeroizing::new(encode(secret)?);
        let _turn = zephium_credentials::turn()?;
        put_record(&OsEntries, account_id, &bytes)
    }

    fn get(&self, account_id: &str) -> Result<StoredSecret> {
        target(account_id).ok_or(OAuthError::InvalidInput)?;
        let _turn = zephium_credentials::turn()?;
        let bytes = get_record(&OsEntries, account_id)?;
        decode(&bytes)
    }

    fn remove(&self, account_id: &str) -> Result<()> {
        target(account_id).ok_or(OAuthError::InvalidInput)?;
        let _turn = zephium_credentials::turn()?;
        remove_record(&OsEntries, account_id)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl RefreshVault for OsRefreshVault {
    fn set(&self, _account_id: &str, _secret: &StoredSecret) -> Result<()> {
        Err(OAuthError::Unsupported)
    }

    fn get(&self, _account_id: &str) -> Result<StoredSecret> {
        Err(OAuthError::Unsupported)
    }

    fn remove(&self, _account_id: &str) -> Result<()> {
        Err(OAuthError::Unsupported)
    }
}

/// An in-memory vault for tests and fakes.
#[derive(Debug, Default)]
pub struct MemoryRefreshVault(Mutex<HashMap<String, Vec<u8>>>);

impl RefreshVault for MemoryRefreshVault {
    fn set(&self, account_id: &str, secret: &StoredSecret) -> Result<()> {
        target(account_id).ok_or(OAuthError::InvalidInput)?;
        let bytes = encode(secret)?;
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(account_id.to_owned(), bytes);
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<StoredSecret> {
        let guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        decode(guard.get(account_id).ok_or(OAuthError::Credentials)?)
    }

    fn remove(&self, account_id: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(account_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_names_the_service_and_rejects_bad_ids() {
        assert_eq!(target("abc-1").as_deref(), Some("app.zephium.oauth|abc-1"));
        assert_eq!(target("a|b"), None);
        assert_eq!(target(""), None);
    }

    #[test]
    fn memory_vault_round_trips_and_refuses_empty_tokens() {
        let vault = MemoryRefreshVault::default();
        assert!(vault.get("a1").is_err());
        let secret = StoredSecret {
            refresh_token: "rt".into(),
            client_secret: Some("cs".into()),
        };
        vault.set("a1", &secret).unwrap();
        assert_eq!(vault.get("a1").unwrap(), secret);
        let empty = StoredSecret {
            refresh_token: String::new(),
            client_secret: None,
        };
        assert_eq!(vault.set("a1", &empty), Err(OAuthError::InvalidInput));
        vault.remove("a1").unwrap();
        vault.remove("a1").unwrap();
        assert!(vault.get("a1").is_err());
        assert_eq!(format!("{secret:?}"), "StoredSecret(..)");
    }

    /// Flat entries that refuse a blob over the Windows limit, as
    /// `zephium_credentials::write` does.
    #[derive(Default)]
    struct WindowsSized(Mutex<HashMap<String, Vec<u8>>>);

    const WINDOWS_BLOB_MAX: usize = 2560;

    impl Entries for WindowsSized {
        fn write(&self, target: &str, bytes: &[u8]) -> Result<()> {
            if bytes.len() > WINDOWS_BLOB_MAX {
                return Err(OAuthError::Credentials);
            }
            self.0
                .lock()
                .unwrap()
                .insert(target.to_owned(), bytes.to_vec());
            Ok(())
        }

        fn read(&self, target: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
            self.0
                .lock()
                .unwrap()
                .get(target)
                .map(|bytes| zeroize::Zeroizing::new(bytes.clone()))
                .ok_or(OAuthError::Credentials)
        }

        fn delete(&self, target: &str) -> Result<()> {
            self.0.lock().unwrap().remove(target);
            Ok(())
        }
    }

    fn secret_with(token_len: usize) -> StoredSecret {
        StoredSecret {
            refresh_token: "r".repeat(token_len),
            client_secret: Some("GOCSPX-secret".into()),
        }
    }

    #[test]
    fn a_6_kib_refresh_token_is_split_over_entries_and_read_back() {
        let entries = WindowsSized::default();
        let secret = secret_with(6 * 1024);
        let bytes = encode(&secret).unwrap();
        assert!(bytes.len() > WINDOWS_BLOB_MAX);
        put_record(&entries, "acct-1", &bytes).unwrap();
        {
            let map = entries.0.lock().unwrap();
            assert!(map.contains_key("app.zephium.oauth|acct-1"));
            assert!(map.contains_key("app.zephium.oauth|acct-1|0|0"));
            assert!(map.contains_key("app.zephium.oauth|acct-1|0|2"));
            assert!(map.values().all(|blob| blob.len() <= WINDOWS_BLOB_MAX));
        }
        assert_eq!(
            decode(&get_record(&entries, "acct-1").unwrap()).unwrap(),
            secret
        );
    }

    #[test]
    fn a_whole_unsplit_record_would_be_refused_by_the_windows_limit() {
        // The check the fake performs is the one that failed before the split.
        let entries = WindowsSized::default();
        let bytes = encode(&secret_with(6 * 1024)).unwrap();
        assert_eq!(
            entries.write("app.zephium.oauth|acct-1", &bytes),
            Err(OAuthError::Credentials)
        );
    }

    #[test]
    fn a_short_record_keeps_the_old_single_entry_format() {
        let entries = WindowsSized::default();
        let secret = secret_with(40);
        put_record(&entries, "acct-1", &encode(&secret).unwrap()).unwrap();
        let map = entries.0.lock().unwrap();
        assert_eq!(map.len(), 1);
        // The entry is the bare JSON an older build wrote and reads.
        assert_eq!(decode(&map["app.zephium.oauth|acct-1"]).unwrap(), secret);
        drop(map);
        assert_eq!(
            decode(&get_record(&entries, "acct-1").unwrap()).unwrap(),
            secret
        );
    }

    #[test]
    fn shrinking_or_removing_a_record_leaves_no_chunk_behind() {
        let entries = WindowsSized::default();
        put_record(&entries, "acct-1", &encode(&secret_with(7000)).unwrap()).unwrap();
        assert!(entries.0.lock().unwrap().len() > 2);
        let small = secret_with(30);
        put_record(&entries, "acct-1", &encode(&small).unwrap()).unwrap();
        assert_eq!(entries.0.lock().unwrap().len(), 1);
        assert_eq!(
            decode(&get_record(&entries, "acct-1").unwrap()).unwrap(),
            small
        );
        put_record(&entries, "acct-1", &encode(&secret_with(6000)).unwrap()).unwrap();
        put_record(&entries, "acct-2", &encode(&small).unwrap()).unwrap();
        remove_record(&entries, "acct-1").unwrap();
        remove_record(&entries, "acct-1").unwrap();
        let map = entries.0.lock().unwrap();
        assert_eq!(map.keys().collect::<Vec<_>>(), ["app.zephium.oauth|acct-2"]);
    }

    #[test]
    fn damaged_or_oversized_chunked_records_are_refused() {
        let entries = WindowsSized::default();
        let bytes = encode(&secret_with(6000)).unwrap();
        put_record(&entries, "acct-1", &bytes).unwrap();
        // A missing chunk.
        entries
            .0
            .lock()
            .unwrap()
            .remove("app.zephium.oauth|acct-1|0|1");
        assert!(get_record(&entries, "acct-1").is_err());
        // A short chunk (length disagrees with the header); the second write
        // went to the other generation.
        put_record(&entries, "acct-1", &bytes).unwrap();
        entries
            .0
            .lock()
            .unwrap()
            .insert("app.zephium.oauth|acct-1|1|1".into(), vec![b'x'; 10]);
        assert_eq!(
            get_record(&entries, "acct-1").unwrap_err(),
            OAuthError::Credentials
        );
        // Headers that claim too much.
        for header in [
            b"ZCH2\x00\x00\x00\x00\x00\x10".to_vec(),
            b"ZCH2\x00\x09\x00\x00\x20\x00".to_vec(),
            b"ZCH2\x00\x02\x00\x10\x00\x00".to_vec(),
            b"ZCH2\x05\x02\x00\x00\x10\x00".to_vec(),
            b"ZCH2\x00\x02".to_vec(),
            b"ZCH1\x02".to_vec(),
        ] {
            entries
                .0
                .lock()
                .unwrap()
                .insert("app.zephium.oauth|acct-1".into(), header);
            assert_eq!(
                get_record(&entries, "acct-1").unwrap_err(),
                OAuthError::Credentials
            );
        }
        // Writing more than the record bound is refused before any entry is written.
        let fresh = WindowsSized::default();
        assert_eq!(
            put_record(&fresh, "acct-9", &vec![b'a'; MAX_RECORD_BYTES + 1]),
            Err(OAuthError::InvalidInput)
        );
        assert!(fresh.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_record_of_the_first_chunked_layout_still_reads_and_is_replaced_cleanly() {
        let entries = WindowsSized::default();
        let secret = secret_with(5000);
        let bytes = encode(&secret).unwrap();
        // Write it the way the first layout did: `ZCH1`, chunks at `|<i>`.
        let mut header = b"ZCH1".to_vec();
        header.push(u8::try_from(bytes.len().div_ceil(CHUNK_BYTES)).unwrap());
        header.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        entries.write("app.zephium.oauth|acct-1", &header).unwrap();
        for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
            entries
                .write(&format!("app.zephium.oauth|acct-1|{index}"), chunk)
                .unwrap();
        }
        assert_eq!(
            decode(&get_record(&entries, "acct-1").unwrap()).unwrap(),
            secret
        );
        let next = secret_with(6000);
        put_record(&entries, "acct-1", &encode(&next).unwrap()).unwrap();
        assert_eq!(
            decode(&get_record(&entries, "acct-1").unwrap()).unwrap(),
            next
        );
        assert!(!entries
            .0
            .lock()
            .unwrap()
            .contains_key("app.zephium.oauth|acct-1|0"));
        remove_record(&entries, "acct-1").unwrap();
        assert!(entries.0.lock().unwrap().is_empty());
    }

    /// Entries that start failing after `budget` writes or deletes.
    struct Flaky {
        inner: WindowsSized,
        budget: Mutex<usize>,
    }

    impl Flaky {
        fn spend(&self) -> Result<()> {
            let mut budget = self.budget.lock().unwrap();
            if *budget == 0 {
                return Err(OAuthError::Credentials);
            }
            *budget -= 1;
            Ok(())
        }
    }

    impl Entries for Flaky {
        fn write(&self, target: &str, bytes: &[u8]) -> Result<()> {
            self.spend()?;
            self.inner.write(target, bytes)
        }

        fn read(&self, target: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
            self.inner.read(target)
        }

        fn delete(&self, target: &str) -> Result<()> {
            self.spend()?;
            self.inner.delete(target)
        }
    }

    #[test]
    fn a_failure_at_any_point_of_a_replacement_leaves_one_whole_record() {
        let old = secret_with(6000);
        let new = secret_with(7000);
        for budget in 0..16 {
            // Chunked over chunked, and single-entry over chunked.
            for replacement in [&new, &secret_with(40)] {
                let flaky = Flaky {
                    inner: WindowsSized::default(),
                    budget: Mutex::new(usize::MAX),
                };
                put_record(&flaky, "acct-1", &encode(&old).unwrap()).unwrap();
                *flaky.budget.lock().unwrap() = budget;
                let _ = put_record(&flaky, "acct-1", &encode(replacement).unwrap());
                let read = decode(&get_record(&flaky, "acct-1").unwrap())
                    .unwrap_or_else(|_| panic!("unreadable after a failure at step {budget}"));
                assert!(
                    read == old || read == *replacement,
                    "mixed record at step {budget}"
                );
                // A later write with a working vault leaves no stale chunk.
                *flaky.budget.lock().unwrap() = usize::MAX;
                put_record(&flaky, "acct-1", &encode(&new).unwrap()).unwrap();
                assert_eq!(decode(&get_record(&flaky, "acct-1").unwrap()).unwrap(), new);
                let count = flaky.inner.0.lock().unwrap().len();
                assert_eq!(
                    count,
                    1 + new_chunks(&new),
                    "stale entries after step {budget}"
                );
                remove_record(&flaky, "acct-1").unwrap();
                assert!(flaky.inner.0.lock().unwrap().is_empty());
            }
        }
    }

    fn new_chunks(secret: &StoredSecret) -> usize {
        encode(secret).unwrap().len().div_ceil(CHUNK_BYTES)
    }

    #[test]
    fn corrupt_records_are_credential_errors() {
        assert_eq!(decode(b"not json").unwrap_err(), OAuthError::Credentials);
        assert_eq!(
            decode(br#"{"refresh_token":""}"#).unwrap_err(),
            OAuthError::Credentials
        );
    }
}
