//! The authorization code flow with PKCE for a native public client: open
//! the system browser at the consent page, receive the redirect on the
//! loopback listener, check `state`, exchange the code, and later refresh.

use std::fmt;
use std::time::Duration;

use tokio::sync::Notify;
use zeroize::Zeroizing;

use crate::error::{OAuthError, Result};
use crate::http::HttpClient;
use crate::loopback::Loopback;
use crate::pkce;
use crate::provider::{client_secret_ok, ClientConfig, Provider, Purpose};
use crate::token::{parse_response, TokenSet};

/// How long a person gets to finish consent.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(180);

/// Opens a URL in the system browser. The desktop implements this with the
/// platform's own opener; tests record the URL.
pub trait BrowserOpener: Send + Sync {
    /// Opens `url`; false when it could not be opened.
    fn open(&self, url: &str) -> bool;
}

/// What a completed sign-in produced. Secrets are wiped on drop.
pub struct Grant {
    /// Who signed in.
    pub config: ClientConfig,
    /// The client secret the person supplied, if any (Google desktop clients).
    pub client_secret: Option<Zeroizing<String>>,
    /// The first access token. Memory only.
    pub access_token: Zeroizing<String>,
    /// The refresh token: goes to the vault, nowhere else.
    pub refresh_token: Zeroizing<String>,
    /// Access token lifetime.
    pub lifetime: Duration,
    /// What the grant covers.
    pub purposes: Vec<Purpose>,
}

impl fmt::Debug for Grant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Grant")
            .field("provider", &self.config.provider)
            .finish_non_exhaustive()
    }
}

/// One sign-in to perform.
#[derive(Clone, Copy, Debug)]
pub struct SignInRequest<'a> {
    /// Provider, client ID, tenant.
    pub config: &'a ClientConfig,
    /// The client secret, only when the provider insists on one.
    pub client_secret: Option<&'a str>,
    /// What to ask permission for.
    pub purposes: &'a [Purpose],
    /// How long to wait for consent.
    pub wait: Duration,
}

impl ClientConfig {
    /// The host name in the redirect URI. Microsoft matches the registered
    /// `http://localhost` (port ignored) and does not treat `127.0.0.1` as
    /// the same; Google's desktop clients use `127.0.0.1`. The listener
    /// itself only ever binds `127.0.0.1`.
    #[must_use]
    pub fn redirect_host(&self) -> &'static str {
        match self.provider {
            Provider::Microsoft => "localhost",
            Provider::Google => "127.0.0.1",
        }
    }
}

/// Builds the consent URL.
pub fn authorization_url(
    config: &ClientConfig,
    purposes: &[Purpose],
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String> {
    config.validate()?;
    let mut url =
        url::Url::parse(&config.authorize_endpoint()).map_err(|_| OAuthError::InvalidInput)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &config.scopes(purposes))
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        match config.provider {
            Provider::Google => {
                // Without these Google may issue no refresh token.
                query
                    .append_pair("access_type", "offline")
                    .append_pair("prompt", "consent");
            }
            Provider::Microsoft => {
                query.append_pair("response_mode", "query");
            }
        }
    }
    Ok(url.into())
}

fn base_form(
    config: &ClientConfig,
    client_secret: Option<&str>,
    grant_type: &str,
) -> Vec<(String, String)> {
    let mut form = vec![
        ("grant_type".to_owned(), grant_type.to_owned()),
        ("client_id".to_owned(), config.client_id.clone()),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret".to_owned(), secret.to_owned()));
    }
    form
}

