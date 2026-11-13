//! Fork-owned (T-0034, prepares T-0040): OAuth sign-in commands.
//!
//! The frame sends a client ID the person typed (never committed) and gets
//! back an opaque, single-use grant handle. The tokens stay here: the
//! refresh token goes to the OS vault when the handle is consumed by
//! `mail_add_account` (or `mail_update_account`), the access token stays in
//! memory. Nothing secret is returned, logged or stored in the database.
use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use zephium_oauth::flow::{self, BrowserOpener, Grant, SignInRequest, DEFAULT_WAIT};
use zephium_oauth::http::{HttpClient, HttpResponse, MAX_BODY_BYTES};
use zephium_oauth::manager::TokenManager;
use zephium_oauth::pkce;
use zephium_oauth::provider::{ClientConfig, Provider, Purpose};
use zephium_oauth::vault::OsRefreshVault;
use zephium_oauth::{BoxFuture, OAuthError};
use zeroize::{Zeroize, Zeroizing};

/// Grants waiting to be attached to an account.
const MAX_GRANTS: usize = 4;
/// How long an unused grant stays valid.
const GRANT_TTL: Duration = Duration::from_secs(10 * 60);
const TOKEN_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Why an OAuth command failed. A closed set: no server or browser text.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OAuthErrorV1 {
    InvalidInput,
    Cancelled,
    Timeout,
    Offline,
    Denied,
    StateMismatch,
    Protocol,
    Rejected,
    SignInRequired,
    Credentials,
    Busy,
    Unsupported,
    Internal,
}

impl From<OAuthError> for OAuthErrorV1 {
    fn from(error: OAuthError) -> Self {
        match error {
            OAuthError::InvalidInput => Self::InvalidInput,
            OAuthError::Cancelled => Self::Cancelled,
            OAuthError::Timeout => Self::Timeout,
            OAuthError::Offline => Self::Offline,
            OAuthError::Denied => Self::Denied,
            OAuthError::StateMismatch => Self::StateMismatch,
            OAuthError::Protocol => Self::Protocol,
            OAuthError::Rejected => Self::Rejected,
            OAuthError::SignInRequired => Self::SignInRequired,
            OAuthError::Credentials => Self::Credentials,
            OAuthError::Busy => Self::Busy,
            OAuthError::Unsupported => Self::Unsupported,
            OAuthError::Internal => Self::Internal,
        }
    }
}

type Reply<T> = Result<T, OAuthErrorV1>;

/// The identity provider.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OAuthProviderV1 {
    Microsoft,
    Google,
}

impl From<OAuthProviderV1> for Provider {
    fn from(provider: OAuthProviderV1) -> Self {
        match provider {
            OAuthProviderV1::Microsoft => Self::Microsoft,
            OAuthProviderV1::Google => Self::Google,
        }
    }
}

impl From<Provider> for OAuthProviderV1 {
    fn from(provider: Provider) -> Self {
        match provider {
            Provider::Microsoft => Self::Microsoft,
            Provider::Google => Self::Google,
        }
    }
}

/// The result of a completed sign-in. The handle is opaque and single-use:
/// pass it as `oauth_grant` to `mail_add_account` (or `mail_update_account`)
/// within ten minutes. No token is in here.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct OAuthGrantV1 {
    pub(crate) handle: String,
    pub(crate) provider: OAuthProviderV1,
    pub(crate) client_id: String,
    pub(crate) tenant: Option<String>,
    /// Seconds until the handle expires.
    pub(crate) expires_in_secs: u32,
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Posts token requests with `reqwest`: HTTPS only, no redirects, bounded
/// time and size.
struct ReqwestHttp(Option<reqwest::Client>);

impl ReqwestHttp {
    fn new() -> Self {
        Self(
            reqwest::Client::builder()
                .timeout(TOKEN_REQUEST_TIMEOUT)
                .user_agent("Zephium")
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok(),
        )
    }
}

impl HttpClient for ReqwestHttp {
    fn post_form<'a>(
        &'a self,
        url: &'a str,
        form: Vec<(String, String)>,
    ) -> BoxFuture<'a, Result<HttpResponse, OAuthError>> {
        Box::pin(async move {
            let client = self.0.as_ref().ok_or(OAuthError::Internal)?;
            if !url.starts_with("https://") {
                return Err(OAuthError::InvalidInput);
            }
            let body = zephium_oauth::http::encode_form(&form);
            let mut response = client
                .post(url)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body.to_string())
                .send()
                .await
                .map_err(|error| {
                    if error.is_timeout() {
                        OAuthError::Timeout
                    } else {
                        OAuthError::Offline
                    }
                })?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| OAuthError::Offline)? {
                if body.len() + chunk.len() > MAX_BODY_BYTES {
                    return Err(OAuthError::Protocol);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse { status, body })
        })
    }
}

