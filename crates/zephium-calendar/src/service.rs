//! Orchestrates accounts, subscriptions, the event cache (in memory, backed by
//! a durable [`EventCache`]), sync and event changes. Network, storage, cache,
//! secrets and the clock are injected.

use crate::api::{
    apply_draft, occurrence_wire, parse_draft_time, CalendarAccountKindV1, CalendarAccountV1,
    CalendarDiscoveryV1, CalendarEventDraftV1, CalendarEventRefV1, CalendarEventsV1,
    CalendarSyncOutcomeV1, CalendarSyncReportV1, CalendarV1,
};
use crate::caldav::{self, CaldavClient};
use crate::error::{CalendarError, Result};
use crate::http::{
    check_status, parse_address, send, Credentials, HttpClient, HttpRequest, Method, Secret,
};
use crate::ical;
use crate::model::{Event, EventTime, TimeKind};
use crate::recur::{self, MAX_OCCURRENCES_PER_EVENT};
use crate::store::{
    CacheMeta, CachedRecord, CalendarStorage, EventCache, MemoryEventCache, SecretStore,
    StoredState,
};
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use url::Url;

/// Most occurrences returned by one query.
pub const MAX_OCCURRENCES_PER_QUERY: usize = 5_000;
/// Longest range one query may cover.
pub const MAX_QUERY_DAYS: i64 = 366 * 5;
/// Most accounts and subscriptions together.
pub const MAX_ACCOUNTS: usize = 20;
/// Window read by a full CalDAV refresh: past and future.
const SYNC_PAST_DAYS: i64 = 180;
const SYNC_FUTURE_DAYS: i64 = 730;
/// How old a calendar may get before a periodic refresh re-reads it.
const CALDAV_STALE_MINUTES: i64 = 15;
const SUBSCRIPTION_STALE_MINUTES: i64 = 60;

const PALETTE: [&str; 8] = [
    "#4285F4", "#0B8043", "#E67C73", "#F6BF26", "#8E24AA", "#039BE5", "#F4511E", "#616161",
];

#[derive(Clone, Debug)]
struct CachedResource {
    etag: Option<String>,
    events: Vec<Event>,
    /// The iCalendar text the events were parsed from, when known.
    raw: Option<String>,
}

#[derive(Default)]
struct CalendarCache {
    resources: HashMap<String, CachedResource>,
    ctag: Option<String>,
    sync_token: Option<String>,
    synced_at: Option<DateTime<Utc>>,
}

struct Inner {
    state: StoredState,
    caches: HashMap<String, CalendarCache>,
    /// The durable cache has been read into `caches`.
    hydrated: bool,
}

/// Source of the current time.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The calendar backend.
pub struct CalendarService {
    http: Arc<dyn HttpClient>,
    storage: Arc<dyn CalendarStorage>,
    secrets: Arc<dyn SecretStore>,
    cache: Arc<dyn EventCache>,
    clock: Clock,
    inner: Mutex<Inner>,
    sync_gate: tokio::sync::Mutex<()>,
}

fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

/// A resource the server returned: its etag only counts when its data was
/// read, so a listing without data is fetched again by the next refresh.
fn cached_from(resource: caldav::RemoteResource) -> (String, CachedResource) {
    let etag = if resource.raw.is_some() {
        resource.etag
    } else {
        None
    };
    (
        resource.href.to_string(),
        CachedResource {
            etag,
            events: resource.events,
            raw: resource.raw,
        },
    )
}

