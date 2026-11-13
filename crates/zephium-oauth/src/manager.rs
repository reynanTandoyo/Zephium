//! Access tokens on demand. The manager keeps access tokens in memory only,
//! refreshes them shortly before they expire from the refresh token in the
//! OS vault, and stores a rotated refresh token back.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::error::{OAuthError, Result};
use crate::flow::{refresh, Grant};
use crate::http::HttpClient;
use crate::provider::{ClientConfig, Purpose};
use crate::vault::{id_ok, RefreshVault, StoredSecret};
use crate::BoxFuture;

/// An access token this close to expiry is refreshed first.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(60);

/// Hands out valid access tokens. Implemented by [`TokenManager`]; the mail
/// engine takes it as a port.
pub trait AccessTokenSource: Send + Sync {
    /// A bearer token for `purpose`, valid for at least [`REFRESH_MARGIN`].
    fn access_token<'a>(
        &'a self,
        account_id: &'a str,
        config: &'a ClientConfig,
        purpose: Purpose,
    ) -> BoxFuture<'a, Result<Zeroizing<String>>>;
}

struct Cached {
    token: Zeroizing<String>,
    expires_at: Instant,
}

type Slot = Arc<AsyncMutex<Option<Cached>>>;

/// The production [`AccessTokenSource`].
pub struct TokenManager {
    http: Arc<dyn HttpClient>,
    vault: Arc<dyn RefreshVault>,
    slots: Mutex<HashMap<(String, Purpose), Slot>>,
}

impl fmt::Debug for TokenManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenManager")
            .finish_non_exhaustive()
    }
}

