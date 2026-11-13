use super::{response, utc, Fake};
use crate::caldav::{discover, CaldavClient};
use crate::error::CalendarError;
use crate::http::{send, Credentials, HttpRequest, Method, Secret};
use crate::xml;
use url::Url;

pub(crate) const HEADER: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
    <d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" \
    xmlns:a=\"http://apple.com/ns/ical/\" xmlns:cs=\"http://calendarserver.org/ns/\">";

pub(crate) fn multistatus(body: &str) -> String {
    format!("{HEADER}{body}</d:multistatus>")
}

pub(crate) const ROOT: &str = "<d:response><d:href>/dav/</d:href><d:propstat><d:prop>\
    <d:current-user-principal><d:href>/dav/principals/ann/</d:href></d:current-user-principal>\
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>";

pub(crate) const PRINCIPAL: &str =
    "<d:response><d:href>/dav/principals/ann/</d:href><d:propstat><d:prop>\
    <c:calendar-home-set><d:href>/dav/calendars/ann/</d:href></c:calendar-home-set>\
    </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>";

pub(crate) const HOME: &str = "\
<d:response><d:href>/dav/calendars/ann/</d:href><d:propstat><d:prop>\
  <d:resourcetype><d:collection/></d:resourcetype></d:prop>\
  <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
<d:response><d:href>/dav/calendars/ann/work/</d:href><d:propstat><d:prop>\
  <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>\
  <d:displayname>Work &amp; Co</d:displayname><a:calendar-color>#ff8800ff</a:calendar-color>\
  <d:current-user-privilege-set><d:privilege><d:read/></d:privilege>\
  <d:privilege><d:write-content/></d:privilege></d:current-user-privilege-set>\
  <c:supported-calendar-component-set><c:comp name=\"VEVENT\"/><c:comp name=\"VTODO\"/>\
  </c:supported-calendar-component-set></d:prop>\
  <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
<d:response><d:href>/dav/calendars/ann/holidays/</d:href><d:propstat><d:prop>\
  <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>\
  <d:displayname>Holidays</d:displayname>\
  <d:current-user-privilege-set><d:privilege><d:read/></d:privilege></d:current-user-privilege-set>\
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
<d:response><d:href>/dav/calendars/ann/tasks/</d:href><d:propstat><d:prop>\
  <d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname>\
  <c:supported-calendar-component-set><c:comp name=\"VTODO\"/></c:supported-calendar-component-set>\
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>";

pub(crate) fn script_account(fake: &Fake) {
    fake.route(
        Method::Propfind,
        "/dav/",
        "",
        response(207, &[], &multistatus(ROOT)),
    );
    fake.route(
        Method::Propfind,
        "/dav/principals/ann/",
        "",
        response(207, &[], &multistatus(PRINCIPAL)),
    );
    fake.route(
        Method::Propfind,
        "/dav/calendars/ann/",
        "",
        response(207, &[], &multistatus(HOME)),
    );
}

pub(crate) fn credentials() -> Option<Credentials> {
    Some(Credentials {
        username: "ann@example.com".to_owned(),
        password: Secret::new("hunter2".to_owned()),
    })
}

#[tokio::test]
async fn discovery_follows_well_known_and_accepts_a_login_challenge() {
    let fake = Fake::default();
    fake.route(
        Method::Propfind,
        "/.well-known/caldav",
        "",
        response(301, &[("location", "/dav/")], ""),
    );
    fake.route(
        Method::Propfind,
        "/dav/",
        "",
        response(401, &[("www-authenticate", "Basic realm=\"x\"")], ""),
    );
    let url = discover(&fake, "ann@example.com")
        .await
        .expect("discovered");
    assert_eq!(url.as_str(), "https://example.com/dav/");
    // Discovery never sends credentials.
    assert!(fake.requests().iter().all(|r| r.credentials.is_none()));
}

