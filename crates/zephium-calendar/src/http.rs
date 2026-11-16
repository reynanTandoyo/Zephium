//! The only network boundary of the crate. Everything above it talks to a
//! [`HttpClient`] trait object so tests use fixtures and never the network.

use crate::error::{CalendarError, Result};
use async_trait::async_trait;
use std::fmt;
use url::Url;
use zeroize::Zeroizing;

/// Largest response body read from any calendar server.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// Redirects followed per request.
pub const MAX_REDIRECTS: usize = 5;

/// HTTP methods the calendar protocols use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    /// `GET`
    Get,
    /// `PUT`
    Put,
    /// `POST`
    Post,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `PROPFIND`
    Propfind,
    /// `REPORT`
    Report,
}

impl Method {
    /// Method token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Post => "POST",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Propfind => "PROPFIND",
            Self::Report => "REPORT",
        }
    }
}

/// A password held in memory only as long as needed; never printed.
#[derive(Clone)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// Wraps a secret.
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The secret text. Keep the borrow short-lived.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

/// HTTP Basic credentials.
#[derive(Clone, Debug)]
pub struct Credentials {
    /// User name.
    pub username: String,
    /// Password.
    pub password: Secret,
}

/// One request. Redirects are handled by [`send`], not by the client.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    /// Method.
    pub method: Method,
    /// Absolute URL.
    pub url: Url,
    /// Extra headers (names are lower-case ASCII).
    pub headers: Vec<(&'static str, String)>,
    /// Request body.
    pub body: Vec<u8>,
    /// Basic credentials, sent only to the origin they were given for.
    pub credentials: Option<Credentials>,
    /// OAuth bearer token, sent only to the origin it was given for.
    pub bearer: Option<Secret>,
}

impl HttpRequest {
    /// A request without body or credentials.
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: Vec::new(),
            credentials: None,
            bearer: None,
        }
    }

    /// Adds a header.
    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// Sets the body and its content type.
    pub fn body(mut self, content_type: &'static str, body: Vec<u8>) -> Self {
        self.headers.push(("content-type", content_type.to_owned()));
        self.body = body;
        self
    }

    /// Sets the OAuth bearer token.
    pub fn bearer(mut self, token: Option<Secret>) -> Self {
        self.bearer = token;
        self
    }

    /// Sets Basic credentials.
    pub fn credentials(mut self, credentials: Option<Credentials>) -> Self {
        self.credentials = credentials;
        self
    }
}

/// A response with every status passed through.
#[derive(Clone, Debug, Default)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Headers with lower-case names.
    pub headers: Vec<(String, String)>,
    /// Body, at most [`MAX_RESPONSE_BYTES`].
    pub body: Vec<u8>,
    /// Address that produced this response after redirects; filled by [`send`].
    pub final_url: Option<Url>,
}

impl HttpResponse {
    /// First header named `name` (lower-case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The body as UTF-8 text.
    pub fn text(&self) -> Result<&str> {
        std::str::from_utf8(&self.body).map_err(|_| CalendarError::Parse)
    }
}

/// Sends one request without following redirects or interpreting status.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Executes `request`.
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse>;
}

/// Accepts `https`, and `http` only for loopback hosts.
pub fn check_url(url: &Url) -> Result<()> {
    match url.scheme() {
        "https" => Ok(()),
        "http" => match url.host() {
            Some(url::Host::Domain(name)) if name.eq_ignore_ascii_case("localhost") => Ok(()),
            Some(url::Host::Ipv4(address)) if address.is_loopback() => Ok(()),
            Some(url::Host::Ipv6(address)) if address.is_loopback() => Ok(()),
            _ => Err(CalendarError::Insecure),
        },
        _ => Err(CalendarError::InvalidInput),
    }
    .and_then(|()| {
        if url.username().is_empty() && url.password().is_none() {
            Ok(())
        } else {
            // Credentials never travel inside the address.
            Err(CalendarError::InvalidInput)
        }
    })
}

/// Turns a `webcal(s)://` or `http(s)://` string into a checked URL.
pub fn parse_address(text: &str) -> Result<Url> {
    let text = text.trim();
    if text.is_empty() || text.len() > 2048 {
        return Err(CalendarError::InvalidInput);
    }
    let lowered = text.to_ascii_lowercase();
    let text = if lowered.starts_with("webcals://") {
        format!("https://{}", &text[10..])
    } else if lowered.starts_with("webcal://") {
        format!("https://{}", &text[9..])
    } else {
        text.to_owned()
    };
    let url = Url::parse(&text).map_err(|_| CalendarError::InvalidInput)?;
    check_url(&url)?;
    Ok(url)
}

