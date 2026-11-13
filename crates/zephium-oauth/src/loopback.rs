//! The loopback redirect receiver (RFC 8252 section 7.3): a listener on
//! `127.0.0.1` and a random port that waits, bounded in time, size and
//! connection count, for the one request the browser makes after consent.
//!
//! Every byte is untrusted: another local process or a web page may hit the
//! port too. Requests that are not the redirect are answered and ignored;
//! a redirect with the wrong `state` ends the sign-in.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::time::{timeout, timeout_at, Instant};
use zeroize::Zeroizing;

use crate::error::{OAuthError, Result};

/// Largest request head read.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// Longest authorization code accepted.
pub const MAX_CODE_BYTES: usize = 4 * 1024;
/// Longest `state` accepted.
pub const MAX_STATE_BYTES: usize = 256;
/// How long one connection may take to send its request head.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Connections served before the sign-in gives up.
pub const MAX_CONNECTIONS: usize = 8;

/// What the redirect said.
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The provider issued an authorization code.
    Code(String),
    /// The provider reported an error (the person declined, for example).
    Denied,
}

/// A parsed redirect.
#[derive(Debug, Eq, PartialEq)]
pub struct Callback {
    /// The echoed `state`.
    pub state: String,
    /// The code or the refusal.
    pub outcome: Outcome,
}

/// What to do with one request.
#[derive(Debug, Eq, PartialEq)]
pub enum Parsed {
    /// The redirect.
    Callback(Callback),
    /// Anything else: answer with this status and keep waiting.
    Ignore(u16),
}

fn header_value<'a>(lines: &[&'a str], name: &str) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// Parses one request head received on `port`. Pure, so the hostile cases
/// are tested without sockets.
#[must_use]
pub fn parse_request(head: &[u8], port: u16) -> Parsed {
    if head.len() > MAX_REQUEST_BYTES {
        return Parsed::Ignore(431);
    }
    let Ok(text) = std::str::from_utf8(head) else {
        return Parsed::Ignore(400);
    };
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let headers: Vec<&str> = lines.take_while(|line| !line.is_empty()).collect();
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Parsed::Ignore(400);
    };
    if !version.starts_with("HTTP/1.") {
        return Parsed::Ignore(400);
    }
    if method != "GET" {
        return Parsed::Ignore(405);
    }
    // A web page can aim a request at the port; only the literal loopback
    // names are served, which also defeats DNS rebinding.
    let host_ok = header_value(&headers, "host").is_some_and(|host| {
        host == format!("127.0.0.1:{port}")
            || host.eq_ignore_ascii_case(&format!("localhost:{port}"))
    });
    if !host_ok {
        return Parsed::Ignore(400);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return Parsed::Ignore(404);
    }
    let (mut code, mut state, mut error) = (None, None, false);
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" => {
                if code.is_some() {
                    return Parsed::Ignore(400);
                }
                code = Some(value.into_owned());
            }
            "state" => {
                if state.is_some() {
                    return Parsed::Ignore(400);
                }
                state = Some(value.into_owned());
            }
            "error" => error = true,
            _ => {}
        }
    }
    let Some(state) = state.filter(|s| !s.is_empty() && s.len() <= MAX_STATE_BYTES) else {
        return Parsed::Ignore(400);
    };
    if error {
        return Parsed::Callback(Callback {
            state,
            outcome: Outcome::Denied,
        });
    }
    match code {
        Some(code) if !code.is_empty() && code.len() <= MAX_CODE_BYTES => {
            Parsed::Callback(Callback {
                state,
                outcome: Outcome::Code(code),
            })
        }
        _ => Parsed::Ignore(400),
    }
}

fn same(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Zephium</title>\
<body style=\"font-family:system-ui;margin:3em\"><h1>Zephium</h1>\
<p>You can close this tab and return to Zephium.</p></body>";

async fn respond(stream: &mut TcpStream, status: u16) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Request Header Fields Too Large",
    };
    let body = if status == 200 { PAGE } else { "" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
Content-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\
Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\n\
Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = timeout(READ_TIMEOUT, async {
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    })
    .await;
}