impl CalendarService {
    /// Loads saved accounts; events start empty until the first sync.
    pub fn new(
        http: Arc<dyn HttpClient>,
        storage: Arc<dyn CalendarStorage>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self> {
        Self::with_clock(http, storage, secrets, Arc::new(Utc::now))
    }

    /// [`Self::new`] with an explicit clock.
    pub fn with_clock(
        http: Arc<dyn HttpClient>,
        storage: Arc<dyn CalendarStorage>,
        secrets: Arc<dyn SecretStore>,
        clock: Clock,
    ) -> Result<Self> {
        Self::with_cache(
            http,
            storage,
            secrets,
            Arc::new(MemoryEventCache::default()),
            clock,
        )
    }

    /// [`Self::with_clock`] with a durable event cache: events and refresh
    /// state saved there are read on first use, so they survive a restart.
    pub fn with_cache(
        http: Arc<dyn HttpClient>,
        storage: Arc<dyn CalendarStorage>,
        secrets: Arc<dyn SecretStore>,
        cache: Arc<dyn EventCache>,
        clock: Clock,
    ) -> Result<Self> {
        let state = storage.load()?;
        Ok(Self {
            http,
            storage,
            secrets,
            cache,
            clock,
            inner: Mutex::new(Inner {
                state,
                caches: HashMap::new(),
                hydrated: false,
            }),
            sync_gate: tokio::sync::Mutex::new(()),
        })
    }

    /// Reads the durable cache now instead of on first use.
    pub fn warm(&self) {
        drop(self.lock());
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !inner.hydrated {
            inner.hydrated = true;
            self.hydrate(&mut inner);
        }
        inner
    }

    /// Fills the in-memory cache from the durable one. A calendar whose rows
    /// cannot be read stays empty and is fetched again by the next refresh.
    fn hydrate(&self, inner: &mut Inner) {
        let ids: Vec<String> = inner
            .state
            .calendars
            .iter()
            .map(|calendar| calendar.id.clone())
            .collect();
        for id in ids {
            let Ok(Some((meta, records))) = self.cache.load(&id) else {
                continue;
            };
            if meta.synced_at.is_none() {
                continue;
            }
            let resources = records
                .into_iter()
                .map(|record| {
                    let events = ical::parse_calendar(&record.raw).unwrap_or_default();
                    (
                        record.href,
                        CachedResource {
                            etag: record.etag,
                            events,
                            raw: Some(record.raw),
                        },
                    )
                })
                .collect();
            inner.caches.insert(
                id,
                CalendarCache {
                    resources,
                    ctag: meta.ctag,
                    sync_token: meta.sync_token,
                    synced_at: meta.synced_at,
                },
            );
        }
    }

    fn record(&self, href: &str, resource: &CachedResource) -> Option<CachedRecord> {
        Some(CachedRecord {
            href: href.to_owned(),
            etag: resource.etag.clone(),
            raw: resource.raw.clone()?,
            last_seen: self.now(),
        })
    }

    /// Writes refresh state to the durable cache. A failure only costs a
    /// re-sync, so it never fails the operation.
    fn save_meta(&self, calendar_id: &str, cache: &CalendarCache) {
        let _ = self.cache.set_meta(
            calendar_id,
            &CacheMeta {
                ctag: cache.ctag.clone(),
                sync_token: cache.sync_token.clone(),
                synced_at: cache.synced_at,
            },
        );
    }

    /// Writes changed resources and removals, then the refresh state.
    fn save_delta(
        &self,
        calendar_id: &str,
        cache: &CalendarCache,
        changed: &[String],
        removed: &[String],
    ) {
        let upserts: Vec<CachedRecord> = changed
            .iter()
            .filter_map(|href| self.record(href, cache.resources.get(href)?))
            .collect();
        let _ = self.cache.apply(calendar_id, &upserts, removed);
        self.save_meta(calendar_id, cache);
    }

    /// Writes every resource of a calendar, then the refresh state.
    fn save_all(&self, calendar_id: &str, cache: &CalendarCache) {
        let records: Vec<CachedRecord> = cache
            .resources
            .iter()
            .filter_map(|(href, resource)| self.record(href, resource))
            .collect();
        let _ = self.cache.replace(calendar_id, &records);
        self.save_meta(calendar_id, cache);
    }

    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    fn persist(&self, state: &StoredState) -> Result<()> {
        self.storage.save(state)
    }

    fn credentials_for(&self, account: &CalendarAccountV1) -> Result<Option<Credentials>> {
        if account.kind != CalendarAccountKindV1::Caldav {
            return Ok(None);
        }
        let password = self
            .secrets
            .get(&account.id)?
            .ok_or(CalendarError::Unauthorized)?;
        Ok(Some(Credentials {
            username: account.username.clone().unwrap_or_default(),
            password,
        }))
    }

    // -- discovery and accounts ---------------------------------------------

    /// Finds the CalDAV entry point for an email address or URL.
    pub async fn discover(&self, email_or_url: &str) -> Result<CalendarDiscoveryV1> {
        let url = caldav::discover(self.http.as_ref(), email_or_url).await?;
        let input = email_or_url.trim();
        Ok(CalendarDiscoveryV1 {
            server_url: url.to_string(),
            username: if input.contains('@') && !input.contains("://") {
                input.to_owned()
            } else {
                String::new()
            },
        })
    }

    /// Signs in to a CalDAV server, stores the password in the vault and
    /// records the account with its calendars. Events load on the next sync.
    pub async fn add_account(
        &self,
        server_url: &str,
        username: &str,
        password: Secret,
        name: Option<&str>,
    ) -> Result<CalendarAccountV1> {
        let server = parse_address(server_url)?;
        let username = username.trim();
        if username.is_empty() || username.len() > 320 || password.expose().is_empty() {
            return Err(CalendarError::InvalidInput);
        }
        {
            let inner = self.lock();
            if inner.state.accounts.len() >= MAX_ACCOUNTS
                || inner.state.accounts.iter().any(|account| {
                    account.server_url == server.as_str()
                        && account.username.as_deref() == Some(username)
                })
            {
                return Err(CalendarError::InvalidInput);
            }
        }
        let credentials = Credentials {
            username: username.to_owned(),
            password: password.clone(),
        };
        let client = CaldavClient::new(self.http.as_ref(), Some(credentials));
        let (_, remote) = client.connect(&server).await?;
        let id = new_id();
        self.secrets.set(&id, &password)?;
        let account = CalendarAccountV1 {
            id: id.clone(),
            kind: CalendarAccountKindV1::Caldav,
            name: name
                .map(|value| value.trim().chars().take(100).collect::<String>())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| server.host_str().unwrap_or("CalDAV").to_owned()),
            server_url: server.to_string(),
            username: Some(username.to_owned()),
        };
        let result = {
            let mut inner = self.lock();
            let mut next = inner.state.clone();
            next.accounts.push(account.clone());
            for (index, calendar) in remote.iter().enumerate() {
                next.calendars.push(CalendarV1 {
                    id: new_id(),
                    account_id: id.clone(),
                    url: calendar.url.to_string(),
                    name: calendar.name.clone(),
                    color: calendar
                        .color
                        .clone()
                        .or_else(|| Some(PALETTE[index % PALETTE.len()].to_owned())),
                    read_only: calendar.read_only,
                    visible: true,
                });
            }
            self.persist(&next).map(|()| inner.state = next)
        };
        if let Err(error) = result {
            let _ = self.secrets.delete(&id);
            return Err(error);
        }
        Ok(account)
    }

