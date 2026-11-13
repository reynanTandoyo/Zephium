//! Google and Microsoft accounts in the calendar service: sign-in, listing,
//! sync and event changes. Provider specifics live in
//! [`crate::provider`]; this module applies what they return to the cache.

use super::*;
use crate::api::CalendarOAuthV1;
use crate::provider::{self, google, graph, Api, Item};
use zephium_oauth::flow::Grant;
use zephium_oauth::provider::{ClientConfig, Provider, Purpose};

/// Applies provider items to a calendar's resources. A resource is a series
/// master (with the overrides known so far) or a single event, keyed by the
/// provider's id; the cached text is regenerated from the model. Returns the
/// addresses written and the addresses removed.
fn apply_items(
    resources: &mut HashMap<String, CachedResource>,
    items: Vec<Item>,
    now: DateTime<Utc>,
) -> Result<(Vec<String>, Vec<String>)> {
    let mut changed: HashSet<String> = HashSet::new();
    let mut removed: HashSet<String> = HashSet::new();
    // Masters and removals first: an occurrence may arrive before its series.
    for item in &items {
        match item {
            Item::Event { href, etag, event } => {
                let mut event = event.clone();
                if event.is_recurring() {
                    if let Some(old) = resources
                        .get(href)
                        .and_then(|resource| resource.events.first())
                        .filter(|old| old.start == event.start)
                    {
                        // Exceptions arrive as their own items and are not
                        // repeated when only the series changes.
                        event.overrides = old.overrides.clone();
                        for date in &old.exdates {
                            if event.exdates.len() < ical::MAX_LIST
                                && !event
                                    .exdates
                                    .iter()
                                    .any(|known| recur::same_time(known, date))
                            {
                                event.exdates.push(date.clone());
                            }
                        }
                    }
                }
                let raw = ical::serialize_event(&event, now);
                resources.insert(
                    href.clone(),
                    CachedResource {
                        etag: etag.clone(),
                        events: vec![event],
                        raw: Some(raw),
                    },
                );
                removed.remove(href);
                changed.insert(href.clone());
            }
            Item::Gone { href } => {
                resources.remove(href);
                changed.remove(href);
                removed.insert(href.clone());
            }
            Item::Occurrence { .. } => {}
        }
    }
    for item in items {
        let Item::Occurrence {
            series,
            original,
            event,
        } = item
        else {
            continue;
        };
        let Some(resource) = resources.get_mut(&series) else {
            continue;
        };
        let Some(master) = resource.events.first_mut() else {
            continue;
        };
        master.overrides.retain(|existing| {
            !existing
                .recurrence_id
                .as_ref()
                .is_some_and(|id| recur::same_time(id, &original))
        });
        match event {
            Some(event) => {
                if master.overrides.len() < ical::MAX_LIST {
                    master.overrides.push(event);
                }
                master
                    .exdates
                    .retain(|date| !recur::same_time(date, &original));
            }
            None => {
                if master.exdates.len() < ical::MAX_LIST
                    && !master
                        .exdates
                        .iter()
                        .any(|date| recur::same_time(date, &original))
                {
                    master.exdates.push(original);
                }
            }
        }
        resource.raw = Some(ical::serialize_event(master, now));
        changed.insert(series);
    }
    if resources.len() > ical::MAX_EVENTS {
        return Err(CalendarError::Limit);
    }
    Ok((changed.into_iter().collect(), removed.into_iter().collect()))
}

/// The tokens and OAuth client one account's requests need.
struct Session {
    tokens: Arc<dyn TokenSessions>,
    config: ClientConfig,
}

impl CalendarService {
    /// Gives the service the sign-in port that Google and Microsoft accounts
    /// need. Without it those accounts answer
    /// [`CalendarError::Unsupported`].
    #[must_use]
    pub fn with_oauth(mut self, tokens: Arc<dyn TokenSessions>) -> Self {
        self.tokens = Some(tokens);
        self
    }

    fn session(&self, account: &CalendarAccountV1) -> Result<Session> {
        Ok(Session {
            tokens: self.tokens.clone().ok_or(CalendarError::Unsupported)?,
            config: provider::client_config(account)?,
        })
    }

