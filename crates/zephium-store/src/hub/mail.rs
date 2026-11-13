//! Fork-owned: gives the actor access to the mail cache in the meta database.

use super::*;
use crate::mail_cache::MailCache;

impl Hub {
    /// Runs `job` over the mail cache, or returns `None` while the store is
    /// in read-only recovery.
    pub(crate) fn with_mail_cache<R>(
        &mut self,
        job: impl FnOnce(&mut MailCache<'_>) -> R,
    ) -> Option<R> {
        if self.recovery_required.is_some() {
            return None;
        }
        Some(job(&mut MailCache::new(&mut self.meta)))
    }
}
