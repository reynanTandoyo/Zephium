//! Provider autoconfig (T-0022): Mozilla ISPDB and the provider's own
//! `config-v1.1.xml`, parsed with bounded input. Fetching goes through
//! [`ConfigFetcher`] so tests use fixtures and the desktop supplies HTTPS.

use futures::future::BoxFuture;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use crate::error::{MailError, Result};
use crate::limits::MAX_AUTOCONFIG_BYTES;
use crate::model::{email_ok, Endpoint, Security};

/// Where a configuration came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoconfigSource {
    /// Mozilla's ISP database.
    Ispdb,
    /// `autoconfig.<domain>`.
    ProviderSubdomain,
    /// `<domain>/.well-known/autoconfig`.
    WellKnown,
}

/// The outcome of [`discover`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Autoconfig {
    /// Settings were found.
    Found {
        /// Incoming server.
        imap: Endpoint,
        /// Outgoing server.
        smtp: Endpoint,
        /// Login name with placeholders resolved.
        username: String,
        /// Provider display name, when given.
        provider: Option<String>,
        /// Which lookup answered.
        source: AutoconfigSource,
    },
    /// Nothing answered; the person enters the servers by hand.
    Manual,
}

/// Fetches one HTTPS document. Implementations must refuse non-HTTPS URLs,
/// cap the body at [`MAX_AUTOCONFIG_BYTES`] and time out.
pub trait ConfigFetcher: Send + Sync {
    /// Returns the response body, or an error for any non-200 outcome.
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;
}

/// The lookup URLs for `domain`, in order.
#[must_use]
pub fn candidate_urls(domain: &str) -> [(String, AutoconfigSource); 3] {
    [
        (
            format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
            AutoconfigSource::Ispdb,
        ),
        (
            format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
            AutoconfigSource::ProviderSubdomain,
        ),
        (
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
            AutoconfigSource::WellKnown,
        ),
    ]
}

/// Tries each lookup in turn and returns the first usable configuration.
pub async fn discover(email: &str, fetcher: &dyn ConfigFetcher) -> Result<Autoconfig> {
    if !email_ok(email) {
        return Err(MailError::InvalidInput);
    }
    let (_, domain) = email.rsplit_once('@').ok_or(MailError::InvalidInput)?;
    let domain = domain.to_ascii_lowercase();
    for (url, source) in candidate_urls(&domain) {
        let Ok(body) = fetcher.fetch(&url).await else {
            continue;
        };
        if let Some(parsed) = parse_client_config(&body, email) {
            return Ok(Autoconfig::Found {
                imap: parsed.imap,
                smtp: parsed.smtp,
                username: parsed.username,
                provider: parsed.provider,
                source,
            });
        }
    }
    Ok(Autoconfig::Manual)
}

/// Servers read from one document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedConfig {
    /// Incoming server.
    pub imap: Endpoint,
    /// Outgoing server.
    pub smtp: Endpoint,
    /// Login name.
    pub username: String,
    /// Provider name.
    pub provider: Option<String>,
}

#[derive(Default)]
struct Server {
    kind: String,
    host: String,
    port: Option<u16>,
    socket: String,
    username: String,
}

fn substitute(template: &str, email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

fn endpoint(server: &Server, email: &str) -> Option<Endpoint> {
    let security = match server.socket.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => return None,
    };
    let endpoint = Endpoint {
        host: substitute(&server.host, email).to_ascii_lowercase(),
        port: server.port.unwrap_or(match security {
            Security::Tls => 993,
            Security::StartTls => 143,
        }),
        security,
    };
    endpoint.validate().ok()?;
    Some(endpoint)
}