/// The authorization-code exchange request body.
#[must_use]
pub fn exchange_form(
    config: &ClientConfig,
    client_secret: Option<&str>,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Vec<(String, String)> {
    let mut form = base_form(config, client_secret, "authorization_code");
    form.push(("code".to_owned(), code.to_owned()));
    form.push(("redirect_uri".to_owned(), redirect_uri.to_owned()));
    form.push(("code_verifier".to_owned(), verifier.to_owned()));
    form
}

/// The refresh request body. Microsoft issues one access token per
/// resource, so the scope names the one wanted; Google's covers all grants.
#[must_use]
pub fn refresh_form(
    config: &ClientConfig,
    client_secret: Option<&str>,
    refresh_token: &str,
    purpose: Purpose,
) -> Vec<(String, String)> {
    let mut form = base_form(config, client_secret, "refresh_token");
    form.push(("refresh_token".to_owned(), refresh_token.to_owned()));
    if config.provider == Provider::Microsoft {
        form.push(("scope".to_owned(), config.scopes(&[purpose])));
    }
    form
}

/// Runs the whole sign-in. `cancel` aborts the wait ([`OAuthError::Cancelled`]).
pub async fn sign_in(
    http: &dyn HttpClient,
    opener: &dyn BrowserOpener,
    request: SignInRequest<'_>,
    cancel: &Notify,
) -> Result<Grant> {
    request.config.validate()?;
    if request.purposes.is_empty() || request.client_secret.is_some_and(|s| !client_secret_ok(s)) {
        return Err(OAuthError::InvalidInput);
    }
    let loopback = Loopback::bind().await?;
    let redirect_uri = loopback.redirect_uri_for(request.config.redirect_host());
    let verifier = pkce::verifier()?;
    let state = pkce::state()?;
    let url = authorization_url(
        request.config,
        request.purposes,
        &redirect_uri,
        &state,
        &pkce::challenge(&verifier),
    )?;
    if !opener.open(&url) {
        return Err(OAuthError::Internal);
    }
    let code = loopback.receive(&state, request.wait, cancel).await?;
    let form = exchange_form(
        request.config,
        request.client_secret,
        &code,
        &redirect_uri,
        &verifier,
    );
    let response = http
        .post_form(&request.config.token_endpoint(), form)
        .await?;
    let tokens = parse_response(&response)?;
    // Without a refresh token the sign-in would not outlive the access token.
    let refresh_token = tokens.refresh_token.ok_or(OAuthError::Protocol)?;
    Ok(Grant {
        config: request.config.clone(),
        client_secret: request.client_secret.map(|s| Zeroizing::new(s.to_owned())),
        access_token: tokens.access_token,
        refresh_token,
        lifetime: tokens.lifetime,
        purposes: request.purposes.to_vec(),
    })
}

/// Exchanges a refresh token for a new access token.
pub async fn refresh(
    http: &dyn HttpClient,
    config: &ClientConfig,
    client_secret: Option<&str>,
    refresh_token: &str,
    purpose: Purpose,
) -> Result<TokenSet> {
    config.validate()?;
    let form = refresh_form(config, client_secret, refresh_token, purpose);
    let response = http.post_form(&config.token_endpoint(), form).await?;
    parse_response(&response)
}

#[cfg(test)]
pub(crate) mod fakes {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::http::HttpResponse;
    use crate::BoxFuture;

    /// One recorded post: URL and form.
    pub type Post = (String, Vec<(String, String)>);

    /// Answers posts from a queue and records them.
    #[derive(Default)]
    pub struct FakeHttp {
        pub answers: Mutex<VecDeque<Result<HttpResponse>>>,
        pub posts: Mutex<Vec<Post>>,
    }

    impl FakeHttp {
        pub fn answer(&self, status: u16, body: &str) {
            self.answers.lock().unwrap().push_back(Ok(HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            }));
        }
    }

    impl HttpClient for FakeHttp {
        fn post_form<'a>(
            &'a self,
            url: &'a str,
            form: Vec<(String, String)>,
        ) -> BoxFuture<'a, Result<HttpResponse>> {
            self.posts.lock().unwrap().push((url.to_owned(), form));
            let answer = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(OAuthError::Offline));
            Box::pin(async move { answer })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::fakes::FakeHttp;
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    fn config(provider: Provider) -> ClientConfig {
        ClientConfig {
            provider,
            client_id: "client-1".into(),
            tenant: None,
        }
    }

    fn query(url: &str) -> std::collections::HashMap<String, String> {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    /// Plays the browser: records the URL, then calls the redirect with a
    /// state of the test's choosing.
    struct Browser {
        reply_state: Option<&'static str>,
        seen: Mutex<Option<String>>,
    }

    impl BrowserOpener for Browser {
        fn open(&self, url: &str) -> bool {
            *self.seen.lock().unwrap() = Some(url.to_owned());
            let params = query(url);
            let redirect = url::Url::parse(&params["redirect_uri"]).unwrap();
            let port = redirect.port().unwrap();
            let state = self
                .reply_state
                .map_or_else(|| params["state"].clone(), str::to_owned);
            tokio::spawn(async move {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                let request = format!(
                    "GET /?code=auth-code&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
                );
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink).await;
            });
            true
        }
    }

    #[test]
    fn consent_url_carries_pkce_state_and_scopes() {
        let url = authorization_url(
            &config(Provider::Google),
            &[Purpose::Mail],
            "http://127.0.0.1:9/",
            "st",
            "ch",
        )
        .unwrap();
        let q = query(&url);
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], "ch");
        assert_eq!(q["state"], "st");
        assert_eq!(q["scope"], "https://mail.google.com/");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:9/");
        let ms = authorization_url(
            &config(Provider::Microsoft),
            &[Purpose::Mail],
            "http://localhost:9/",
            "st",
            "ch",
        )
        .unwrap();
        assert!(ms.contains("/common/oauth2/v2.0/authorize?"));
        assert!(query(&ms)["scope"].contains("offline_access"));
        assert!(authorization_url(
            &ClientConfig {
                client_id: "a b".into(),
                ..config(Provider::Google)
            },
            &[Purpose::Mail],
            "http://127.0.0.1:9/",
            "st",
            "ch"
        )
        .is_err());
    }

    #[tokio::test]
    async fn signs_in_and_exchanges_the_code_with_the_verifier() {
        let http = FakeHttp::default();
        http.answer(
            200,
            r#"{"access_token":"AT","refresh_token":"RT","token_type":"Bearer","expires_in":3600}"#,
        );
        let browser = Browser {
            reply_state: None,
            seen: Mutex::new(None),
        };
        let cfg = config(Provider::Microsoft);
        let grant = sign_in(
            &http,
            &browser,
            SignInRequest {
                config: &cfg,
                client_secret: None,
                purposes: &[Purpose::Mail],
                wait: Duration::from_secs(10),
            },
            &Notify::new(),
        )
        .await
        .unwrap();
        assert_eq!(&*grant.access_token, "AT");
        assert_eq!(&*grant.refresh_token, "RT");
        assert_eq!(format!("{grant:?}"), "Grant { provider: Microsoft, .. }");

        let seen = browser.seen.lock().unwrap().clone().unwrap();
        let consent = query(&seen);
        assert!(consent["redirect_uri"].starts_with("http://localhost:"));
        let posts = http.posts.lock().unwrap();
        let (endpoint, form) = &posts[0];
        assert_eq!(
            endpoint,
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        let form: std::collections::HashMap<_, _> = form.iter().cloned().collect();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "auth-code");
        assert_eq!(form["redirect_uri"], consent["redirect_uri"]);
        assert_eq!(
            pkce::challenge(&form["code_verifier"]),
            consent["code_challenge"]
        );
        assert!(!form.contains_key("client_secret"));
    }

    #[tokio::test]
    async fn a_forged_state_never_reaches_the_token_endpoint() {
        let http = FakeHttp::default();
        let browser = Browser {
            reply_state: Some("forged"),
            seen: Mutex::new(None),
        };
        let cfg = config(Provider::Google);
        let result = sign_in(
            &http,
            &browser,
            SignInRequest {
                config: &cfg,
                client_secret: Some("GOCSPX-x"),
                purposes: &[Purpose::Mail],
                wait: Duration::from_millis(500),
            },
            &Notify::new(),
        )
        .await;
        assert_eq!(result.unwrap_err(), OAuthError::Timeout);
        assert!(http.posts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_sign_in_without_a_refresh_token_fails() {
        let http = FakeHttp::default();
        http.answer(200, r#"{"access_token":"AT","token_type":"Bearer"}"#);
        let browser = Browser {
            reply_state: None,
            seen: Mutex::new(None),
        };
        let cfg = config(Provider::Google);
        let result = sign_in(
            &http,
            &browser,
            SignInRequest {
                config: &cfg,
                client_secret: None,
                purposes: &[Purpose::Mail],
                wait: Duration::from_secs(10),
            },
            &Notify::new(),
        )
        .await;
        assert_eq!(result.unwrap_err(), OAuthError::Protocol);
    }

    #[tokio::test]
    async fn refuses_bad_input_before_opening_the_browser() {
        struct Never;
        impl BrowserOpener for Never {
            fn open(&self, _: &str) -> bool {
                panic!("must not open")
            }
        }
        let http = FakeHttp::default();
        let cfg = config(Provider::Google);
        let request = |purposes: &'static [Purpose], secret| SignInRequest {
            config: &cfg,
            client_secret: secret,
            purposes,
            wait: Duration::from_secs(1),
        };
        for bad in [request(&[], None), request(&[Purpose::Mail], Some("a b"))] {
            let result = sign_in(&http, &Never, bad, &Notify::new()).await;
            assert_eq!(result.unwrap_err(), OAuthError::InvalidInput);
        }
    }

    #[tokio::test]
    async fn refresh_posts_a_refresh_grant() {
        let http = FakeHttp::default();
        http.answer(
            200,
            r#"{"access_token":"AT2","token_type":"Bearer","expires_in":1800}"#,
        );
        let cfg = config(Provider::Microsoft);
        let set = refresh(&http, &cfg, None, "RT", Purpose::Mail)
            .await
            .unwrap();
        assert_eq!(&*set.access_token, "AT2");
        assert!(set.refresh_token.is_none());
        let posts = http.posts.lock().unwrap();
        let form: std::collections::HashMap<_, _> = posts[0].1.iter().cloned().collect();
        assert_eq!(form["grant_type"], "refresh_token");
        assert_eq!(form["refresh_token"], "RT");
        assert!(form["scope"].contains("IMAP.AccessAsUser.All"));

        let google = refresh_form(&config(Provider::Google), Some("sec"), "RT", Purpose::Mail);
        let google: std::collections::HashMap<_, _> = google.into_iter().collect();
        assert_eq!(google["client_secret"], "sec");
        assert!(!google.contains_key("scope"));
    }
}
