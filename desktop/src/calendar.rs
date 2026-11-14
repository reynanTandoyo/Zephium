//! Fork-owned (T-0037/T-0038/T-0039/T-0041): IPC for calendars. Accounts,
//! calendars, refresh state and cached events live in the application store
//! (`zephium-store`, META migration v30) and survive restarts; an old
//! `calendar.json` in the profile data directory is imported once and renamed
//! to `calendar.json.migrated`. Passwords live only in the OS credential vault
//! (`app.zephium.calendar|<account id>`) and never cross IPC outward. All
//! domain logic is in `zephium-calendar`. Reminders (T-0044) are shown from a
//! background scheduler here; their ledger is one app setting.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use zephium_calendar::api::{
    parse_instant, CalendarAccountV1, CalendarDiscoveryV1, CalendarEventDraftV1,
    CalendarEventRefV1, CalendarEventsV1, CalendarSyncReportV1, CalendarV1,
};
use zephium_calendar::chrono::{DateTime, Utc};
use zephium_calendar::http::{ReqwestClient, Secret};
use zephium_calendar::persist::StoreCalendar;
use zephium_calendar::reminders::{self, Ledger, Plan, Reminder, DEFAULT_CHOICES};
use zephium_calendar::store::VaultSecrets;
use zephium_calendar::{CalendarError, CalendarService};
use zephium_core::ports::store::Store;

/// How often the background task looks for calendars that are due. A CalDAV
/// calendar is due once its last refresh is 15 minutes old (see
/// `zephium-calendar`), so each refreshes every 15 to 20 minutes.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// How often the background task checks for shutdown while waiting.
const SHUTDOWN_POLL: std::time::Duration = std::time::Duration::from_secs(2);
/// Settings: `calendar.reminders` ("false" turns reminders off) and
/// `calendar.default_reminder` (minutes, for timed events without an alarm).
const REMINDERS_SETTING: &str = "calendar.reminders";
const DEFAULT_REMINDER_SETTING: &str = "calendar.default_reminder";
/// The reminder ledger: when the last pass ran and which reminders were shown.
const LEDGER_SETTING: &str = "calendar.reminder-ledger";
/// Longest location a reminder toast shows after its start time.
const LOCATION_MAX: usize = 60;
/// The first reminder pass waits for the saved events to be read.
const REMINDER_START_DELAY: std::time::Duration = std::time::Duration::from_secs(8);
/// Bumped whenever calendars or events change, so the reminder scheduler
/// plans again instead of sleeping on an older plan.
static REPLAN: AtomicU64 = AtomicU64::new(0);

/// Calendars, accounts or events changed; read them again.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[tauri_specta(event_name = "zephium:calendar-changed")]
pub(crate) struct CalendarChanged {
    pub(crate) version: u32,
}

struct CalendarState(Arc<CalendarService>);

fn changed(app: &tauri::AppHandle) {
    REPLAN.fetch_add(1, Ordering::AcqRel);
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
        .map(|service| service.with_oauth(crate::oauth::token_manager()))
    });
    let service = match built {
        Ok(service) => Arc::new(service),
        Err(error) => {
            write_diagnostic(format_args!("calendar: unavailable: {error}"));
            return;
        }
    };
    app.manage(CalendarState(service.clone()));
    spawn_reminders(app.clone(), service.clone());
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

/// Runs the reminder scheduler: a pass now and then one at each next
/// reminder, at least once a minute, and at once when calendars change.
fn spawn_reminders(app: tauri::AppHandle, service: Arc<CalendarService>) {
    tauri::async_runtime::spawn(async move {
        let mut ledger = Ledger::load(app_setting(LEDGER_SETTING).as_deref());
        let seen = REPLAN.load(Ordering::Acquire);
        if wait_for_replan(&app, REMINDER_START_DELAY, seen).await {
            return;
        }
        loop {
            let seen = REPLAN.load(Ordering::Acquire);
            let now = Utc::now();
            let plan = reminder_pass(&service, &mut ledger, now);
            if shutdown_started(&app) {
                return;
            }
            let wait = Ledger::wait_after(&plan, now);
            if wait_for_replan(&app, wait, seen).await {
                return;
            }
        }
    });
}