/// Reads the request head: up to the blank line, [`MAX_REQUEST_BYTES`] and
/// [`READ_TIMEOUT`]. `None` means the peer sent too much or too slowly.
async fn read_head(stream: &mut TcpStream) -> Option<Vec<u8>> {
    timeout(READ_TIMEOUT, async {
        let mut head = Vec::with_capacity(512);
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            head.extend_from_slice(&chunk[..read]);
            if head.windows(4).any(|window| window == b"\r\n\r\n") {
                return Some(head);
            }
            if head.len() > MAX_REQUEST_BYTES {
                return Some(head);
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// A bound loopback listener.
#[derive(Debug)]
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    /// Binds `127.0.0.1` on a random free port. Never a wildcard address.
    pub async fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| OAuthError::Internal)?;
        let port = listener
            .local_addr()
            .map_err(|_| OAuthError::Internal)?
            .port();
        Ok(Self { listener, port })
    }

    /// The port.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The redirect URI with the literal loopback address.
    #[must_use]
    pub fn redirect_uri(&self) -> String {
        self.redirect_uri_for("127.0.0.1")
    }

    /// The redirect URI naming the loopback as `host`: `127.0.0.1` or
    /// `localhost` (the only two names the receiver serves).
    #[must_use]
    pub fn redirect_uri_for(&self, host: &str) -> String {
        format!("http://{host}:{}/", self.port)
    }

    /// Waits for the redirect and returns its code. Gives up after `wait`
    /// ([`OAuthError::Timeout`]), on `cancel` ([`OAuthError::Cancelled`]),
    /// after [`MAX_CONNECTIONS`] connections without a redirect
    /// ([`OAuthError::Protocol`]), or at a redirect for another sign-in
    /// ([`OAuthError::StateMismatch`]). Consumes the listener: the port
    /// closes when this returns.
    pub async fn receive(
        self,
        expected_state: &str,
        wait: Duration,
        cancel: &Notify,
    ) -> Result<Zeroizing<String>> {
        let deadline = Instant::now() + wait;
        for _ in 0..MAX_CONNECTIONS {
            let mut stream = tokio::select! {
                () = cancel.notified() => return Err(OAuthError::Cancelled),
                accepted = timeout_at(deadline, self.listener.accept()) => {
                    match accepted {
                        Err(_) => return Err(OAuthError::Timeout),
                        Ok(Err(_)) => continue,
                        Ok(Ok((stream, _))) => stream,
                    }
                }
            };
            let Some(head) = read_head(&mut stream).await else {
                respond(&mut stream, 400).await;
                continue;
            };
            match parse_request(&head, self.port) {
                Parsed::Ignore(status) => respond(&mut stream, status).await,
                Parsed::Callback(callback) => {
                    if !same(&callback.state, expected_state) {
                        respond(&mut stream, 400).await;
                        return Err(OAuthError::StateMismatch);
                    }
                    respond(&mut stream, 200).await;
                    return match callback.outcome {
                        Outcome::Code(code) => Ok(Zeroizing::new(code)),
                        Outcome::Denied => Err(OAuthError::Denied),
                    };
                }
            }
        }
        Err(OAuthError::Protocol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn get(target: &str, host: &str) -> Vec<u8> {
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: t\r\n\r\n").into_bytes()
    }

    #[test]
    fn parses_a_good_redirect() {
        let parsed = parse_request(
            &get("/?code=ab%2Fc&state=xyz&scope=s", "127.0.0.1:5000"),
            5000,
        );
        assert_eq!(
            parsed,
            Parsed::Callback(Callback {
                state: "xyz".into(),
                outcome: Outcome::Code("ab/c".into())
            })
        );
        let named = parse_request(&get("/?code=c&state=s", "LocalHost:5000"), 5000);
        assert!(matches!(named, Parsed::Callback(_)));
    }

    #[test]
    fn provider_errors_become_denied_without_their_text() {
        let parsed = parse_request(
            &get(
                "/?error=access_denied&error_description=SECRET&state=s",
                "127.0.0.1:1",
            ),
            1,
        );
        assert_eq!(
            parsed,
            Parsed::Callback(Callback {
                state: "s".into(),
                outcome: Outcome::Denied
            })
        );
    }

    #[test]
    fn hostile_requests_are_ignored() {
        let ok_host = "127.0.0.1:7";
        let ignore = |bytes: Vec<u8>| matches!(parse_request(&bytes, 7), Parsed::Ignore(_));
        // Wrong or missing Host (DNS rebinding, other ports, other names).
        assert!(ignore(get("/?code=c&state=s", "evil.example:7")));
        assert!(ignore(get("/?code=c&state=s", "127.0.0.1:8")));
        assert!(ignore(get("/?code=c&state=s", "127.0.0.1")));
        assert!(ignore(b"GET /?code=c&state=s HTTP/1.1\r\n\r\n".to_vec()));
        // Wrong method, path, version, shape.
        assert!(ignore(
            format!("POST /?code=c&state=s HTTP/1.1\r\nHost: {ok_host}\r\n\r\n").into_bytes()
        ));
        assert!(ignore(get("/other?code=c&state=s", ok_host)));
        assert!(ignore(get("//?code=c&state=s", ok_host)));
        assert!(ignore(b"GET / HTTP/0.9 extra\r\n\r\n".to_vec()));
        assert!(ignore(b"garbage".to_vec()));
        assert!(ignore(vec![0xff, 0xfe, 0x00]));
        assert!(ignore(Vec::new()));
        // Missing, empty, duplicated or oversized parameters.
        assert!(ignore(get("/", ok_host)));
        assert!(ignore(get("/?code=c", ok_host)));
        assert!(ignore(get("/?state=s", ok_host)));
        assert!(ignore(get("/?code=&state=s", ok_host)));
        assert!(ignore(get("/?code=a&code=b&state=s", ok_host)));
        assert!(ignore(get("/?code=a&state=s&state=t", ok_host)));
        let long_code = format!("/?code={}&state=s", "a".repeat(MAX_CODE_BYTES + 1));
        assert!(ignore(get(&long_code, ok_host)));
        let long_state = format!("/?code=c&state={}", "a".repeat(MAX_STATE_BYTES + 1));
        assert!(ignore(get(&long_state, ok_host)));
        // Oversized head.
        let mut big = get("/?code=c&state=s", ok_host);
        big.extend(std::iter::repeat_n(b'x', MAX_REQUEST_BYTES));
        assert_eq!(parse_request(&big, 7), Parsed::Ignore(431));
    }

    async fn send(port: u16, bytes: &[u8]) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(bytes).await.unwrap();
        let mut reply = Vec::new();
        let _ = timeout(Duration::from_secs(5), stream.read_to_end(&mut reply)).await;
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[tokio::test]
    async fn receives_the_code_after_ignoring_noise() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        assert!(loopback.redirect_uri().starts_with("http://127.0.0.1:"));
        let cancel = Arc::new(Notify::new());
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                loopback
                    .receive("good", Duration::from_secs(10), &cancel)
                    .await
            }
        });
        let favicon = send(port, &get("/favicon.ico", &format!("127.0.0.1:{port}"))).await;
        assert!(favicon.starts_with("HTTP/1.1 404"));
        let rebinding = send(port, &get("/?code=evil&state=good", "evil.example")).await;
        assert!(rebinding.starts_with("HTTP/1.1 400"));
        let page = send(
            port,
            &get("/?code=thecode&state=good", &format!("127.0.0.1:{port}")),
        )
        .await;
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(!page.contains("thecode"));
        let code = task.await.unwrap().unwrap();
        assert_eq!(&*code, "thecode");
    }

    #[tokio::test]
    async fn refuses_a_state_mismatch() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        let cancel = Notify::new();
        let request = get("/?code=c&state=forged", &format!("127.0.0.1:{port}"));
        let (result, reply) = tokio::join!(
            loopback.receive("expected", Duration::from_secs(10), &cancel),
            send(port, &request)
        );
        assert_eq!(result.unwrap_err(), OAuthError::StateMismatch);
        assert!(reply.starts_with("HTTP/1.1 400"));
    }

    #[tokio::test]
    async fn a_refusal_from_the_provider_is_denied() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        let cancel = Notify::new();
        let request = get(
            "/?error=access_denied&state=s",
            &format!("127.0.0.1:{port}"),
        );
        let (result, _) = tokio::join!(
            loopback.receive("s", Duration::from_secs(10), &cancel),
            send(port, &request)
        );
        assert_eq!(result.unwrap_err(), OAuthError::Denied);
    }

    #[tokio::test]
    async fn oversized_and_endless_requests_are_bounded() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        let cancel = Notify::new();
        let flood = async {
            // No blank line, more than the cap: answered and dropped.
            let reply = send(port, &vec![b'a'; MAX_REQUEST_BYTES + 2048]).await;
            assert!(
                reply.starts_with("HTTP/1.1 431")
                    || reply.is_empty()
                    || reply.starts_with("HTTP/1.1 400")
            );
            send(
                port,
                &get("/?code=ok&state=s", &format!("127.0.0.1:{port}")),
            )
            .await
        };
        let (result, _) = tokio::join!(
            loopback.receive("s", Duration::from_secs(10), &cancel),
            flood
        );
        assert_eq!(&*result.unwrap(), "ok");
    }

    #[tokio::test]
    async fn gives_up_after_too_many_connections() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port();
        let cancel = Notify::new();
        let noise = async {
            for _ in 0..MAX_CONNECTIONS {
                send(port, &get("/x", &format!("127.0.0.1:{port}"))).await;
            }
        };
        let (result, ()) = tokio::join!(
            loopback.receive("s", Duration::from_secs(10), &cancel),
            noise
        );
        assert_eq!(result.unwrap_err(), OAuthError::Protocol);
    }

    #[tokio::test]
    async fn times_out_and_cancels() {
        let loopback = Loopback::bind().await.unwrap();
        let cancel = Notify::new();
        let result = loopback
            .receive("s", Duration::from_millis(50), &cancel)
            .await;
        assert_eq!(result.unwrap_err(), OAuthError::Timeout);

        let loopback = Loopback::bind().await.unwrap();
        let cancel = Notify::new();
        cancel.notify_one();
        let result = loopback
            .receive("s", Duration::from_secs(10), &cancel)
            .await;
        assert_eq!(result.unwrap_err(), OAuthError::Cancelled);
    }

    #[test]
    fn state_comparison_checks_length_and_content() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
    }
}