#[tokio::test]
async fn discovery_from_a_root_url_falls_back_to_the_address_itself() {
    let fake = Fake::default();
    fake.route(
        Method::Propfind,
        "/",
        "",
        response(207, &[], &multistatus(ROOT)),
    );
    let url = discover(&fake, "https://cal.example.com")
        .await
        .expect("found");
    assert_eq!(url.as_str(), "https://cal.example.com/");
    let nothing = Fake::default();
    assert_eq!(
        discover(&nothing, "ann@example.com").await,
        Err(CalendarError::Unsupported)
    );
}

#[tokio::test]
async fn discovery_rejects_bad_and_insecure_input() {
    let fake = Fake::default();
    assert_eq!(
        discover(&fake, "ann@exa mple.com/evil").await,
        Err(CalendarError::InvalidInput)
    );
    assert_eq!(
        discover(&fake, "http://cal.example.com/").await,
        Err(CalendarError::Insecure)
    );
    assert_eq!(
        discover(&fake, "ftp://cal.example.com/").await,
        Err(CalendarError::InvalidInput)
    );
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn connect_lists_calendars_with_colour_and_write_access() {
    let fake = Fake::default();
    script_account(&fake);
    let client = CaldavClient::new(&fake, credentials());
    let server = Url::parse("https://cal.example.com/dav/").expect("url");
    let (home, calendars) = client.connect(&server).await.expect("connect");
    assert_eq!(home.path(), "/dav/calendars/ann/");
    // The folder, and the VTODO-only list, are not event calendars.
    assert_eq!(calendars.len(), 2);
    assert_eq!(calendars[0].name, "Work & Co");
    assert_eq!(calendars[0].color.as_deref(), Some("#FF8800"));
    assert!(!calendars[0].read_only);
    assert_eq!(calendars[1].name, "Holidays");
    assert!(calendars[1].read_only);
    assert!(fake.requests().iter().all(|r| r
        .credentials
        .as_ref()
        .is_some_and(|c| c.username == "ann@example.com")));
}

#[tokio::test]
async fn connect_maps_login_failures_and_non_caldav_servers() {
    let fake = Fake::default();
    fake.route(
        Method::Propfind,
        "/dav/",
        "",
        response(401, &[], "<html>secret text</html>"),
    );
    let client = CaldavClient::new(&fake, credentials());
    let server = Url::parse("https://cal.example.com/dav/").expect("url");
    assert_eq!(
        client.connect(&server).await.err(),
        Some(CalendarError::Unauthorized)
    );

    let web = Fake::default();
    web.route(Method::Propfind, "/dav/", "", response(200, &[], "<html/>"));
    let client = CaldavClient::new(&web, credentials());
    assert_eq!(
        client.connect(&server).await.err(),
        Some(CalendarError::Unsupported)
    );

    let junk = Fake::default();
    junk.route(
        Method::Propfind,
        "/dav/",
        "",
        response(207, &[], "<not-xml"),
    );
    let client = CaldavClient::new(&junk, credentials());
    assert_eq!(
        client.connect(&server).await.err(),
        Some(CalendarError::Parse)
    );
}

const EVENT_ONE: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:one@x\nSUMMARY:One\n\
DTSTART;TZID=Europe/Berlin:20260105T090000\nDTEND;TZID=Europe/Berlin:20260105T100000\nEND:VEVENT\nEND:VCALENDAR\n";
const EVENT_TWO: &str = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:two@x\nSUMMARY:Two\n\
DTSTART;VALUE=DATE:20260106\nEND:VEVENT\nEND:VCALENDAR\n";

fn report_body() -> String {
    multistatus(&format!(
        "<d:response><d:href>/dav/calendars/ann/work/one.ics</d:href><d:propstat><d:prop>\
         <d:getetag>\"e1\"</d:getetag><c:calendar-data><![CDATA[{EVENT_ONE}]]></c:calendar-data>\
         </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
         <d:response><d:href>/dav/calendars/ann/work/two.ics</d:href><d:propstat><d:prop>\
         <d:getetag>\"e2\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>\
         </d:response>"
    ))
}

#[tokio::test]
async fn calendar_query_reads_etags_and_fetches_missing_data_by_multiget() {
    let fake = Fake::default();
    fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "calendar-multiget",
        response(
            207,
            &[],
            &multistatus(&format!(
                "<d:response><d:href>/dav/calendars/ann/work/two.ics</d:href><d:propstat><d:prop>\
                 <d:getetag>\"e2\"</d:getetag><c:calendar-data>{}</c:calendar-data></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
                xml::escape(EVENT_TWO)
            )),
        ),
    );
    fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "calendar-query",
        response(207, &[], &report_body()),
    );
    let client = CaldavClient::new(&fake, credentials());
    let calendar = Url::parse("https://cal.example.com/dav/calendars/ann/work/").expect("url");
    let resources = client
        .query_range(
            &calendar,
            utc("2026-01-01T00:00:00Z"),
            utc("2026-02-01T00:00:00Z"),
        )
        .await
        .expect("query");
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].etag.as_deref(), Some("\"e1\""));
    assert_eq!(resources[0].events[0].summary, "One");
    assert_eq!(resources[1].events[0].summary, "Two");
    assert!(resources.iter().all(|r| !r.pending));
    let query = fake
        .requests()
        .into_iter()
        .find(|r| String::from_utf8_lossy(&r.body).contains("calendar-query"))
        .expect("query request");
    let body = String::from_utf8_lossy(&query.body).into_owned();
    assert!(
        body.contains("start=\"20260101T000000Z\"") && body.contains("end=\"20260201T000000Z\"")
    );
}