    async fn list_remote(
        &self,
        kind: CalendarAccountKindV1,
        api: &Api<'_>,
    ) -> Result<Vec<provider::RemoteCalendar>> {
        match kind {
            CalendarAccountKindV1::Google => google::calendars(api).await,
            CalendarAccountKindV1::Microsoft => graph::calendars(api).await,
            _ => Err(CalendarError::InvalidInput),
        }
    }

    /// Adds the Google or Microsoft account a completed sign-in belongs to.
    /// The grant must cover [`Purpose::Calendar`]; its refresh token goes to
    /// the vault under the new account's id. The calendars are listed from
    /// the provider; events load on the next sync. A failure signs the
    /// account out again, so nothing is left behind.
    pub async fn add_oauth_account(
        &self,
        grant: &Grant,
        name: Option<&str>,
    ) -> Result<CalendarAccountV1> {
        let tokens = self.tokens.clone().ok_or(CalendarError::Unsupported)?;
        grant.config.validate().map_err(provider::map_oauth)?;
        if !grant.purposes.contains(&Purpose::Calendar) {
            return Err(CalendarError::InvalidInput);
        }
        let (kind, root, default_name) = match grant.config.provider {
            Provider::Google => (
                CalendarAccountKindV1::Google,
                google::API_ROOT,
                "Google Calendar",
            ),
            Provider::Microsoft => (
                CalendarAccountKindV1::Microsoft,
                graph::API_ROOT,
                "Microsoft 365",
            ),
        };
        if self.lock().state.accounts.len() >= MAX_ACCOUNTS {
            return Err(CalendarError::InvalidInput);
        }
        let id = new_id();
        tokens
            .adopt(&id, grant)
            .await
            .map_err(provider::map_oauth)?;
        let outcome = self
            .finish_oauth_account(&id, kind, root, default_name, grant, name)
            .await;
        if outcome.is_err() {
            let _ = tokens.sign_out(&id).await;
        }
        outcome
    }