/// One scheduler pass at `now`: reads the two settings, finds the reminders,
/// shows the due ones and saves the ledger when it is due. The result says
/// what was shown and when to look again.
fn reminder_pass(service: &CalendarService, ledger: &mut Ledger, now: DateTime<Utc>) -> Plan {
    let enabled = app_setting(REMINDERS_SETTING).as_deref() != Some("false");
    let default = app_setting(DEFAULT_REMINDER_SETTING)
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|minutes| DEFAULT_CHOICES.contains(minutes));
    let found = if enabled {
        service.reminders(now, default)
    } else {
        Vec::new()
    };
    let plan = ledger.pass(now, &found);
    if let Some(saved) = ledger.save_if_due(now, &plan) {
        if let Some(store) = crate::APP_STORE.get() {
            store.set_app_setting(LEDGER_SETTING.into(), saved);
        }
    }
    if enabled {
        for reminder in &plan.due {
            show_reminder(reminder, now);
        }
        if plan.missed > 0 {
            let title = crate::menu_text::message("calendar_reminders_title", "Calendar reminders");
            let body = crate::menu_text::message(
                "calendar_reminders_missed",
                "{count} calendar reminders were missed.",
            )
            .replace("{count}", &plan.missed.to_string());
            crate::focus_alerts::calendar_reminder("missed", &title, &body);
        }
    }
    plan
}

/// The toast for one reminder: the event's title, then its start time and,
/// when short, its location. Nothing else of the event is shown.
fn show_reminder(reminder: &Reminder, now: DateTime<Utc>) {
    let title = if reminder.title.trim().is_empty() {
        crate::menu_text::message("calendar_reminder_untitled", "Untitled event")
    } else {
        reminder.title.clone()
    };
    let when = if reminder.all_day {
        crate::menu_text::message("calendar_reminder_all_day", "All day")
    } else {
        reminders::start_label(reminder.start, now)
    };
    let location = reminder.location.trim();
    let body = if !location.is_empty() && location.chars().count() <= LOCATION_MAX {
        format!("{when}, {location}")
    } else {
        when
    };
    crate::focus_alerts::calendar_reminder(&reminder.key, &title, &body);
}

/// Sleeps up to `wait`, returning early when calendars changed since `seen`.
/// `true` when shutdown began meanwhile.
async fn wait_for_replan(app: &tauri::AppHandle, wait: std::time::Duration, seen: u64) -> bool {
    let mut waited = std::time::Duration::ZERO;
    while waited < wait {
        if shutdown_started(app) {
            return true;
        }
        if REPLAN.load(Ordering::Acquire) != seen {
            return false;
        }
        let step = SHUTDOWN_POLL.min(wait - waited);
        tokio::time::sleep(step).await;
        waited += step;
    }
    shutdown_started(app)
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

/// Adds a Google Calendar or Microsoft 365 account from a finished
/// `oauth_sign_in` (the grant handle it returned; the sign-in must have asked
/// for the calendar purpose). The handle is consumed: the refresh token goes
/// to the OS vault, the calendars are listed from the provider, events load
/// in the background and `zephium:calendar-changed` follows. A failure leaves
/// nothing behind; sign in again to retry.
#[tauri::command]
#[specta::specta]
pub(crate) async fn calendar_add_oauth_account(
    caller: WebviewWindow,
    app: tauri::AppHandle,
    grant: String,
    name: Option<String>,
) -> Result<CalendarAccountV1, CalendarError> {
    let service = service(&app, &caller, "calendar_add_oauth_account")?;
    let grant = crate::oauth::take_grant(&grant).ok_or(CalendarError::InvalidInput)?;
    let account = service.add_oauth_account(&grant, name.as_deref()).await?;
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
    service(&app, &caller, "calendar_remove_account")?
        .remove_account_and_session(&account)
        .await?;
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
