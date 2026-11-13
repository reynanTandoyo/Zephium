//! Fork-owned (T-0037/T-0038/T-0039/T-0041): IPC for calendars. Accounts,
//! calendars, refresh state and cached events live in the application store
//! (`zephium-store`, META migration v30) and survive restarts; an old
//! `calendar.json` in the profile data directory is imported once and renamed
//! to `calendar.json.migrated`. Passwords live only in the OS credential vault
//! (`app.zephium.calendar|<account id>`) and never cross IPC outward. All
//! domain logic is in `zephium-calendar`.
use super::*;
use std::sync::Arc;
use zephium_calendar::api::{
    parse_instant, CalendarAccountV1, CalendarDiscoveryV1, CalendarEventDraftV1,
    CalendarEventRefV1, CalendarEventsV1, CalendarSyncReportV1, CalendarV1,
};
use zephium_calendar::http::{ReqwestClient, Secret};
use zephium_calendar::persist::StoreCalendar;
use zephium_calendar::store::VaultSecrets;
use zephium_calendar::{CalendarError, CalendarService};

/// How often the background task looks for calendars that are due. A CalDAV
/// calendar is due once its last refresh is 15 minutes old (see
/// `zephium-calendar`), so each refreshes every 15 to 20 minutes.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// How often the background task checks for shutdown while waiting.
const SHUTDOWN_POLL: std::time::Duration = std::time::Duration::from_secs(2);

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
    let Some(store) = APP_STORE.get().cloned() else {
        write_diagnostic(format_args!("calendar: unavailable: no application store"));
        return;
    };
    let persistent = Arc::new(StoreCalendar::new(
        store,
        Some(data_dir.join("calendar.json")),
    ));
    let built = ReqwestClient::new().and_then(|http| {
        CalendarService::with_cache(
            Arc::new(http),
            persistent.clone(),
            Arc::new(VaultSecrets),
            persistent,
            zephium_calendar::service::system_clock(),
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
        // Read the saved events off the setup path.
        let warm = service.clone();
        let _ = tokio::task::spawn_blocking(move || warm.warm()).await;
        if wait_or_shutdown(&app, std::time::Duration::from_secs(8)).await {
            return;
        }
        loop {
            let report = service.sync_due().await;
            if shutdown_started(&app) {
                return;
            }
            if !report.outcomes.is_empty() {
                changed(&app);
            }
            if wait_or_shutdown(&app, REFRESH_EVERY).await {
                return;
            }
        }
    });
}

/// Sleeps for `duration`; `true` when shutdown began meanwhile.
async fn wait_or_shutdown(app: &tauri::AppHandle, duration: std::time::Duration) -> bool {
    let mut waited = std::time::Duration::ZERO;
    while waited < duration {
        if shutdown_started(app) {
            return true;
        }
        let step = SHUTDOWN_POLL.min(duration - waited);
        tokio::time::sleep(step).await;
        waited += step;
    }
    shutdown_started(app)
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
        .add_account(
            &server_url,
            &username,
            Secret::new(password),
            name.as_deref(),
        )
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

/// Changes one occurrence of a recurring event (RECURRENCE-ID override).
/// `recurrence_id` is the occurrence's `recurrence_id` as
/// `calendar_events` lists it; the draft's `rrule` is ignored.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_update_occurrence(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    calendar: String,
    uid: String,
    recurrence_id: String,
    draft: CalendarEventDraftV1,
) -> Result<CalendarEventRefV1, CalendarError> {
    let updated = service(&app, &caller, "calendar_update_occurrence")?
        .update_occurrence(&calendar, &uid, &recurrence_id, &draft)
        .await?;
    changed(&app);
    Ok(updated)
}

/// Deletes one occurrence of a recurring event (EXDATE on the series).
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_delete_occurrence(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    calendar: String,
    uid: String,
    recurrence_id: String,
) -> Result<(), CalendarError> {
    service(&app, &caller, "calendar_delete_occurrence")?
        .delete_occurrence(&calendar, &uid, &recurrence_id)
        .await?;
    changed(&app);
    Ok(())
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
