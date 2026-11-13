//! Fork-owned (T-0008, `docs/fork-cli-models.md`): the AI apps on this
//! computer (Claude Code, Antigravity, OpenCode) as model providers next to
//! the keyed ones. The apps are found and asked for their sign-in only after
//! the composition root enables it, so tests and other hosts never run them.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use zephium_cli_models::CliModelClient;
pub use zephium_cli_models::{CliApp, CliAppState, CliAppStatus};
use zephium_core::work::model::{
    WorkModelClient, WorkModelEntry, WorkModelError, WorkModelProvider, WorkModelRole,
};

/// The app providers, in the order runs fall back to them.
pub const APP_PROVIDERS: [WorkModelProvider; 3] = [
    WorkModelProvider::ClaudeCode,
    WorkModelProvider::Antigravity,
    WorkModelProvider::OpenCode,
];

#[derive(Default)]
struct Apps {
    enabled: bool,
    scanned: bool,
    statuses: Vec<CliAppStatus>,
}

fn apps() -> MutexGuard<'static, Apps> {
    static APPS: OnceLock<Mutex<Apps>> = OnceLock::new();
    APPS.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn scans() -> &'static tokio::sync::Mutex<()> {
    static SCANS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    SCANS.get_or_init(Default::default)
}

/// Lets Zephium look for the apps on this computer. Called once by the
/// desktop composition root; until then no app is found or run.
pub fn enable() {
    apps().enabled = true;
}

/// Scans once per process, the first time models are read.
pub(super) async fn ensure_scanned() {
    let due = {
        let apps = apps();
        apps.enabled && !apps.scanned
    };
    if due {
        statuses(false).await;
    }
}

/// The apps as last scanned; `rescan` (or no scan yet) asks them afresh and
/// tells observers when anything changed.
pub async fn statuses(rescan: bool) -> Vec<CliAppStatus> {
    let _scan = scans().lock().await;
    {
        let apps = apps();
        if !apps.enabled || (apps.scanned && !rescan) {
            return apps.statuses.clone();
        }
    }
    let found = zephium_cli_models::scan_all().await;
    let changed = {
        let mut apps = apps();
        let changed = apps.statuses != found;
        apps.statuses = found.clone();
        apps.scanned = true;
        changed
    };
    if changed {
        super::notify();
    }
    found
}

fn status(provider: WorkModelProvider) -> Option<CliAppStatus> {
    let app = CliApp::from_provider(provider)?;
    apps()
        .statuses
        .iter()
        .find(|status| status.app == app)
        .cloned()
}

/// Whether calls through this app can be made now.
pub(super) fn ready(provider: WorkModelProvider) -> bool {
    status(provider).is_some_and(|status| status.ready() && status.app.has_client())
}

/// The models of every installed app, for the picker.
pub(super) fn entries() -> Vec<WorkModelEntry> {
    apps()
        .statuses
        .iter()
        .filter(|status| status.path.is_some())
        .flat_map(|status| status.models.iter().cloned())
        .collect()
}

/// One app's models.
pub(super) fn models(provider: WorkModelProvider) -> Vec<WorkModelEntry> {
    status(provider)
        .map(|status| status.models)
        .unwrap_or_default()
}

/// What `role` runs from this app when the person chose nothing.
pub(super) fn family_entry(
    provider: WorkModelProvider,
    role: WorkModelRole,
) -> Option<WorkModelEntry> {
    let status = status(provider)?;
    let preferred = zephium_cli_models::preferred_models(status.app, role);
    preferred
        .iter()
        .find_map(|model| {
            status
                .models
                .iter()
                .find(|entry| entry.model.model == *model)
        })
        .or_else(|| {
            status
                .models
                .iter()
                .find(|entry| entry.roles.contains(&role))
        })
        .cloned()
}

/// A client that runs one model call through the app.
pub(super) fn client(entry: &WorkModelEntry) -> Result<Arc<dyn WorkModelClient>, WorkModelError> {
    let status = status(entry.model.provider).ok_or(WorkModelError::BadRequest)?;
    if !status.ready() || !status.app.has_client() {
        return Err(WorkModelError::MissingKey);
    }
    let program = status.path.ok_or(WorkModelError::MissingKey)?;
    Ok(Arc::new(CliModelClient::new(status.app, program)?))
}

/// Replaces the scan, for tests of the model selection.
#[cfg(test)]
pub(super) fn install_for_tests(statuses: Vec<CliAppStatus>) {
    let mut apps = apps();
    apps.enabled = false;
    apps.scanned = true;
    apps.statuses = statuses;
}
