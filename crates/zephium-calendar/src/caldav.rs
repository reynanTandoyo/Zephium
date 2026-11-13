//! CalDAV (RFC 4791) and WebDAV sync (RFC 6578) client over [`HttpClient`].

use crate::error::{CalendarError, Result};
use crate::http::{
    check_status, check_url, parse_address, send, Credentials, HttpClient, HttpRequest,
    HttpResponse, Method,
};
use crate::ical;
use crate::model::Event;
use crate::xml::{self, Node};
use chrono::{DateTime, Utc};
use url::Url;

/// Most calendars read from one account.
pub const MAX_CALENDARS: usize = 100;
/// Most resources read from one REPORT.
pub const MAX_RESOURCES: usize = 20_000;
const MULTIGET_BATCH: usize = 100;

const NS: &str = "xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" \
xmlns:a=\"http://apple.com/ns/ical/\" xmlns:cs=\"http://calendarserver.org/ns/\"";

/// A calendar collection on the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteCalendar {
    /// Collection address.
    pub url: Url,
    /// Display name (falls back to the last path segment).
    pub name: String,
    /// `#RRGGBB` when the server sets one.
    pub color: Option<String>,
    /// `true` when the user may not change events.
    pub read_only: bool,
}

/// One `.ics` resource with the events parsed from it.
#[derive(Clone, Debug)]
pub struct RemoteResource {
    /// Absolute resource address.
    pub href: Url,
    /// ETag as sent by the server.
    pub etag: Option<String>,
    /// Events found in the resource (empty when unreadable).
    pub events: Vec<Event>,
    /// `true` while the server listed the resource without its data.
    pub pending: bool,
    /// The `calendar-data` text as sent, kept for the persistent cache.
    pub raw: Option<String>,
}

/// Result of comparing a server listing with known ETags.
#[derive(Clone, Debug)]
pub struct RangeDelta {
    /// New resources and those whose ETag differs, with their data.
    pub changed: Vec<RemoteResource>,
    /// Every resource the server listed (changed or not); anything cached but
    /// missing here is gone.
    pub present: Vec<Url>,
}

/// Result of a `sync-collection` REPORT.
#[derive(Clone, Debug)]
pub struct SyncChanges {
    /// New or changed resources.
    pub changed: Vec<RemoteResource>,
    /// Addresses of deleted resources.
    pub removed: Vec<Url>,
    /// Token for the next call.
    pub sync_token: Option<String>,
}

/// Collection fingerprint used to skip unchanged calendars.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectionState {
    /// `getctag`.
    pub ctag: Option<String>,
    /// `sync-token`.
    pub sync_token: Option<String>,
}

// -- multistatus -------------------------------------------------------------

struct Response {
    href: String,
    status: Option<u16>,
    props: Vec<(u16, Node)>,
}

impl Response {
    fn prop(&self, name: &str) -> Option<&Node> {
        self.props
            .iter()
            .filter(|(status, _)| (200..300).contains(status))
            .find_map(|(_, prop)| prop.child(name))
    }

    fn text(&self, name: &str) -> Option<String> {
        self.prop(name)
            .map(|node| node.text.trim().to_owned())
            .filter(|text| !text.is_empty())
    }
}

fn status_code(text: &str) -> Option<u16> {
    text.split_whitespace().nth(1)?.parse().ok()
}

fn parse_multistatus(text: &str) -> Result<(Vec<Response>, Option<String>)> {
    let root = xml::parse(text)?;
    if root.name != "multistatus" {
        return Err(CalendarError::Parse);
    }
    let mut responses = Vec::new();
    for node in root.children_named("response") {
        if responses.len() >= MAX_RESOURCES {
            return Err(CalendarError::Limit);
        }
        let Some(href) = node.child_text("href") else {
            continue;
        };
        let props = node
            .children_named("propstat")
            .filter_map(|stat| {
                let code = stat
                    .child_text("status")
                    .and_then(status_code)
                    .unwrap_or(200);
                Some((code, stat.child("prop")?.clone()))
            })
            .collect();
        responses.push(Response {
            href: href.to_owned(),
            status: node.child_text("status").and_then(status_code),
            props,
        });
    }
    let token = root.child_text("sync-token").map(str::to_owned);
    Ok((responses, token))
}