impl TokenManager {
    /// Builds a manager over its two ports.
    #[must_use]
    pub fn new(http: Arc<dyn HttpClient>, vault: Arc<dyn RefreshVault>) -> Self {
        Self {
            http,
            vault,
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn slot(&self, account_id: &str, purpose: Purpose) -> Slot {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((account_id.to_owned(), purpose))
            .or_default()
            .clone()
    }

    /// Stores a fresh sign-in for an account: the refresh token (and client
    /// secret) go to the vault, the access token to memory.
    pub async fn adopt(&self, account_id: &str, grant: &Grant) -> Result<()> {
        if !id_ok(account_id) {
            return Err(OAuthError::InvalidInput);
        }
        self.vault.set(
            account_id,
            &StoredSecret {
                refresh_token: (*grant.refresh_token).clone(),
                client_secret: grant.client_secret.as_ref().map(|s| (**s).clone()),
            },
        )?;
        // The first token serves the first purpose asked for.
        if let Some(purpose) = grant.purposes.first() {
            let slot = self.slot(account_id, *purpose);
            *slot.lock().await = Some(Cached {
                token: grant.access_token.clone(),
                expires_at: Instant::now() + grant.lifetime,
            });
        }
        Ok(())
    }

    /// Drops every cached access token of an account.
    pub async fn forget(&self, account_id: &str) {
        let slots: Vec<Slot> = {
            let mut map = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            let keys: Vec<_> = map
                .keys()
                .filter(|(id, _)| id == account_id)
                .cloned()
                .collect();
            keys.iter().filter_map(|key| map.remove(key)).collect()
        };
        for slot in slots {
            slot.lock().await.take();
        }
    }

    /// Signs an account out: forgets its tokens and removes its refresh
    /// token from the vault. The server-side grant stays until the person
    /// revokes it in their Microsoft or Google account.
    pub async fn sign_out(&self, account_id: &str) -> Result<()> {
        self.forget(account_id).await;
        self.vault.remove(account_id)
    }

    async fn fetch(
        &self,
        account_id: &str,
        config: &ClientConfig,
        purpose: Purpose,
    ) -> Result<Cached> {
        let stored = self.vault.get(account_id)?;
        let result = refresh(
            &*self.http,
            config,
            stored.client_secret.as_deref(),
            &stored.refresh_token,
            purpose,
        )
        .await?;
        if let Some(rotated) = &result.refresh_token {
            if **rotated != stored.refresh_token {
                self.vault.set(
                    account_id,
                    &StoredSecret {
                        refresh_token: (**rotated).clone(),
                        client_secret: stored.client_secret.clone(),
                    },
                )?;
            }
        }
        Ok(Cached {
            token: result.access_token.clone(),
            expires_at: Instant::now() + result.lifetime,
        })
    }

    async fn token(
        &self,
        account_id: &str,
        config: &ClientConfig,
        purpose: Purpose,
    ) -> Result<Zeroizing<String>> {
        if !id_ok(account_id) {
            return Err(OAuthError::InvalidInput);
        }
        config.validate()?;
        let slot = self.slot(account_id, purpose);
        let mut guard = slot.lock().await;
        if let Some(cached) = guard.as_ref() {
            if cached.expires_at > Instant::now() + REFRESH_MARGIN {
                return Ok(cached.token.clone());
            }
        }
        *guard = None;
        let fresh = self.fetch(account_id, config, purpose).await?;
        let token = fresh.token.clone();
        *guard = Some(fresh);
        Ok(token)
    }
}

impl AccessTokenSource for TokenManager {
    fn access_token<'a>(
        &'a self,
        account_id: &'a str,
        config: &'a ClientConfig,
        purpose: Purpose,
    ) -> BoxFuture<'a, Result<Zeroizing<String>>> {
        Box::pin(self.token(account_id, config, purpose))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::fakes::FakeHttp;
    use crate::provider::Provider;
    use crate::vault::MemoryRefreshVault;

    fn config() -> ClientConfig {
        ClientConfig {
            provider: Provider::Microsoft,
            client_id: "client-1".into(),
            tenant: None,
        }
    }

    fn rig() -> (Arc<FakeHttp>, Arc<MemoryRefreshVault>, TokenManager) {
        let http = Arc::new(FakeHttp::default());
        let vault = Arc::new(MemoryRefreshVault::default());
        let manager = TokenManager::new(http.clone(), vault.clone());
        (http, vault, manager)
    }

    fn grant(lifetime: u64) -> Grant {
        Grant {
            config: config(),
            client_secret: None,
            access_token: Zeroizing::new("AT0".into()),
            refresh_token: Zeroizing::new("RT0".into()),
            lifetime: Duration::from_secs(lifetime),
            purposes: vec![Purpose::Mail],
        }
    }

    #[tokio::test]
    async fn adopted_tokens_are_served_from_memory_and_the_vault_holds_the_refresh_token() {
        let (http, vault, manager) = rig();
        manager.adopt("a1", &grant(3600)).await.unwrap();
        assert_eq!(vault.get("a1").unwrap().refresh_token, "RT0");
        let token = manager.access_token("a1", &config(), Purpose::Mail).await;
        assert_eq!(&*token.unwrap(), "AT0");
        assert!(http.posts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_expiring_token_is_refreshed_once_and_cached() {
        let (http, vault, manager) = rig();
        // Inside the margin: counts as expired.
        manager.adopt("a1", &grant(30)).await.unwrap();
        http.answer(
            200,
            r#"{"access_token":"AT1","token_type":"Bearer","expires_in":3600}"#,
        );
        let first = manager
            .access_token("a1", &config(), Purpose::Mail)
            .await
            .unwrap();
        let second = manager
            .access_token("a1", &config(), Purpose::Mail)
            .await
            .unwrap();
        assert_eq!((first.as_str(), second.as_str()), ("AT1", "AT1"));
        assert_eq!(http.posts.lock().unwrap().len(), 1);
        // No rotation: the stored refresh token is untouched.
        assert_eq!(vault.get("a1").unwrap().refresh_token, "RT0");
    }

    #[tokio::test]
    async fn a_rotated_refresh_token_replaces_the_stored_one() {
        let (http, vault, manager) = rig();
        manager.adopt("a1", &grant(0)).await.unwrap();
        http.answer(
            200,
            r#"{"access_token":"AT1","refresh_token":"RT1","token_type":"Bearer","expires_in":3600}"#,
        );
        manager
            .access_token("a1", &config(), Purpose::Mail)
            .await
            .unwrap();
        assert_eq!(vault.get("a1").unwrap().refresh_token, "RT1");
    }

    #[tokio::test]
    async fn purposes_have_separate_tokens() {
        let (http, _vault, manager) = rig();
        manager.adopt("a1", &grant(3600)).await.unwrap();
        http.answer(
            200,
            r#"{"access_token":"GRAPH","token_type":"Bearer","expires_in":3600}"#,
        );
        let calendar = manager
            .access_token("a1", &config(), Purpose::Calendar)
            .await
            .unwrap();
        assert_eq!(&*calendar, "GRAPH");
        let mail = manager
            .access_token("a1", &config(), Purpose::Mail)
            .await
            .unwrap();
        assert_eq!(&*mail, "AT0");
    }

    #[tokio::test]
    async fn a_revoked_refresh_token_asks_for_sign_in() {
        let (http, _vault, manager) = rig();
        manager.adopt("a1", &grant(0)).await.unwrap();
        http.answer(400, r#"{"error":"invalid_grant","error_description":"x"}"#);
        let result = manager.access_token("a1", &config(), Purpose::Mail).await;
        assert_eq!(result.unwrap_err(), OAuthError::SignInRequired);
    }

    #[tokio::test]
    async fn without_a_stored_token_there_is_no_access_token() {
        let (_http, _vault, manager) = rig();
        let result = manager.access_token("a1", &config(), Purpose::Mail).await;
        assert_eq!(result.unwrap_err(), OAuthError::Credentials);
        let result = manager.access_token("a|1", &config(), Purpose::Mail).await;
        assert_eq!(result.unwrap_err(), OAuthError::InvalidInput);
    }

    #[tokio::test]
    async fn sign_out_removes_the_vault_entry_and_the_cache() {
        let (_http, vault, manager) = rig();
        manager.adopt("a1", &grant(3600)).await.unwrap();
        manager.sign_out("a1").await.unwrap();
        assert!(vault.get("a1").is_err());
        let result = manager.access_token("a1", &config(), Purpose::Mail).await;
        assert_eq!(result.unwrap_err(), OAuthError::Credentials);
        manager.sign_out("a1").await.unwrap();
    }
}