    async fn finish_oauth_account(
        &self,
        id: &str,
        kind: CalendarAccountKindV1,
        root: &str,
        default_name: &str,
        grant: &Grant,
        name: Option<&str>,
    ) -> Result<CalendarAccountV1> {
        let tokens = self.tokens.clone().ok_or(CalendarError::Unsupported)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &tokens,
            account_id: id,
            config: &grant.config,
        };
        let remote = self.list_remote(kind, &api).await?;
        if remote.is_empty() {
            return Err(CalendarError::NotFound);
        }
        let owner = remote.iter().find_map(|calendar| calendar.owner.clone());
        let account = CalendarAccountV1 {
            id: id.to_owned(),
            kind,
            name: name
                .map(|value| value.trim().chars().take(100).collect::<String>())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| default_name.to_owned()),
            server_url: root.to_owned(),
            username: owner.clone(),
            oauth: Some(CalendarOAuthV1 {
                client_id: grant.config.client_id.clone(),
                tenant: grant.config.tenant.clone(),
            }),
        };
        let mut inner = self.lock();
        let duplicate = inner.state.accounts.iter().any(|existing| {
            existing.kind == kind
                && existing.oauth.as_ref().map(|o| &o.client_id) == Some(&grant.config.client_id)
                && owner.is_some()
                && existing.username == owner
        });
        if duplicate
            || inner.state.accounts.len() >= MAX_ACCOUNTS
            || inner.state.calendars.len() + remote.len()
                > zephium_store::calendar_cache::MAX_CALENDARS
        {
            return Err(CalendarError::InvalidInput);
        }
        let mut next = inner.state.clone();
        next.accounts.push(account.clone());
        for (index, calendar) in remote.iter().enumerate() {
            next.calendars.push(CalendarV1 {
                id: new_id(),
                account_id: id.to_owned(),
                url: calendar.url.clone(),
                name: calendar.name.clone(),
                color: calendar
                    .color
                    .clone()
                    .or_else(|| Some(PALETTE[index % PALETTE.len()].to_owned())),
                read_only: calendar.read_only,
                visible: true,
            });
        }
        self.persist(&next)?;
        inner.state = next;
        Ok(account)
    }

    /// Removes an account like [`Self::remove_account`] and, for a Google or
    /// Microsoft account, signs it out: its refresh token leaves the vault.
    /// The grant itself stays valid at the provider until the person revokes
    /// it there.
    pub async fn remove_account_and_session(&self, account_id: &str) -> Result<()> {
        let oauth = self
            .lock()
            .state
            .accounts
            .iter()
            .any(|account| account.id == account_id && account.kind.is_oauth());
        self.remove_account(account_id)?;
        if oauth {
            if let Some(tokens) = &self.tokens {
                let _ = tokens.sign_out(account_id).await;
            }
        }
        Ok(())
    }

    // -- sync -----------------------------------------------------------------

    pub(super) async fn sync_oauth(
        &self,
        calendar: &CalendarV1,
        account: &CalendarAccountV1,
    ) -> Result<()> {
        let session = self.session(account)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &session.tokens,
            account_id: &account.id,
            config: &session.config,
        };
        let known = {
            let inner = self.lock();
            inner
                .caches
                .get(&calendar.id)
                .filter(|cache| cache.synced_at.is_some())
                .and_then(|cache| cache.sync_token.clone())
        };
        let now = self.now();
        let delta = match account.kind {
            CalendarAccountKindV1::Google => {
                google::sync(&api, &calendar.url, known.as_deref()).await?
            }
            _ => graph::sync(&api, &calendar.url, known.as_deref(), now).await?,
        };
        let mut inner = self.lock();
        let cache = inner.caches.entry(calendar.id.clone()).or_default();
        if delta.full {
            let mut fresh = HashMap::new();
            apply_items(&mut fresh, delta.items, now)?;
            cache.resources = fresh;
            cache.sync_token = delta.next_token;
            cache.synced_at = Some(now);
            self.save_all(&calendar.id, cache);
        } else {
            let (changed, removed) = apply_items(&mut cache.resources, delta.items, now)?;
            cache.sync_token = delta.next_token.or(known);
            cache.synced_at = Some(now);
            self.save_delta(&calendar.id, cache, &changed, &removed);
        }
        Ok(())
    }

    /// Applies what a write returned to the cache, so the change shows at
    /// once; the sync token stays, and the next sync repeats the same change.
    fn apply_written(&self, calendar_id: &str, items: Vec<Item>) -> Result<()> {
        let now = self.now();
        let mut inner = self.lock();
        let cache = inner.caches.entry(calendar_id.to_owned()).or_default();
        let (changed, removed) = apply_items(&mut cache.resources, items, now)?;
        if cache.synced_at.is_some() {
            self.save_delta(calendar_id, cache, &changed, &removed);
        }
        Ok(())
    }

    /// Reads a calendar again after a write whose result cannot be applied
    /// directly (a new Graph series). A failure only delays the change until
    /// the next periodic sync.
    async fn refresh_after_write(&self, calendar_id: &str) {
        let _gate = self.sync_gate.lock().await;
        let _ = self.sync_one(calendar_id, true).await;
    }

    // -- changing events ------------------------------------------------------

    /// Creates an event. A Graph series is read back with the next sync.
    pub(super) async fn oauth_create(
        &self,
        calendar: &CalendarV1,
        account: &CalendarAccountV1,
        draft: &CalendarEventDraftV1,
    ) -> Result<CalendarEventRefV1> {
        let event = apply_draft("new", draft, None)?;
        let session = self.session(account)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &session.tokens,
            account_id: &account.id,
            config: &session.config,
        };
        let (items, series) = match account.kind {
            CalendarAccountKindV1::Google => {
                (google::create(&api, &calendar.url, &event).await?, false)
            }
            _ => {
                let item = graph::create(&api, &calendar.url, &event).await?;
                let series = matches!(&item, Item::Event { event, .. } if event.is_recurring());
                (vec![item], series)
            }
        };
        let uid = items
            .iter()
            .find_map(|item| match item {
                Item::Event { href, .. } => Some(href.clone()),
                _ => None,
            })
            .ok_or(CalendarError::Parse)?;
        if series {
            self.refresh_after_write(&calendar.id).await;
        } else {
            self.apply_written(&calendar.id, items)?;
        }
        Ok(CalendarEventRefV1 {
            calendar_id: calendar.id.clone(),
            uid,
        })
    }

    /// Changes an event. On Google that is the series master (or a single
    /// event); on Microsoft it is the one cached event, which for a repeating
    /// event is a single occurrence.
    pub(super) async fn oauth_update(
        &self,
        calendar: &CalendarV1,
        account: &CalendarAccountV1,
        uid: &str,
        draft: &CalendarEventDraftV1,
    ) -> Result<CalendarEventRefV1> {
        let (_, resource, base) = self.master_of(&calendar.id, uid)?;
        let updated = apply_draft(uid, draft, Some(&base))?;
        let session = self.session(account)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &session.tokens,
            account_id: &account.id,
            config: &session.config,
        };
        let item = match account.kind {
            CalendarAccountKindV1::Google => {
                google::update(&api, &calendar.url, &updated, resource.etag.as_deref()).await?
            }
            _ => vec![graph::update(&api, &updated).await?],
        };
        self.apply_written(&calendar.id, item)?;
        Ok(CalendarEventRefV1 {
            calendar_id: calendar.id.clone(),
            uid: uid.to_owned(),
        })
    }

    /// Changes one occurrence of a Google series. Microsoft repeats are
    /// cached as individual events, so they are changed through
    /// [`Self::update_event`].
    pub(super) async fn oauth_update_occurrence(
        &self,
        calendar: &CalendarV1,
        account: &CalendarAccountV1,
        uid: &str,
        recurrence_id: &str,
        draft: &CalendarEventDraftV1,
    ) -> Result<CalendarEventRefV1> {
        if account.kind != CalendarAccountKindV1::Google {
            return Err(CalendarError::Unsupported);
        }
        let (_, _, master) = self.master_of(&calendar.id, uid)?;
        let instance = Self::instance_of(&master, recurrence_id)?;
        let item = Self::occurrence_override(&master, &instance, draft)?;
        let session = self.session(account)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &session.tokens,
            account_id: &account.id,
            config: &session.config,
        };
        let items = google::update_occurrence(
            &api,
            &calendar.url,
            uid,
            master.start.is_date(),
            &instance,
            &item,
        )
        .await?;
        self.apply_written(&calendar.id, items)?;
        Ok(CalendarEventRefV1 {
            calendar_id: calendar.id.clone(),
            uid: uid.to_owned(),
        })
    }

    /// Deletes one occurrence of a Google series.
    pub(super) async fn oauth_delete_occurrence(
        &self,
        calendar: &CalendarV1,
        account: &CalendarAccountV1,
        uid: &str,
        recurrence_id: &str,
    ) -> Result<()> {
        if account.kind != CalendarAccountKindV1::Google {
            return Err(CalendarError::Unsupported);
        }
        let (_, _, master) = self.master_of(&calendar.id, uid)?;
        let instance = Self::instance_of(&master, recurrence_id)?;
        let session = self.session(account)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &session.tokens,
            account_id: &account.id,
            config: &session.config,
        };
        google::delete_occurrence(&api, &calendar.url, uid, master.start.is_date(), &instance)
            .await?;
        self.apply_written(
            &calendar.id,
            vec![Item::Occurrence {
                series: uid.to_owned(),
                original: instance,
                event: None,
            }],
        )
    }

    /// Deletes an event (the whole series on Google; one occurrence on
    /// Microsoft).
    pub(super) async fn oauth_delete(
        &self,
        calendar: &CalendarV1,
        account: &CalendarAccountV1,
        uid: &str,
    ) -> Result<()> {
        let (href, resource) = self
            .locate(&calendar.id, uid)
            .ok_or(CalendarError::NotFound)?;
        let session = self.session(account)?;
        let api = Api {
            http: self.http.as_ref(),
            tokens: &session.tokens,
            account_id: &account.id,
            config: &session.config,
        };
        match account.kind {
            CalendarAccountKindV1::Google => {
                google::delete(&api, &calendar.url, uid, resource.etag.as_deref()).await?;
            }
            _ => graph::delete(&api, uid).await?,
        }
        self.apply_written(&calendar.id, vec![Item::Gone { href }])
    }
}