fn resolve_href(base: &Url, href: &str) -> Result<Url> {
    let url = base.join(href).map_err(|_| CalendarError::Server)?;
    // A server must not steer credentials to another origin.
    if url.scheme() != base.scheme()
        || url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
    {
        return Err(CalendarError::Server);
    }
    Ok(url)
}

fn normalize_color(value: &str) -> Option<String> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.chars().all(|ch| ch.is_ascii_hexdigit()) || !matches!(hex.len(), 6 | 8) {
        return None;
    }
    Some(format!("#{}", hex[..6].to_ascii_uppercase()))
}

// -- client ------------------------------------------------------------------

/// A CalDAV session bound to credentials.
pub struct CaldavClient<'a> {
    http: &'a dyn HttpClient,
    credentials: Option<Credentials>,
}

impl<'a> CaldavClient<'a> {
    /// Creates a session; `credentials` are sent only to the origin of the
    /// address each call receives.
    pub fn new(http: &'a dyn HttpClient, credentials: Option<Credentials>) -> Self {
        Self { http, credentials }
    }

    async fn dav(
        &self,
        method: Method,
        url: &Url,
        depth: &str,
        body: String,
    ) -> Result<HttpResponse> {
        let request = HttpRequest::new(method, url.clone())
            .header("depth", depth)
            .header("accept", "application/xml, text/xml")
            .body("application/xml; charset=utf-8", body.into_bytes())
            .credentials(self.credentials.clone());
        send(self.http, request).await
    }

    async fn multistatus(
        &self,
        method: Method,
        url: &Url,
        depth: &str,
        body: String,
    ) -> Result<(Vec<Response>, Option<String>)> {
        let response = self.dav(method, url, depth, body).await?;
        if response.status != 207 {
            check_status(response.status)?;
            return Err(CalendarError::Unsupported);
        }
        parse_multistatus(response.text()?)
    }

    /// Finds the calendar home set and lists the calendars in it.
    pub async fn connect(&self, server: &Url) -> Result<(Url, Vec<RemoteCalendar>)> {
        check_url(server)?;
        let root_props = format!(
            "<d:propfind {NS}><d:prop><d:current-user-principal/><c:calendar-home-set/>\
             </d:prop></d:propfind>"
        );
        let (responses, _) = self
            .multistatus(Method::Propfind, server, "0", root_props)
            .await?;
        let first = responses.first();
        let home_of = |response: &Response| {
            response
                .prop("calendar-home-set")
                .and_then(|node| node.child_text("href"))
                .map(str::to_owned)
        };
        let mut home = first.and_then(home_of);
        if home.is_none() {
            let principal = first
                .and_then(|response| response.prop("current-user-principal"))
                .and_then(|node| node.child_text("href"))
                .map(str::to_owned);
            if let Some(principal) = principal {
                let principal = resolve_href(server, &principal)?;
                let (responses, _) = self
                    .multistatus(
                        Method::Propfind,
                        &principal,
                        "0",
                        format!(
                            "<d:propfind {NS}><d:prop><c:calendar-home-set/></d:prop></d:propfind>"
                        ),
                    )
                    .await?;
                home = responses.first().and_then(home_of);
            }
        }
        let home = match home {
            Some(href) => resolve_href(server, &href)?,
            // The address may already be a calendar home or one calendar.
            None => server.clone(),
        };
        let calendars = self.list_calendars(&home).await?;
        if calendars.is_empty() {
            return Err(CalendarError::Unsupported);
        }
        Ok((home, calendars))
    }