#[tokio::test]
async fn resources_outside_the_server_origin_are_refused() {
    let fake = Fake::default();
    fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "calendar-query",
        response(
            207,
            &[],
            &multistatus(
                "<d:response><d:href>https://evil.example.net/x.ics</d:href><d:propstat><d:prop>\
                 <d:getetag>\"e\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status>\
                 </d:propstat></d:response>",
            ),
        ),
    );
    let client = CaldavClient::new(&fake, credentials());
    let calendar = Url::parse("https://cal.example.com/dav/calendars/ann/work/").expect("url");
    let result = client
        .query_range(
            &calendar,
            utc("2026-01-01T00:00:00Z"),
            utc("2026-02-01T00:00:00Z"),
        )
        .await;
    assert_eq!(result.err(), Some(CalendarError::Server));
}

#[tokio::test]
async fn sync_collection_reports_changes_removals_and_a_new_token() {
    let fake = Fake::default();
    fake.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "sync-collection",
        response(
            207,
            &[],
            &format!(
                "{HEADER}<d:response><d:href>/dav/calendars/ann/work/one.ics</d:href><d:propstat><d:prop>\
                 <d:getetag>\"e9\"</d:getetag><c:calendar-data>{}</c:calendar-data></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
                 <d:response><d:href>/dav/calendars/ann/work/gone.ics</d:href>\
                 <d:status>HTTP/1.1 404 Not Found</d:status></d:response>\
                 <d:sync-token>http://x/sync/42</d:sync-token></d:multistatus>",
                xml::escape(EVENT_ONE)
            ),
        ),
    );
    let client = CaldavClient::new(&fake, credentials());
    let calendar = Url::parse("https://cal.example.com/dav/calendars/ann/work/").expect("url");
    let changes = client
        .sync_changes(&calendar, "http://x/sync/41")
        .await
        .expect("sync");
    assert_eq!(changes.changed.len(), 1);
    assert_eq!(changes.removed.len(), 1);
    assert!(changes.removed[0].path().ends_with("gone.ics"));
    assert_eq!(changes.sync_token.as_deref(), Some("http://x/sync/42"));
    let sent = String::from_utf8_lossy(&fake.requests()[0].body).into_owned();
    assert!(sent.contains("http://x/sync/41"));

    let refused = Fake::default();
    refused.route(
        Method::Report,
        "/dav/calendars/ann/work/",
        "",
        response(403, &[], ""),
    );
    let client = CaldavClient::new(&refused, credentials());
    assert_eq!(
        client.sync_changes(&calendar, "old").await.err(),
        Some(CalendarError::Conflict)
    );
}

