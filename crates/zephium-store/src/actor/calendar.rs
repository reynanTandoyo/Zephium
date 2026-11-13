//! Fork-owned: the calendar cache as seen from outside the actor thread.

use super::{Cmd, SqliteStore};
use crate::calendar_cache::CalendarCache;

impl SqliteStore {
    /// Runs `job` against the calendar cache on the storage thread and returns
    /// its result, or `None` when the store is not admitting work, is in
    /// recovery, or did not answer in time.
    pub fn with_calendar_cache<R: Send + 'static>(
        &self,
        job: impl FnOnce(&mut CalendarCache<'_>) -> R + Send + 'static,
    ) -> Option<R> {
        self.ask(|reply| {
            Cmd::Calendar(Box::new(move |hub| {
                let _ = reply.send(hub.with_calendar_cache(job));
            }))
        })
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar_cache::{CalendarAccountRecord, CalendarRecord};

    #[test]
    fn the_calendar_cache_is_reachable_through_the_actor_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = vec![CalendarAccountRecord {
            id: "A1".into(),
            kind: "subscription".into(),
            name: "Holidays".into(),
            server_url: "https://example.com/h.ics".into(),
            username: None,
        }];
        let calendars = vec![CalendarRecord {
            id: "C1".into(),
            account_id: "A1".into(),
            href: "https://example.com/h.ics".into(),
            display_name: "Holidays".into(),
            colour: None,
            read_only: true,
            visible: true,
        }];
        {
            let store = SqliteStore::open(dir.path()).unwrap();
            let (a, c) = (accounts.clone(), calendars.clone());
            assert_eq!(
                store.with_calendar_cache(move |cache| cache.replace_directory(&a, &c).unwrap()),
                Some(true)
            );
        }
        let store = SqliteStore::open(dir.path()).unwrap();
        assert_eq!(
            store.with_calendar_cache(|cache| cache.calendars().unwrap()),
            Some(calendars)
        );
        assert_eq!(
            store.with_calendar_cache(|cache| cache.accounts().unwrap()),
            Some(accounts)
        );
    }
}
