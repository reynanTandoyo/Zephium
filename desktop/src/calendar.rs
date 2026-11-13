//! Fork-owned (T-0037/T-0038/T-0039): IPC for calendars. Accounts and
//! calendars live in `calendar.json` in the profile data directory, events in
//! memory; passwords only in the OS credential vault
//! (`app.zephium.calendar|<account id>`) and never cross IPC outward. All
//! domain logic is in `zephium-calendar`.
use super::*;
use std::sync::Arc;
use zephium_calendar::api::{
    parse_instant, CalendarAccountV1, CalendarDiscoveryV1, CalendarEventDraftV1,
    CalendarEventRefV1, CalendarEventsV1, CalendarSyncReportV1, CalendarV1,
};
use zephium_calendar::http::{ReqwestClient, Secret};
use zephium_calendar::store::{JsonFileStorage, VaultSecrets};
use zephium_calendar::{CalendarError, CalendarService};

/// How often due calendars are refreshed in the background.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Calendars, accounts or events changed; read them again.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "zephium:calendar-changed")]
pub(crate) struct CalendarChanged {
    pub(crate) version: u32,
}

struct CalendarState(Arc<CalendarService>);

fn changed(app: &tauri::AppHandle) {
    emit_to_privileged(
        app,
        MAIN_LABEL,
        "zephium:calendar-changed",
        &CalendarChanged { version: 1 },
    );
}

/// Builds the service over the profile directory and starts the periodic
/// refresh. A failure leaves every `calendar_*` command answering
/// `unsupported` rather than blocking startup.
pub(crate) fn install(app: &tauri::AppHandle, data_dir: &std::path::Path) {
    let built = ReqwestClient::new().and_then(|http| {
        CalendarService::new(
            Arc::new(http),
            Arc::new(JsonFileStorage::new(data_dir.join("calendar.json"))),
            Arc::new(VaultSecrets),
        )
    });
    let service = match built {
        Ok(service) => Arc::new(service),
        Err(error) => {
            write_diagnostic(format_args!("calendar: unavailable: {error}"));
            return;
        }
    };
    app.manage(CalendarState(service.clone()));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        loop {
            if !service.sync_due().await.outcomes.is_empty() {
                changed(&app);
            }
            tokio::time::sleep(REFRESH_EVERY).await;
        }
    });
}

fn service(
    app: &tauri::AppHandle,
    caller: &WebviewWindow,
    command: &str,
) -> Result<Arc<CalendarService>, CalendarError> {
    if !authorize(caller, CallerPolicy::Main, command) {
        return Err(CalendarError::Forbidden);
    }
    app.try_state::<CalendarState>()
        .map(|state| state.0.clone())
        .ok_or(CalendarError::Unsupported)
}

/// Finds the CalDAV login address for an email address or URL.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_discover(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    email_or_url: String,
) -> Result<CalendarDiscoveryV1, CalendarError> {
    service(&app, &caller, "calendar_discover")?
        .discover(&email_or_url)
        .await
}

/// Signs in to a CalDAV server and records the account; calendars load in the
/// background and `zephium:calendar-changed` follows.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_add_account(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    server_url: String,
    username: String,
    password: String,
    name: Option<String>,
) -> Result<CalendarAccountV1, CalendarError> {
    let service = service(&app, &caller, "calendar_add_account")?;
    let account = service
        .add_account(&server_url, &username, Secret::new(password), name.as_deref())
        .await?;
    changed(&app);
    tauri::async_runtime::spawn(async move {
        service.sync_all().await;
        changed(&app);
    });
    Ok(account)
}

/// Adds a read-only calendar from an `https://` or `webcal://` address.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_add_subscription(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    url: String,
    name: String,
) -> Result<CalendarAccountV1, CalendarError> {
    let account = service(&app, &caller, "calendar_add_subscription")?
        .add_subscription(&url, &name)
        .await?;
    changed(&app);
    Ok(account)
}

/// Accounts and subscriptions (no secrets).
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_accounts(
    caller: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Vec<CalendarAccountV1>, CalendarError> {
    Ok(service(&app, &caller, "calendar_accounts")?.accounts())
}

/// Removes an account with its calendars, cached events and saved password.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_remove_account(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    account: String,
) -> Result<(), CalendarError> {
    service(&app, &caller, "calendar_remove_account")?.remove_account(&account)?;
    changed(&app);
    Ok(())
}

/// All calendars with colour, read-only and visibility flags.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_calendars(
    caller: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Vec<CalendarV1>, CalendarError> {
    Ok(service(&app, &caller, "calendar_calendars")?.calendars())
}

/// Shows or hides a calendar.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_set_visible(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    calendar: String,
    visible: bool,
) -> Result<(), CalendarError> {
    service(&app, &caller, "calendar_set_visible")?.set_visible(&calendar, visible)?;
    changed(&app);
    Ok(())
}

/// Occurrences overlapping `[from, to)` (RFC 3339 instants) in visible
/// calendars, bounded in count and range.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_events(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    from: String,
    to: String,
) -> Result<CalendarEventsV1, CalendarError> {
    service(&app, &caller, "calendar_events")?.events(parse_instant(&from)?, parse_instant(&to)?)
}

/// Creates an event in a writable calendar.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_create_event(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    calendar: String,
    draft: CalendarEventDraftV1,
) -> Result<CalendarEventRefV1, CalendarError> {
    let created = service(&app, &caller, "calendar_create_event")?
        .create_event(&calendar, &draft)
        .await?;
    changed(&app);
    Ok(created)
}

/// Changes an event. A concurrent server change answers `conflict`; run
/// `calendar_sync` and retry.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_update_event(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    calendar: String,
    uid: String,
    draft: CalendarEventDraftV1,
) -> Result<CalendarEventRefV1, CalendarError> {
    let updated = service(&app, &caller, "calendar_update_event")?
        .update_event(&calendar, &uid, &draft)
        .await?;
    changed(&app);
    Ok(updated)
}

/// Deletes an event (the whole series when it repeats).
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_delete_event(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    calendar: String,
    uid: String,
) -> Result<(), CalendarError> {
    service(&app, &caller, "calendar_delete_event")?
        .delete_event(&calendar, &uid)
        .await?;
    changed(&app);
    Ok(())
}

/// Refreshes every calendar now.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_sync(
    caller: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<CalendarSyncReportV1, CalendarError> {
    let report = service(&app, &caller, "calendar_sync")?.sync_all().await;
    changed(&app);
    Ok(report)
}