/// Opens the consent page in the system's default browser.
struct SystemBrowser;

impl BrowserOpener for SystemBrowser {
    fn open(&self, url: &str) -> bool {
        // Only the HTTPS consent page the oauth crate built is ever opened.
        if !url.starts_with("https://") || url.contains(['\0', '\r', '\n']) {
            return false;
        }
        open_in_system_browser(url)
    }
}

#[cfg(target_os = "windows")]
fn open_in_system_browser(url: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
    // SAFETY: a NUL-terminated https URL for the plain "open" verb;
    // ShellExecute reports success as any value above 32.
    let result = unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize > 32
}

#[cfg(target_os = "macos")]
fn open_in_system_browser(url: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    NSURL::URLWithString(&NSString::from_str(url))
        .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_in_system_browser(_url: &str) -> bool {
    false
}

/// The process-wide token manager: refresh tokens in the OS vault, access
/// tokens in memory. The mail engine uses it through `AccessTokenSource`.
pub(crate) fn token_manager() -> Arc<TokenManager> {
    static MANAGER: OnceLock<Arc<TokenManager>> = OnceLock::new();
    MANAGER
        .get_or_init(|| {
            Arc::new(TokenManager::new(
                Arc::new(ReqwestHttp::new()),
                Arc::new(OsRefreshVault),
            ))
        })
        .clone()
}

struct Pending {
    grant: Grant,
    created: Instant,
}

fn grants() -> &'static Mutex<HashMap<String, Pending>> {
    static GRANTS: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    GRANTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_grants() -> std::sync::MutexGuard<'static, HashMap<String, Pending>> {
    grants().lock().unwrap_or_else(|e| e.into_inner())
}

fn purge(map: &mut HashMap<String, Pending>) {
    map.retain(|_, pending| pending.created.elapsed() < GRANT_TTL);
}

fn keep_grant(grant: Grant) -> Result<String, OAuthError> {
    let handle = pkce::handle(18)?;
    let mut map = lock_grants();
    purge(&mut map);
    if map.len() >= MAX_GRANTS {
        let oldest = map
            .iter()
            .min_by_key(|(_, pending)| pending.created)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            map.remove(&oldest);
        }
    }
    map.insert(
        handle.clone(),
        Pending {
            grant,
            created: Instant::now(),
        },
    );
    Ok(handle)
}

/// Consumes a grant handle. A handle works once.
pub(crate) fn take_grant(handle: &str) -> Option<Grant> {
    let mut map = lock_grants();
    purge(&mut map);
    map.remove(handle).map(|pending| pending.grant)
}

/// The sign-in's configuration and a copy of its access token, without
/// consuming the handle (used to test an account before it is saved).
pub(crate) fn peek_grant(handle: &str) -> Option<(ClientConfig, Zeroizing<String>)> {
    let mut map = lock_grants();
    purge(&mut map);
    let pending = map.get(handle)?;
    (pending.created.elapsed() + Duration::from_secs(60) < pending.grant.lifetime).then(|| {
        (
            pending.grant.config.clone(),
            pending.grant.access_token.clone(),
        )
    })
}

fn waiting() -> &'static Mutex<Option<Arc<Notify>>> {
    static WAITING: OnceLock<Mutex<Option<Arc<Notify>>>> = OnceLock::new();
    WAITING.get_or_init(|| Mutex::new(None))
}

fn log(command: &str, ok: bool) {
    // Codes only: never a client ID, address, token or URL.
    write_diagnostic(format_args!("oauth: command={command} ok={ok}"));
}