/// Parses a `clientConfig` document. Returns `None` for malformed or
/// oversized input, or when it lacks a secure IMAP and SMTP server.
#[must_use]
pub fn parse_client_config(xml: &[u8], email: &str) -> Option<ParsedConfig> {
    if xml.len() > MAX_AUTOCONFIG_BYTES {
        return None;
    }
    let text = std::str::from_utf8(xml).ok()?;
    // A DTD is never needed and is how entity-expansion attacks arrive.
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return None;
    }
    let mut reader = Reader::from_str(text);
    let mut path: Vec<String> = Vec::new();
    let mut current: Option<Server> = None;
    let mut incoming: Vec<Server> = Vec::new();
    let mut outgoing: Vec<Server> = Vec::new();
    let mut provider: Option<String> = None;
    let mut events = 0usize;
    loop {
        events += 1;
        if events > 4096 {
            return None;
        }
        match reader.read_event().ok()? {
            Event::Start(start) => {
                let name = String::from_utf8_lossy(start.local_name().as_ref()).into_owned();
                if path.len() > 16 {
                    return None;
                }
                if name == "incomingServer" || name == "outgoingServer" {
                    let kind = start
                        .attributes()
                        .flatten()
                        .find(|attribute| attribute.key.as_ref() == b"type")
                        .map(|attribute| String::from_utf8_lossy(&attribute.value).into_owned())
                        .unwrap_or_default();
                    current = Some(Server {
                        kind,
                        ..Server::default()
                    });
                }
                path.push(name);
            }
            Event::End(_) => {
                let name = path.pop()?;
                if name == "incomingServer" {
                    incoming.extend(current.take());
                } else if name == "outgoingServer" {
                    outgoing.extend(current.take());
                }
            }
            Event::Text(text) => {
                let value = text.decode().ok()?.trim().to_owned();
                if value.is_empty() || value.len() > 512 {
                    continue;
                }
                let leaf = path.last().map(String::as_str).unwrap_or("");
                match (current.as_mut(), leaf) {
                    (Some(server), "hostname") => server.host = value,
                    (Some(server), "port") => server.port = value.parse().ok(),
                    (Some(server), "socketType") => server.socket = value,
                    (Some(server), "username") => server.username = value,
                    (None, "displayName")
                        if path.iter().any(|segment| segment == "emailProvider") =>
                    {
                        provider.get_or_insert(value);
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let imap_server = incoming
        .iter()
        .filter(|server| server.kind.eq_ignore_ascii_case("imap"))
        .find_map(|server| endpoint(server, email).map(|e| (e, server)))?;
    let smtp = outgoing
        .iter()
        .filter(|server| server.kind.eq_ignore_ascii_case("smtp"))
        .find_map(|server| endpoint(server, email))?;
    let username = if imap_server.1.username.is_empty() {
        email.to_owned()
    } else {
        substitute(&imap_server.1.username, email)
    };
    if username.is_empty() || username.chars().any(char::is_control) || username.len() > 320 {
        return None;
    }
    Some(ParsedConfig {
        imap: imap_server.0,
        smtp,
        username,
        provider,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const ISPDB: &[u8] = include_bytes!("../tests/fixtures/ispdb-example.xml");

    #[test]
    fn parses_the_ispdb_fixture_and_resolves_placeholders() {
        let parsed = parse_client_config(ISPDB, "ann@example.com").expect("config");
        assert_eq!(parsed.imap.host, "imap.example.com");
        assert_eq!(parsed.imap.port, 993);
        assert_eq!(parsed.imap.security, Security::Tls);
        assert_eq!(parsed.smtp.host, "smtp.example.com");
        assert_eq!(parsed.smtp.port, 587);
        assert_eq!(parsed.smtp.security, Security::StartTls);
        assert_eq!(parsed.username, "ann");
        assert_eq!(parsed.provider.as_deref(), Some("Example Mail"));
    }

    #[test]
    fn refuses_plaintext_oversized_and_doctype_documents() {
        let plain = String::from_utf8_lossy(ISPDB)
            .replace("SSL", "plain")
            .replace("STARTTLS", "plain");
        assert!(parse_client_config(plain.as_bytes(), "ann@example.com").is_none());
        let big = vec![b' '; MAX_AUTOCONFIG_BYTES + 1];
        assert!(parse_client_config(&big, "ann@example.com").is_none());
        let doctype = String::from_utf8_lossy(ISPDB).replace(
            "<clientConfig",
            "<!DOCTYPE x [<!ENTITY a \"b\">]><clientConfig",
        );
        assert!(parse_client_config(doctype.as_bytes(), "ann@example.com").is_none());
        assert!(parse_client_config(b"<clientConfig><emailProvider>", "a@b.co").is_none());
        assert!(parse_client_config(b"\xff\xfe", "a@b.co").is_none());
    }

    #[test]
    fn rejects_a_hostile_hostname() {
        let xml = String::from_utf8_lossy(ISPDB).replace("imap.%EMAILDOMAIN%", "evil.com/../x y");
        assert!(parse_client_config(xml.as_bytes(), "ann@example.com").is_none());
    }

    struct Fixtures {
        seen: Mutex<Vec<String>>,
        answers: Vec<(&'static str, &'static [u8])>,
    }

    impl ConfigFetcher for Fixtures {
        fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
            self.seen.lock().unwrap().push(url.to_owned());
            let found = self
                .answers
                .iter()
                .find(|(prefix, _)| url.starts_with(prefix));
            Box::pin(async move {
                found
                    .map(|(_, body)| body.to_vec())
                    .ok_or(MailError::NotFound)
            })
        }
    }

    #[tokio::test]
    async fn discovery_falls_through_the_three_sources_then_manual() {
        let provider_only = Fixtures {
            seen: Mutex::default(),
            answers: vec![("https://example.com/.well-known/", ISPDB)],
        };
        let found = discover("ann@Example.com", &provider_only).await.unwrap();
        assert!(matches!(
            found,
            Autoconfig::Found {
                source: AutoconfigSource::WellKnown,
                ..
            }
        ));
        assert_eq!(
            *provider_only.seen.lock().unwrap(),
            [
                "https://autoconfig.thunderbird.net/v1.1/example.com",
                "https://autoconfig.example.com/mail/config-v1.1.xml",
                "https://example.com/.well-known/autoconfig/mail/config-v1.1.xml",
            ]
        );
        let none = Fixtures {
            seen: Mutex::default(),
            answers: vec![],
        };
        assert_eq!(
            discover("ann@example.com", &none).await.unwrap(),
            Autoconfig::Manual
        );
        let garbage = Fixtures {
            seen: Mutex::default(),
            answers: vec![("https://autoconfig.thunderbird", b"<html>nope</html>")],
        };
        assert_eq!(
            discover("ann@example.com", &garbage).await.unwrap(),
            Autoconfig::Manual
        );
        assert_eq!(
            discover("not-an-address", &none).await,
            Err(MailError::InvalidInput)
        );
    }
}
