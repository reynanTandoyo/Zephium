//! Fork-owned: the mail cache as seen from outside the actor thread.

use super::{Cmd, SqliteStore};
use crate::mail_cache::MailCache;

impl SqliteStore {
    /// Runs `job` against the mail cache on the storage thread and returns its
    /// result, or `None` when the store is not admitting work, is in recovery,
    /// or did not answer in time.
    pub fn with_mail_cache<R: Send + 'static>(
        &self,
        job: impl FnOnce(&mut MailCache<'_>) -> R + Send + 'static,
    ) -> Option<R> {
        self.ask(|reply| {
            Cmd::Mail(Box::new(move |hub| {
                let _ = reply.send(hub.with_mail_cache(job));
            }))
        })
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_cache::MailAccountRecord;

    #[test]
    fn the_mail_cache_is_reachable_through_the_actor_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let account = MailAccountRecord {
            id: "a1".into(),
            display_name: "Ann".into(),
            email: "ann@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: "tls".into(),
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            smtp_security: "tls".into(),
            username: "ann".into(),
            colour: "#336699".into(),
            signature: String::new(),
            save_sent_copy: true,
        };
        {
            let store = SqliteStore::open(dir.path()).unwrap();
            let saved = account.clone();
            assert_eq!(
                store.with_mail_cache(move |cache| cache.save_account(&saved).unwrap()),
                Some(true)
            );
            assert_eq!(
                store.with_mail_cache(|cache| cache.accounts().unwrap()),
                Some(vec![account.clone()])
            );
        }
        let store = SqliteStore::open(dir.path()).unwrap();
        assert_eq!(
            store.with_mail_cache(|cache| cache.accounts().unwrap()),
            Some(vec![account])
        );
    }
}