fn gate(caller: &WebviewWindow, command: &'static str) -> Reply<()> {
    if !authorize(caller, CallerPolicy::Main, command) || shutdown_started(caller.app_handle()) {
        return Err(OAuthErrorV1::Internal);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Opens the system browser at the provider's consent page and waits (up to
/// three minutes) for the person to finish. One sign-in at a time. The
/// `client_id` is the one the person registered; `client_secret` is only for
/// Google "Desktop app" clients, which Google also issues a secret for.
#[tauri::command]
#[specta::specta]
pub(crate) async fn oauth_sign_in(
    caller: WebviewWindow,
    provider: OAuthProviderV1,
    client_id: String,
    tenant: Option<String>,
    client_secret: Option<String>,
) -> Reply<OAuthGrantV1> {
    gate(&caller, "oauth_sign_in")?;
    let mut client_secret = client_secret.filter(|secret| !secret.trim().is_empty());
    let outcome = sign_in(provider, client_id, tenant, client_secret.as_deref()).await;
    client_secret.zeroize();
    log("sign_in", outcome.is_ok());
    outcome.map_err(Into::into)
}

async fn sign_in(
    provider: OAuthProviderV1,
    client_id: String,
    tenant: Option<String>,
    client_secret: Option<&str>,
) -> Result<OAuthGrantV1, OAuthError> {
    let tenant = tenant
        .map(|tenant| tenant.trim().to_owned())
        .filter(|tenant| !tenant.is_empty());
    let config = ClientConfig {
        provider: provider.into(),
        client_id: client_id.trim().to_owned(),
        tenant: (Provider::from(provider) == Provider::Microsoft)
            .then_some(tenant)
            .flatten(),
    };
    let cancel = Arc::new(Notify::new());
    {
        let mut slot = waiting().lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(OAuthError::Busy);
        }
        *slot = Some(cancel.clone());
    }
    let http = ReqwestHttp::new();
    let result = flow::sign_in(
        &http,
        &SystemBrowser,
        SignInRequest {
            config: &config,
            client_secret,
            purposes: &[Purpose::Mail],
            wait: DEFAULT_WAIT,
        },
        &cancel,
    )
    .await;
    *waiting().lock().unwrap_or_else(|e| e.into_inner()) = None;
    let grant = result?;
    let expires = u32::try_from(GRANT_TTL.as_secs()).unwrap_or(u32::MAX);
    let handle = keep_grant(grant)?;
    Ok(OAuthGrantV1 {
        handle,
        provider,
        client_id: config.client_id,
        tenant: config.tenant,
        expires_in_secs: expires,
    })
}

/// Stops a sign-in that is waiting for the browser (the person gave up).
#[tauri::command]
#[specta::specta]
pub(crate) fn oauth_cancel_sign_in(caller: WebviewWindow) -> Reply<()> {
    gate(&caller, "oauth_cancel_sign_in")?;
    if let Some(cancel) = waiting().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        cancel.notify_one();
    }
    log("cancel_sign_in", true);
    Ok(())
}

/// Throws away a grant that was not attached to an account.
#[tauri::command]
#[specta::specta]
pub(crate) fn oauth_discard_grant(caller: WebviewWindow, handle: String) -> Reply<()> {
    gate(&caller, "oauth_discard_grant")?;
    drop(take_grant(&handle));
    log("discard_grant", true);
    Ok(())
}

/// Forgets an account's tokens: removes its refresh token from the OS vault
/// and drops the access token. The mail account stays and asks for a new
/// sign-in; the grant itself is revoked in the person's Microsoft or Google
/// account settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn oauth_sign_out(caller: WebviewWindow, account: String) -> Reply<()> {
    gate(&caller, "oauth_sign_out")?;
    let outcome = token_manager().sign_out(&account).await;
    log("sign_out", outcome.is_ok());
    outcome.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(lifetime: u64) -> Grant {
        Grant {
            config: ClientConfig {
                provider: Provider::Google,
                client_id: "id".into(),
                tenant: None,
            },
            client_secret: None,
            access_token: Zeroizing::new("AT".into()),
            refresh_token: Zeroizing::new("RT".into()),
            lifetime: Duration::from_secs(lifetime),
            purposes: vec![Purpose::Mail],
        }
    }

    #[test]
    fn grant_handles_are_single_use_and_bounded() {
        let handle = keep_grant(grant(3600)).unwrap();
        assert!(peek_grant(&handle).is_some());
        assert!(take_grant(&handle).is_some());
        assert!(take_grant(&handle).is_none());
        assert!(take_grant("not-a-handle").is_none());
        let handles: Vec<_> = (0..MAX_GRANTS + 2)
            .map(|_| keep_grant(grant(3600)).unwrap())
            .collect();
        assert!(lock_grants().len() <= MAX_GRANTS);
        assert!(take_grant(&handles[0]).is_none());
        for handle in &handles {
            drop(take_grant(handle));
        }
    }

    #[test]
    fn an_expiring_access_token_is_not_offered_for_testing() {
        let handle = keep_grant(grant(30)).unwrap();
        assert!(peek_grant(&handle).is_none());
        drop(take_grant(&handle));
    }

    #[test]
    fn the_browser_opener_refuses_non_https_urls() {
        assert!(!SystemBrowser.open("http://example.com/"));
        assert!(!SystemBrowser.open("file:///c:/windows/system32/calc.exe"));
        assert!(!SystemBrowser.open("https://example.com/\r\nx"));
    }

    #[test]
    fn errors_map_one_to_one() {
        assert_eq!(
            OAuthErrorV1::from(OAuthError::SignInRequired),
            OAuthErrorV1::SignInRequired
        );
        assert_eq!(
            OAuthErrorV1::from(OAuthError::StateMismatch),
            OAuthErrorV1::StateMismatch
        );
    }
}