    /// Adds a read-only calendar from an `https`/`webcal` address.
    pub async fn add_subscription(&self, url: &str, name: &str) -> Result<CalendarAccountV1> {
        let address = parse_address(url)?;
        {
            let inner = self.lock();
            if inner.state.accounts.len() >= MAX_ACCOUNTS
                || inner
                    .state
                    .accounts
                    .iter()
                    .any(|account| account.server_url == address.as_str())
            {
                return Err(CalendarError::InvalidInput);
            }
        }
        let text = self.fetch_feed(&address).await?;
        let events = ical::parse_calendar(&text)?;
        let (feed_name, feed_color) = ical::calendar_properties(&text);
        let name: String = Some(name.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or(feed_name)
            .unwrap_or_else(|| address.host_str().unwrap_or("Calendar").to_owned())
            .chars()
            .take(100)
            .collect();
        let account = CalendarAccountV1 {
            id: new_id(),
            kind: CalendarAccountKindV1::Subscription,
            name: name.clone(),
            server_url: address.to_string(),
            username: None,
        };
        let calendar = CalendarV1 {
            id: new_id(),
            account_id: account.id.clone(),
            url: address.to_string(),
            name,
            color: feed_color.or_else(|| Some(PALETTE[0].to_owned())),
            read_only: true,
            visible: true,
        };
        let mut inner = self.lock();
        let mut next = inner.state.clone();
        next.accounts.push(account.clone());
        next.calendars.push(calendar.clone());
        self.persist(&next)?;
        inner.state = next;
        let mut cache = CalendarCache::default();
        cache.resources.insert(
            address.to_string(),
            CachedResource {
                etag: None,
                events,
                raw: Some(text),
            },
        );
        cache.synced_at = Some(self.now());
        self.save_all(&calendar.id, &cache);
        inner.caches.insert(calendar.id, cache);
        Ok(account)
    }

    /// Accounts and subscriptions. Never includes passwords.
    pub fn accounts(&self) -> Vec<CalendarAccountV1> {
        self.lock().state.accounts.clone()
    }

    /// Removes an account, its calendars, cached events and vault entry.
    pub fn remove_account(&self, account_id: &str) -> Result<()> {
        let mut inner = self.lock();
        if !inner.state.accounts.iter().any(|a| a.id == account_id) {
            return Err(CalendarError::NotFound);
        }
        let mut next = inner.state.clone();
        next.accounts.retain(|account| account.id != account_id);
        let gone: Vec<String> = next
            .calendars
            .iter()
            .filter(|calendar| calendar.account_id == account_id)
            .map(|calendar| calendar.id.clone())
            .collect();
        next.calendars
            .retain(|calendar| calendar.account_id != account_id);
        // The durable storage drops the calendars' cached events with them.
        self.persist(&next)?;
        inner.state = next;
        for id in gone {
            inner.caches.remove(&id);
        }
        drop(inner);
        // The account is already gone; a vault failure leaves only an orphan
        // entry, which must not undo the removal.
        let _ = self.secrets.delete(account_id);
        Ok(())
    }

    /// All calendars.
    pub fn calendars(&self) -> Vec<CalendarV1> {
        self.lock().state.calendars.clone()
    }

    /// Shows or hides a calendar.
    pub fn set_visible(&self, calendar_id: &str, visible: bool) -> Result<()> {
        let mut inner = self.lock();
        let mut next = inner.state.clone();
        let calendar = next
            .calendars
            .iter_mut()
            .find(|calendar| calendar.id == calendar_id)
            .ok_or(CalendarError::NotFound)?;
        calendar.visible = visible;
        self.persist(&next)?;
        inner.state = next;
        Ok(())
    }

    // -- reading --------------------------------------------------------------

    /// Occurrences overlapping `[from, to)` in visible calendars.
    pub fn events(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<CalendarEventsV1> {
        if to <= from || to - from > Duration::days(MAX_QUERY_DAYS) {
            return Err(CalendarError::InvalidInput);
        }
        let inner = self.lock();
        let mut out = CalendarEventsV1::default();
        let mut keyed: Vec<(DateTime<Utc>, crate::api::CalendarOccurrenceV1)> = Vec::new();
        'calendars: for calendar in inner.state.calendars.iter().filter(|c| c.visible) {
            let Some(cache) = inner.caches.get(&calendar.id) else {
                continue;
            };
            for resource in cache.resources.values() {
                for event in &resource.events {
                    let budget = MAX_OCCURRENCES_PER_QUERY - keyed.len();
                    let expansion =
                        recur::expand(event, from, to, budget.min(MAX_OCCURRENCES_PER_EVENT));
                    out.truncated |= expansion.truncated;
                    for occurrence in &expansion.occurrences {
                        if keyed.len() >= MAX_OCCURRENCES_PER_QUERY {
                            out.truncated = true;
                            break 'calendars;
                        }
                        keyed.push((
                            occurrence.start.to_utc(),
                            occurrence_wire(
                                occurrence,
                                &calendar.id,
                                calendar.color.as_deref(),
                                calendar.read_only,
                                event.is_recurring(),
                            ),
                        ));
                    }
                }
            }
        }
        keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.summary.cmp(&b.1.summary)));
        out.occurrences = keyed.into_iter().map(|(_, item)| item).collect();
        Ok(out)
    }

    // -- sync -----------------------------------------------------------------

    async fn fetch_feed(&self, url: &Url) -> Result<String> {
        let request = HttpRequest::new(Method::Get, url.clone())
            .header("accept", "text/calendar, text/plain;q=0.5");
        let response = send(self.http.as_ref(), request).await?;
        check_status(response.status)?;
        if response.body.len() > ical::MAX_ICS_BYTES {
            return Err(CalendarError::TooLarge);
        }
        Ok(response.text()?.to_owned())
    }

    /// Refreshes every calendar.
    pub async fn sync_all(&self) -> CalendarSyncReportV1 {
        self.sync_matching(false).await
    }

    /// Refreshes calendars that were not read recently (15 minutes for
    /// CalDAV, one hour for subscriptions).
    pub async fn sync_due(&self) -> CalendarSyncReportV1 {
        self.sync_matching(true).await
    }

    async fn sync_matching(&self, only_due: bool) -> CalendarSyncReportV1 {
        let _gate = self.sync_gate.lock().await;
        let now = self.now();
        let targets: Vec<String> = {
            let inner = self.lock();
            inner
                .state
                .calendars
                .iter()
                .filter(|calendar| {
                    if !only_due {
                        return true;
                    }
                    let stale = inner
                        .state
                        .accounts
                        .iter()
                        .find(|account| account.id == calendar.account_id)
                        .map_or(CALDAV_STALE_MINUTES, |account| match account.kind {
                            CalendarAccountKindV1::Caldav => CALDAV_STALE_MINUTES,
                            CalendarAccountKindV1::Subscription => SUBSCRIPTION_STALE_MINUTES,
                        });
                    inner
                        .caches
                        .get(&calendar.id)
                        .and_then(|cache| cache.synced_at)
                        .is_none_or(|at| now - at >= Duration::minutes(stale))
                })
                .map(|calendar| calendar.id.clone())
                .collect()
        };
        let mut report = CalendarSyncReportV1::default();
        for id in targets {
            let error = self.sync_one(&id, !only_due).await.err();
            report.outcomes.push(CalendarSyncOutcomeV1 {
                calendar_id: id,
                error,
            });
        }
        report
    }

    async fn sync_one(&self, calendar_id: &str, force: bool) -> Result<()> {
        let (calendar, account) = {
            let inner = self.lock();
            let calendar = inner
                .state
                .calendars
                .iter()
                .find(|calendar| calendar.id == calendar_id)
                .cloned()
                .ok_or(CalendarError::NotFound)?;
            let account = inner
                .state
                .accounts
                .iter()
                .find(|account| account.id == calendar.account_id)
                .cloned()
                .ok_or(CalendarError::NotFound)?;
            (calendar, account)
        };
        let url = Url::parse(&calendar.url).map_err(|_| CalendarError::InvalidInput)?;
        if account.kind == CalendarAccountKindV1::Subscription {
            let text = self.fetch_feed(&url).await?;
            let events = ical::parse_calendar(&text)?;
            let mut inner = self.lock();
            let cache = inner.caches.entry(calendar.id.clone()).or_default();
            cache.resources.clear();
            cache.resources.insert(
                url.to_string(),
                CachedResource {
                    etag: None,
                    events,
                    raw: Some(text),
                },
            );
            cache.synced_at = Some(self.now());
            self.save_all(&calendar.id, cache);
            return Ok(());
        }
        let credentials = self.credentials_for(&account)?;
        let client = CaldavClient::new(self.http.as_ref(), credentials);
        let state = client.collection_state(&url).await.unwrap_or_default();
        let (known_ctag, known_token, known_etags) = {
            let inner = self.lock();
            inner
                .caches
                .get(&calendar.id)
                .filter(|cache| cache.synced_at.is_some())
                .map_or((None, None, HashMap::new()), |cache| {
                    (
                        cache.ctag.clone(),
                        cache.sync_token.clone(),
                        cache
                            .resources
                            .iter()
                            .map(|(href, resource)| (href.clone(), resource.etag.clone()))
                            .collect::<HashMap<_, _>>(),
                    )
                })
        };
        if !force && state.ctag.is_some() && state.ctag == known_ctag {
            let mut inner = self.lock();
            if let Some(cache) = inner.caches.get_mut(&calendar.id) {
                cache.synced_at = Some(self.now());
                self.save_meta(&calendar.id, cache);
            }
            return Ok(());
        }
        let incremental = match (&known_token, &state.sync_token) {
            (Some(token), Some(_)) => match client.sync_changes(&url, token).await {
                Ok(changes) => Some(changes),
                Err(CalendarError::Conflict | CalendarError::Unsupported) => None,
                Err(error) => return Err(error),
            },
            _ => None,
        };
        if let Some(changes) = incremental {
            let mut inner = self.lock();
            let cache = inner.caches.entry(calendar.id.clone()).or_default();
            let removed: Vec<String> = changes.removed.iter().map(Url::to_string).collect();
            for href in &removed {
                cache.resources.remove(href.as_str());
            }
            let mut changed = Vec::new();
            for resource in changes.changed {
                let (href, cached) = cached_from(resource);
                changed.push(href.clone());
                cache.resources.insert(href, cached);
            }
            cache.sync_token = changes.sync_token.or(state.sync_token);
            cache.ctag = state.ctag;
            cache.synced_at = Some(self.now());
            self.save_delta(&calendar.id, cache, &changed, &removed);
            return Ok(());
        }
        let now = self.now();
        let (from, to) = (
            now - Duration::days(SYNC_PAST_DAYS),
            now + Duration::days(SYNC_FUTURE_DAYS),
        );
        if known_etags.is_empty() {
            let resources = client.query_range(&url, from, to).await?;
            if resources.len() > caldav::MAX_RESOURCES {
                return Err(CalendarError::Limit);
            }
            let mut inner = self.lock();
            let cache = inner.caches.entry(calendar.id.clone()).or_default();
            cache.resources = resources.into_iter().map(cached_from).collect();
            cache.sync_token = state.sync_token;
            cache.ctag = state.ctag;
            cache.synced_at = Some(now);
            self.save_all(&calendar.id, cache);
            return Ok(());
        }
        // No usable sync token: compare ETags and fetch only what changed.
        let delta = client
            .query_range_delta(&url, from, to, &known_etags)
            .await?;
        if delta.present.len() > caldav::MAX_RESOURCES {
            return Err(CalendarError::Limit);
        }
        let present: HashSet<String> = delta.present.iter().map(Url::to_string).collect();
        let mut inner = self.lock();
        let cache = inner.caches.entry(calendar.id.clone()).or_default();
        let removed: Vec<String> = cache
            .resources
            .keys()
            .filter(|href| !present.contains(*href))
            .cloned()
            .collect();
        for href in &removed {
            cache.resources.remove(href);
        }
        let mut changed = Vec::new();
        for resource in delta.changed {
            let (href, cached) = cached_from(resource);
            changed.push(href.clone());
            cache.resources.insert(href, cached);
        }
        cache.sync_token = state.sync_token;
        cache.ctag = state.ctag;
        cache.synced_at = Some(now);
        self.save_delta(&calendar.id, cache, &changed, &removed);
        Ok(())
    }

    // -- changing events ------------------------------------------------------

    fn writable(&self, calendar_id: &str) -> Result<(CalendarV1, CalendarAccountV1)> {
        let inner = self.lock();
        let calendar = inner
            .state
            .calendars
            .iter()
            .find(|calendar| calendar.id == calendar_id)
            .cloned()
            .ok_or(CalendarError::NotFound)?;
        let account = inner
            .state
            .accounts
            .iter()
            .find(|account| account.id == calendar.account_id)
            .cloned()
            .ok_or(CalendarError::NotFound)?;
        if calendar.read_only || account.kind != CalendarAccountKindV1::Caldav {
            return Err(CalendarError::ReadOnly);
        }
        Ok((calendar, account))
    }

    fn locate(&self, calendar_id: &str, uid: &str) -> Option<(String, CachedResource)> {
        let inner = self.lock();
        inner
            .caches
            .get(calendar_id)?
            .resources
            .iter()
            .find_map(|(href, resource)| {
                resource
                    .events
                    .iter()
                    .any(|event| event.uid == uid && event.recurrence_id.is_none())
                    .then(|| (href.clone(), resource.clone()))
            })
    }

    fn store_resource(
        &self,
        calendar_id: &str,
        href: &Url,
        etag: Option<String>,
        events: Vec<Event>,
        raw: String,
    ) {
        let mut inner = self.lock();
        let cache = inner.caches.entry(calendar_id.to_owned()).or_default();
        let href = href.to_string();
        cache.resources.insert(
            href.clone(),
            CachedResource {
                etag,
                events,
                raw: Some(raw),
            },
        );
        // The server's collection state moved; the next refresh must look.
        cache.ctag = None;
        // A calendar never refreshed has no refresh state to keep.
        if cache.synced_at.is_some() {
            self.save_delta(calendar_id, cache, &[href], &[]);
        }
    }

    /// Creates an event in a writable calendar.
    pub async fn create_event(
        &self,
        calendar_id: &str,
        draft: &CalendarEventDraftV1,
    ) -> Result<CalendarEventRefV1> {
        let (calendar, account) = self.writable(calendar_id)?;
        let uid = format!("{}@zephium", new_id().to_ascii_lowercase());
        let event = apply_draft(&uid, draft, None)?;
        let ics = ical::serialize_event(&event, self.now());
        let url = Url::parse(&calendar.url).map_err(|_| CalendarError::InvalidInput)?;
        let client = CaldavClient::new(self.http.as_ref(), self.credentials_for(&account)?);
        let (href, etag) = client.put_event(&url, &uid, &ics, None).await?;
        self.store_resource(calendar_id, &href, etag, vec![event], ics);
        Ok(CalendarEventRefV1 {
            calendar_id: calendar_id.to_owned(),
            uid,
        })
    }

    /// The master of `uid` with the resource it lives in.
    fn master_of(&self, calendar_id: &str, uid: &str) -> Result<(String, CachedResource, Event)> {
        let (href, resource) = self
            .locate(calendar_id, uid)
            .ok_or(CalendarError::NotFound)?;
        let master = resource
            .events
            .iter()
            .find(|event| event.uid == uid && event.recurrence_id.is_none())
            .cloned()
            .ok_or(CalendarError::NotFound)?;
        Ok((href, resource, master))
    }

    /// Writes `updated` over the resource it came from, guarded by its ETag.
    async fn put_master(
        &self,
        calendar_id: &str,
        uid: &str,
        href: &str,
        resource: &CachedResource,
        updated: Event,
    ) -> Result<CalendarEventRefV1> {
        let (calendar, account) = self.writable(calendar_id)?;
        let ics = ical::serialize_event(&updated, self.now());
        let url = Url::parse(&calendar.url).map_err(|_| CalendarError::InvalidInput)?;
        let href_url = Url::parse(href).map_err(|_| CalendarError::InvalidInput)?;
        let client = CaldavClient::new(self.http.as_ref(), self.credentials_for(&account)?);
        let (href_url, etag) = client
            .put_event(&url, uid, &ics, Some((&href_url, resource.etag.as_deref())))
            .await?;
        self.store_resource(calendar_id, &href_url, etag, vec![updated], ics);
        Ok(CalendarEventRefV1 {
            calendar_id: calendar_id.to_owned(),
            uid: uid.to_owned(),
        })
    }

    /// Replaces the editable fields of an event, keeping attendees, other
    /// properties and, while the series start stays, its exceptions (EXDATEs
    /// and edited occurrences). A concurrent change on the server is
    /// [`CalendarError::Conflict`].
    pub async fn update_event(
        &self,
        calendar_id: &str,
        uid: &str,
        draft: &CalendarEventDraftV1,
    ) -> Result<CalendarEventRefV1> {
        self.writable(calendar_id)?;
        let (href, resource, master) = self.master_of(calendar_id, uid)?;
        let updated = apply_draft(uid, draft, Some(&master))?;
        self.put_master(calendar_id, uid, &href, &resource, updated)
            .await
    }

    /// The instance of `master` named by a `recurrence_id` as occurrences
    /// report it. Refuses a non-repeating event and a time that is no
    /// instance of the series.
    fn instance_of(master: &Event, recurrence_id: &str) -> Result<EventTime> {
        let zone = match &master.start.kind {
            TimeKind::Zoned(zone) => Some(zone.name()),
            _ => None,
        };
        let instance = parse_draft_time(recurrence_id, master.start.is_date(), zone)?;
        let known_override = master.overrides.iter().any(|item| {
            item.recurrence_id
                .as_ref()
                .is_some_and(|id| recur::same_time(id, &instance))
        });
        if !known_override && !recur::is_instance(master, &instance) {
            return Err(CalendarError::InvalidInput);
        }
        Ok(instance)
    }

    /// Changes one occurrence of a recurring event by writing a
    /// RECURRENCE-ID override; the series and its other exceptions stay.
    /// `recurrence_id` is the occurrence's `recurrence_id` as listed. The
    /// draft's `rrule` is ignored.
    pub async fn update_occurrence(
        &self,
        calendar_id: &str,
        uid: &str,
        recurrence_id: &str,
        draft: &CalendarEventDraftV1,
    ) -> Result<CalendarEventRefV1> {
        self.writable(calendar_id)?;
        let (href, resource, mut master) = self.master_of(calendar_id, uid)?;
        let instance = Self::instance_of(&master, recurrence_id)?;
        let same = |item: &Event| {
            item.recurrence_id
                .as_ref()
                .is_some_and(|id| recur::same_time(id, &instance))
        };
        let base = master
            .overrides
            .iter()
            .find(|item| same(item))
            .cloned()
            .unwrap_or_else(|| {
                let mut base = master.clone();
                base.overrides.clear();
                base.rrule = None;
                base.rdates.clear();
                base.exdates.clear();
                base.end = recur::instance_end(&master, &instance);
                base.start = instance.clone();
                base
            });
        let mut draft = draft.clone();
        draft.rrule = None;
        let mut item = apply_draft(uid, &draft, Some(&base))?;
        item.recurrence_id = Some(instance.clone());
        item.rrule = None;
        item.rdates.clear();
        item.exdates.clear();
        item.overrides.clear();
        master.overrides.retain(|existing| !same(existing));
        if master.overrides.len() >= ical::MAX_LIST {
            return Err(CalendarError::Limit);
        }
        master.overrides.push(item);
        // Editing an occurrence that was deleted brings it back.
        master
            .exdates
            .retain(|date| !recur::same_time(date, &instance));
        master.sequence = master.sequence.saturating_add(1);
        self.put_master(calendar_id, uid, &href, &resource, master)
            .await
    }

    /// Deletes one occurrence of a recurring event: an EXDATE on the series,
    /// and any override of that occurrence is dropped.
    pub async fn delete_occurrence(
        &self,
        calendar_id: &str,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<()> {
        self.writable(calendar_id)?;
        let (href, resource, mut master) = self.master_of(calendar_id, uid)?;
        let instance = Self::instance_of(&master, recurrence_id)?;
        master.overrides.retain(|item| {
            !item
                .recurrence_id
                .as_ref()
                .is_some_and(|id| recur::same_time(id, &instance))
        });
        if !master
            .exdates
            .iter()
            .any(|date| recur::same_time(date, &instance))
        {
            if master.exdates.len() >= ical::MAX_LIST {
                return Err(CalendarError::Limit);
            }
            master.exdates.push(instance);
        }
        master.sequence = master.sequence.saturating_add(1);
        self.put_master(calendar_id, uid, &href, &resource, master)
            .await
            .map(drop)
    }

    /// Deletes an event (the whole series for recurring events).
    pub async fn delete_event(&self, calendar_id: &str, uid: &str) -> Result<()> {
        let (_, account) = self.writable(calendar_id)?;
        let (href, resource) = self
            .locate(calendar_id, uid)
            .ok_or(CalendarError::NotFound)?;
        let href_url = Url::parse(&href).map_err(|_| CalendarError::InvalidInput)?;
        let client = CaldavClient::new(self.http.as_ref(), self.credentials_for(&account)?);
        client
            .delete_event(&href_url, resource.etag.as_deref())
            .await?;
        let mut inner = self.lock();
        if let Some(cache) = inner.caches.get_mut(calendar_id) {
            cache.resources.remove(&href);
            cache.ctag = None;
            if cache.synced_at.is_some() {
                self.save_delta(calendar_id, cache, &[], &[href]);
            }
        }
        Ok(())
    }
}

/// The wall clock.
pub fn system_clock() -> Clock {
    Arc::new(Utc::now)
}