    /// Lists the VEVENT calendars directly inside `home`.
    pub async fn list_calendars(&self, home: &Url) -> Result<Vec<RemoteCalendar>> {
        let body = format!(
            "<d:propfind {NS}><d:prop><d:resourcetype/><d:displayname/><a:calendar-color/>\
             <d:current-user-privilege-set/><c:supported-calendar-component-set/>\
             </d:prop></d:propfind>"
        );
        let (responses, _) = self.multistatus(Method::Propfind, home, "1", body).await?;
        let mut out = Vec::new();
        for response in &responses {
            let is_calendar = response
                .prop("resourcetype")
                .is_some_and(|kind| kind.child("calendar").is_some());
            if !is_calendar {
                continue;
            }
            let supports_events = response
                .prop("supported-calendar-component-set")
                .is_none_or(|set| {
                    set.children_named("comp").any(|comp| {
                        comp.attr("name")
                            .is_some_and(|n| n.eq_ignore_ascii_case("VEVENT"))
                    })
                });
            if !supports_events {
                continue;
            }
            if out.len() >= MAX_CALENDARS {
                return Err(CalendarError::Limit);
            }
            let url = resolve_href(home, &response.href)?;
            let read_only = response
                .prop("current-user-privilege-set")
                .is_some_and(|set| {
                    !set.children_named("privilege").any(|privilege| {
                        privilege.children.iter().any(|kind| {
                            matches!(kind.name.as_str(), "write" | "write-content" | "all")
                        })
                    })
                });
            let name = response
                .text("displayname")
                .or_else(|| {
                    url.path_segments()?
                        .rfind(|segment| !segment.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "Calendar".to_owned())
                .chars()
                .take(200)
                .collect();
            out.push(RemoteCalendar {
                url,
                name,
                color: response
                    .text("calendar-color")
                    .and_then(|value| normalize_color(&value)),
                read_only,
            });
        }
        Ok(out)
    }

    /// Reads the collection's ctag and sync token.
    pub async fn collection_state(&self, calendar: &Url) -> Result<CollectionState> {
        let body =
            format!("<d:propfind {NS}><d:prop><cs:getctag/><d:sync-token/></d:prop></d:propfind>");
        let (responses, _) = self
            .multistatus(Method::Propfind, calendar, "0", body)
            .await?;
        Ok(responses
            .first()
            .map(|response| CollectionState {
                ctag: response.text("getctag"),
                sync_token: response.text("sync-token"),
            })
            .unwrap_or_default())
    }

    /// REPORT calendar-query: events overlapping `[from, to)`.
    pub async fn query_range(
        &self,
        calendar: &Url,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<RemoteResource>> {
        let body = format!(
            "<c:calendar-query {NS}><d:prop><d:getetag/><c:calendar-data/></d:prop>\
             <c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\">\
             <c:time-range start=\"{}\" end=\"{}\"/></c:comp-filter></c:comp-filter></c:filter>\
             </c:calendar-query>",
            from.format("%Y%m%dT%H%M%SZ"),
            to.format("%Y%m%dT%H%M%SZ")
        );
        let (responses, _) = self
            .multistatus(Method::Report, calendar, "1", body)
            .await?;
        let mut resources = self.resources(calendar, &responses)?.0;
        self.fill_missing(calendar, &mut resources).await?;
        Ok(resources)
    }

    /// For servers without sync tokens: lists the ETags of events overlapping
    /// `[from, to)` and fetches only the resources whose ETag is new or
    /// differs from `known` (address -> ETag). A resource without an ETag is
    /// always fetched.
    pub async fn query_range_delta(
        &self,
        calendar: &Url,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        known: &std::collections::HashMap<String, Option<String>>,
    ) -> Result<RangeDelta> {
        let body = format!(
            "<c:calendar-query {NS}><d:prop><d:getetag/></d:prop>             <c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\">             <c:time-range start=\"{}\" end=\"{}\"/></c:comp-filter></c:comp-filter></c:filter>             </c:calendar-query>",
            from.format("%Y%m%dT%H%M%SZ"),
            to.format("%Y%m%dT%H%M%SZ")
        );
        let (responses, _) = self
            .multistatus(Method::Report, calendar, "1", body)
            .await?;
        let listed = self.resources(calendar, &responses)?.0;
        let present = listed
            .iter()
            .map(|resource| resource.href.clone())
            .collect();
        let mut changed: Vec<RemoteResource> = listed
            .into_iter()
            .filter(|resource| {
                let unchanged = resource.etag.is_some()
                    && known
                        .get(resource.href.as_str())
                        .is_some_and(|etag| *etag == resource.etag);
                !unchanged
            })
            .collect();
        self.fill_missing(calendar, &mut changed).await?;
        Ok(RangeDelta { changed, present })
    }

    /// REPORT sync-collection since `token`. A refused token is
    /// [`CalendarError::Conflict`]; the caller falls back to a full read.
    pub async fn sync_changes(&self, calendar: &Url, token: &str) -> Result<SyncChanges> {
        let body = format!(
            "<d:sync-collection {NS}><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level>\
             <d:prop><d:getetag/><c:calendar-data/></d:prop></d:sync-collection>",
            xml::escape(token)
        );
        let response = self.dav(Method::Report, calendar, "1", body).await?;
        if matches!(response.status, 403 | 409 | 410 | 507) {
            return Err(CalendarError::Conflict);
        }
        if response.status != 207 {
            check_status(response.status)?;
            return Err(CalendarError::Unsupported);
        }
        let (responses, sync_token) = parse_multistatus(response.text()?)?;
        let (mut changed, removed) = self.resources(calendar, &responses)?;
        self.fill_missing(calendar, &mut changed).await?;
        Ok(SyncChanges {
            changed,
            removed,
            sync_token,
        })
    }

    fn resources(
        &self,
        base: &Url,
        responses: &[Response],
    ) -> Result<(Vec<RemoteResource>, Vec<Url>)> {
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        for response in responses {
            let href = resolve_href(base, &response.href)?;
            if response.status == Some(404) {
                removed.push(href);
                continue;
            }
            // The collection itself and non-calendar resources carry no data.
            if href.path().ends_with('/') && response.prop("calendar-data").is_none() {
                continue;
            }
            let data = response
                .prop("calendar-data")
                .filter(|node| !node.text.trim().is_empty());
            changed.push(RemoteResource {
                href,
                etag: response.text("getetag"),
                events: data
                    .map(|node| ical::parse_calendar(&node.text).unwrap_or_default())
                    .unwrap_or_default(),
                pending: data.is_none(),
                raw: data.map(|node| node.text.clone()),
            });
        }
        Ok((changed, removed))
    }

    /// Fetches resources the server listed without their data.
    async fn fill_missing(&self, calendar: &Url, resources: &mut [RemoteResource]) -> Result<()> {
        let missing: Vec<usize> = resources
            .iter()
            .enumerate()
            .filter(|(_, resource)| resource.pending)
            .map(|(index, _)| index)
            .collect();
        for batch in missing.chunks(MULTIGET_BATCH) {
            let hrefs: String = batch
                .iter()
                .map(|&index| {
                    format!(
                        "<d:href>{}</d:href>",
                        xml::escape(resources[index].href.path())
                    )
                })
                .collect();
            let body = format!(
                "<c:calendar-multiget {NS}><d:prop><d:getetag/><c:calendar-data/></d:prop>\
                 {hrefs}</c:calendar-multiget>"
            );
            let (responses, _) = self
                .multistatus(Method::Report, calendar, "1", body)
                .await?;
            for response in &responses {
                let Ok(href) = resolve_href(calendar, &response.href) else {
                    continue;
                };
                let Some(data) = response.prop("calendar-data") else {
                    continue;
                };
                if let Some(&index) = batch.iter().find(|&&i| resources[i].href == href) {
                    resources[index].pending = false;
                    resources[index].events = ical::parse_calendar(&data.text).unwrap_or_default();
                    resources[index].raw = Some(data.text.clone());
                    resources[index].etag =
                        response.text("getetag").or(resources[index].etag.take());
                }
            }
        }
        Ok(())
    }

    /// Creates (`existing: None`) or replaces a resource. Returns its address
    /// and the new ETag when the server sent one.
    pub async fn put_event(
        &self,
        calendar: &Url,
        uid: &str,
        ics: &str,
        existing: Option<(&Url, Option<&str>)>,
    ) -> Result<(Url, Option<String>)> {
        let (href, guard) = match existing {
            Some((href, Some(etag))) => (href.clone(), ("if-match", etag.to_owned())),
            Some((href, None)) => (href.clone(), ("if-match", "*".to_owned())),
            None => {
                let file: String = uid
                    .chars()
                    .map(|ch| {
                        if ch.is_ascii_alphanumeric() || ch == '-' {
                            ch
                        } else {
                            '_'
                        }
                    })
                    .take(100)
                    .collect();
                let href = calendar
                    .join(&format!("{file}.ics"))
                    .map_err(|_| CalendarError::InvalidInput)?;
                (href, ("if-none-match", "*".to_owned()))
            }
        };
        let request = HttpRequest::new(Method::Put, href.clone())
            .header(guard.0, guard.1)
            .body("text/calendar; charset=utf-8", ics.as_bytes().to_vec())
            .credentials(self.credentials.clone());
        let response = send(self.http, request).await?;
        check_status(response.status)?;
        Ok((href, response.header("etag").map(str::to_owned)))
    }

    /// Deletes a resource, guarded by its ETag when known.
    pub async fn delete_event(&self, href: &Url, etag: Option<&str>) -> Result<()> {
        let mut request =
            HttpRequest::new(Method::Delete, href.clone()).credentials(self.credentials.clone());
        if let Some(etag) = etag {
            request = request.header("if-match", etag);
        }
        let response = send(self.http, request).await?;
        match response.status {
            // Already gone is the outcome the caller wanted.
            404 | 410 => Ok(()),
            status => check_status(status),
        }
    }
}

/// Finds the CalDAV entry point from an email address or a URL, probing
/// `/.well-known/caldav` without credentials. Returns the address to log in
/// at.
pub async fn discover(http: &dyn HttpClient, input: &str) -> Result<Url> {
    let input = input.trim();
    let mut candidates: Vec<Url> = Vec::new();
    if input.contains('@') && !input.contains("://") {
        let domain = input.rsplit('@').next().unwrap_or_default();
        if domain.is_empty()
            || !domain
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        {
            return Err(CalendarError::InvalidInput);
        }
        let url = Url::parse(&format!("https://{domain}/.well-known/caldav"))
            .map_err(|_| CalendarError::InvalidInput)?;
        candidates.push(url);
    } else {
        let url = parse_address(input)?;
        if url.path() == "/" || url.path().is_empty() {
            let mut well_known = url.clone();
            well_known.set_path("/.well-known/caldav");
            candidates.push(well_known);
        }
        candidates.push(url);
    }
    let body =
        format!("<d:propfind {NS}><d:prop><d:current-user-principal/></d:prop></d:propfind>");
    let mut last = CalendarError::NotFound;
    for candidate in candidates {
        let request = HttpRequest::new(Method::Propfind, candidate)
            .header("depth", "0")
            .body("application/xml; charset=utf-8", body.clone().into_bytes());
        match send(http, request).await {
            Ok(response) if matches!(response.status, 200 | 207 | 401 | 403) => {
                return response.final_url.ok_or(CalendarError::Server);
            }
            Ok(_) => last = CalendarError::Unsupported,
            Err(error) => last = error,
        }
    }
    Err(last)
}