#[tokio::test]
async fn collection_state_reads_ctag_and_token() {
    let fake = Fake::default();
    fake.route(
        Method::Propfind,
        "/dav/calendars/ann/work/",
        "",
        response(
            207,
            &[],
            &multistatus(
                "<d:response><d:href>/dav/calendars/ann/work/</d:href><d:propstat><d:prop>\
                 <cs:getctag>ctag-7</cs:getctag><d:sync-token>tok-7</d:sync-token></d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
            ),
        ),
    );
    let client = CaldavClient::new(&fake, credentials());
    let calendar = Url::parse("https://cal.example.com/dav/calendars/ann/work/").expect("url");
    let state = client.collection_state(&calendar).await.expect("state");
    assert_eq!(state.ctag.as_deref(), Some("ctag-7"));
    assert_eq!(state.sync_token.as_deref(), Some("tok-7"));
}

#[tokio::test]
async fn put_is_guarded_by_etag_and_delete_tolerates_missing() {
    let fake = Fake::default();
    fake.route(
        Method::Put,
        "/dav/calendars/ann/work/one.ics",
        "",
        response(412, &[], "server secret"),
    );
    fake.route(
        Method::Put,
        "*",
        "",
        response(201, &[("etag", "\"n1\"")], ""),
    );
    let client = CaldavClient::new(&fake, credentials());
    let calendar = Url::parse("https://cal.example.com/dav/calendars/ann/work/").expect("url");
    let (href, etag) = client
        .put_event(
            &calendar,
            "new@x",
            "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
            None,
        )
        .await
        .expect("created");
    assert!(href.path().ends_with(".ics"));
    assert_eq!(etag.as_deref(), Some("\"n1\""));
    let create = &fake.requests()[0];
    assert!(create.headers.contains(&("if-none-match", "*".to_owned())));
    assert!(create
        .headers
        .contains(&("content-type", "text/calendar; charset=utf-8".to_owned())));

    let existing =
        Url::parse("https://cal.example.com/dav/calendars/ann/work/one.ics").expect("url");
    let conflict = client
        .put_event(&calendar, "one@x", "x", Some((&existing, Some("\"e1\""))))
        .await;
    assert_eq!(conflict.err(), Some(CalendarError::Conflict));
    assert!(fake.requests()[1]
        .headers
        .contains(&("if-match", "\"e1\"".to_owned())));

    assert_eq!(client.delete_event(&existing, Some("\"e1\"")).await, Ok(()));
    assert!(fake.requests()[2]
        .headers
        .contains(&("if-match", "\"e1\"".to_owned())));
}

#[tokio::test]
async fn redirects_never_carry_credentials_to_another_origin() {
    let fake = Fake::default();
    fake.route(
        Method::Propfind,
        "/start",
        "",
        response(
            302,
            &[("location", "https://other.example.net/landing")],
            "",
        ),
    );
    fake.route(
        Method::Propfind,
        "/landing",
        "",
        response(207, &[], &multistatus("")),
    );
    let url = Url::parse("https://cal.example.com/start").expect("url");
    let reply = send(
        &fake,
        HttpRequest::new(Method::Propfind, url).credentials(credentials()),
    )
    .await
    .expect("sent");
    assert_eq!(reply.status, 207);
    let seen = fake.requests();
    assert!(seen[0].credentials.is_some());
    assert!(seen[1].credentials.is_none());

    // A redirect down to plain HTTP is refused.
    let downgrade = Fake::default();
    downgrade.route(
        Method::Get,
        "/a",
        "",
        response(301, &[("location", "http://cal.example.com/b")], ""),
    );
    let url = Url::parse("https://cal.example.com/a").expect("url");
    let result = send(&downgrade, HttpRequest::new(Method::Get, url)).await;
    assert_eq!(result.err(), Some(CalendarError::Insecure));
}

#[test]
fn secrets_do_not_appear_in_debug_output() {
    let text = format!("{:?}", credentials());
    assert!(!text.contains("hunter2"));
}
