//! Fork-owned: gives the actor access to the calendar cache in the meta database.

use super::*;
use crate::calendar_cache::CalendarCache;

impl Hub {
    /// Runs `job` over the calendar cache, or returns `None` while the store
    /// is in read-only recovery.
    pub(crate) fn with_calendar_cache<R>(
        &mut self,
        job: impl FnOnce(&mut CalendarCache<'_>) -> R,
    ) -> Option<R> {
        if self.recovery_required.is_some() {
            return None;
        }
        Some(job(&mut CalendarCache::new(&mut self.meta)))
    }
}