/// Whether `url` names a host that is not on the public internet: a literal
/// loopback, private, link-local, unique-local, CGNAT or unspecified address
/// (IPv4-mapped IPv6 judged as IPv4), `localhost` or a `.localhost`, `.local`,
/// `.internal`, `.lan` or `.home.arpa` name, or a name without a dot (an
/// intranet short name). Names are not resolved here, so a public name that
/// points at a private address is not caught.
#[must_use]
pub fn is_private_host(url: &Url) -> bool {
    fn v4(address: std::net::Ipv4Addr) -> bool {
        let [a, b, ..] = address.octets();
        address.is_loopback()
            || address.is_private()
            || address.is_link_local()
            || address.is_unspecified()
            || address.is_broadcast()
            || (a == 100 && (64..=127).contains(&b))
            || a == 0
    }
    match url.host() {
        Some(url::Host::Ipv4(address)) => v4(address),
        Some(url::Host::Ipv6(address)) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return v4(mapped);
            }
            let first = address.segments()[0];
            address.is_loopback()
                || address.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            !name.contains('.')
                || name == "localhost"
                || [".localhost", ".local", ".internal", ".lan", ".home.arpa"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
        None => true,
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Sends `request`, following up to [`MAX_REDIRECTS`] redirects. Credentials
/// are dropped when a redirect leaves the origin; a downgrade to plain HTTP
/// is refused. A redirect from a public host to a private, loopback or
/// link-local one is refused ([`is_private_host`]): the address the person
/// typed may be internal, but a server on the internet cannot send the client
/// into the local network. A request that already targets a private host may
/// be redirected to another private host.
pub async fn send(client: &dyn HttpClient, mut request: HttpRequest) -> Result<HttpResponse> {
    for _ in 0..=MAX_REDIRECTS {
        check_url(&request.url)?;
        let response = client.execute(request.clone()).await?;
        let mut response = response;
        response.final_url = Some(request.url.clone());
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(location) = response.header("location") else {
            return Ok(response);
        };
        let next = request
            .url
            .join(location)
            .map_err(|_| CalendarError::Server)?;
        check_url(&next)?;
        if is_private_host(&next) && !is_private_host(&request.url) {
            return Err(CalendarError::Forbidden);
        }
        if !same_origin(&request.url, &next) {
            request.credentials = None;
            request.bearer = None;
        }
        if response.status == 303 && request.method != Method::Get {
            request.method = Method::Get;
            request.body.clear();
            request.headers.retain(|(name, _)| *name != "content-type");
        }
        request.url = next;
    }
    Err(CalendarError::Server)
}

/// Maps a final status to the closed error set. A rate limit (429) is
/// [`CalendarError::Limit`].
pub fn check_status(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 => Err(CalendarError::Unauthorized),
        403 => Err(CalendarError::Forbidden),
        404 | 410 => Err(CalendarError::NotFound),
        405 | 501 => Err(CalendarError::Unsupported),
        409 | 412 => Err(CalendarError::Conflict),
        413 | 507 => Err(CalendarError::TooLarge),
        429 => Err(CalendarError::Limit),
        _ => Err(CalendarError::Server),
    }
}

/// Production client over `reqwest` with rustls and the system proxy.
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    /// Builds the client: no redirects (handled by [`send`]), bounded time.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(45))
            .user_agent("Zephium-Calendar")
            .build()
            .map_err(|_| CalendarError::Network)?;
        Ok(Self { client })
    }
}

fn map_transport(error: &reqwest::Error) -> CalendarError {
    if error.is_timeout() {
        CalendarError::Timeout
    } else {
        CalendarError::Network
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
            .map_err(|_| CalendarError::InvalidInput)?;
        let mut builder = self.client.request(method, request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(credentials) = &request.credentials {
            builder =
                builder.basic_auth(&credentials.username, Some(credentials.password.expose()));
        }
        if let Some(token) = &request.bearer {
            builder = builder.bearer_auth(token.expose());
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let mut response = builder.send().await.map_err(|e| map_transport(&e))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().ok()?.to_owned(),
                ))
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| map_transport(&e))? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(CalendarError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
            final_url: None,
        })
    }
}
