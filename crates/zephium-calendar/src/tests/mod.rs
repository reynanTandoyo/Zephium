//! Fixture-driven tests. No test touches the network: every HTTP exchange is
//! answered by [`Fake`].

mod caldav;
mod ical;
mod oauth;
mod persist;
mod recur;
mod recurrence;
mod service;

use crate::error::{CalendarError, Result};
use crate::http::{HttpClient, HttpRequest, HttpResponse, Method};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Mutex;

pub(crate) fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid instant")
        .with_timezone(&Utc)
}

pub(crate) fn response(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.as_bytes().to_vec(),
        final_url: None,
    }
}

struct Route {
    method: Method,
    path: &'static str,
    body_contains: &'static str,
    response: HttpResponse,
}

/// Scripted HTTP server: first matching route answers, anything else is 404.
#[derive(Default)]
pub(crate) struct Fake {
    routes: Mutex<Vec<Route>>,
    pub(crate) seen: Mutex<Vec<HttpRequest>>,
}

impl Fake {
    pub(crate) fn route(
        &self,
        method: Method,
        path: &'static str,
        body_contains: &'static str,
        response: HttpResponse,
    ) {
        self.routes.lock().expect("routes").push(Route {
            method,
            path,
            body_contains,
            response,
        });
    }

    /// Drops every route so a test can script the next exchange.
    pub(crate) fn clear(&self) {
        self.routes.lock().expect("routes").clear();
    }

    pub(crate) fn requests(&self) -> Vec<HttpRequest> {
        self.seen.lock().expect("seen").clone()
    }
}

#[async_trait]
impl HttpClient for Fake {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse> {
        let body = String::from_utf8_lossy(&request.body).into_owned();
        let answer = self
            .routes
            .lock()
            .expect("routes")
            .iter()
            .find(|route| {
                route.method == request.method
                    && (route.path == "*" || route.path == request.url.path())
                    && body.contains(route.body_contains)
            })
            .map(|route| route.response.clone());
        self.seen.lock().expect("seen").push(request);
        Ok(answer.unwrap_or_else(|| response(404, &[], "")))
    }
}

#[test]
fn errors_are_a_closed_set_without_server_text() {
    // Display strings are fixed; none can carry server or credential text.
    assert_eq!(CalendarError::Unauthorized.to_string(), "not signed in");
    assert_eq!(CalendarError::Conflict.to_string(), "conflict");
}
