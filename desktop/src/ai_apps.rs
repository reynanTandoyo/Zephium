//! Fork-owned (T-0008, `docs/fork-cli-models.md`): "AI apps on this PC" in
//! Settings → AI. Scans for Claude Code, Antigravity and OpenCode, and opens
//! a visible terminal to install one or to sign in to it — only when the
//! person clicks; nothing is ever installed or run in the background here.
use super::work_models::WorkModelsFaultV1;
use super::*;
use zephium_core::work::model::WorkModelProvider;

// ---------------------------------------------------------------------------
// INSTALL AND SIGN-IN COMMANDS — the only place they are written.
// Taken from each vendor's official documentation (checked 2026-11-13). `install` runs in a visible PowerShell window
// (Windows) or Terminal (macOS); `None` opens `url` in a tab instead.
// ---------------------------------------------------------------------------
struct AppCommands {
    app: WorkAiAppIdV1,
    install: Option<&'static str>,
    sign_in: &'static str,
    url: &'static str,
}

const APP_COMMANDS: [AppCommands; 3] = [
    AppCommands {
        app: WorkAiAppIdV1::ClaudeCode,
        install: Some("irm https://claude.ai/install.ps1 | iex"),
        sign_in: "claude auth login",
        url: "https://code.claude.com/docs/en/setup",
    },
    AppCommands {
        app: WorkAiAppIdV1::Antigravity,
        install: Some("irm https://antigravity.google/cli/install.ps1 | iex"),
        // Opens the browser sign-in.
        sign_in: "agy",
        url: "https://antigravity.google/docs/getting-started?tab=cli",
    },
    AppCommands {
        app: WorkAiAppIdV1::OpenCode,
        install: Some("npm install -g @opencode/cli"),
        sign_in: "opencode auth login",
        url: "https://opencode.ai/download",
    },
];
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkAiAppIdV1 {
    ClaudeCode,
    Antigravity,
    OpenCode,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkAiAppStateV1 {
    Missing,
    SignInNeeded,
    Ready,
    Unknown,
}

/// How the app is installed: a command for a visible terminal, or a page.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct WorkAiAppInstallV1 {
    pub(crate) command: Option<String>,
    pub(crate) url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, Eq, PartialEq)]
pub(crate) struct WorkAiAppV1 {
    pub(crate) app: WorkAiAppIdV1,
    pub(crate) provider: WorkModelProvider,
    pub(crate) name: String,
    pub(crate) state: WorkAiAppStateV1,
    pub(crate) version: Option<String>,
    pub(crate) install: WorkAiAppInstallV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub(crate) struct WorkAiAppsV1 {
    pub(crate) apps: Vec<WorkAiAppV1>,
}

impl WorkAiAppIdV1 {
    const ALL: [WorkAiAppIdV1; 3] = [Self::ClaudeCode, Self::Antigravity, Self::OpenCode];

    fn provider(self) -> WorkModelProvider {
        match self {
            Self::ClaudeCode => WorkModelProvider::ClaudeCode,
            Self::Antigravity => WorkModelProvider::Antigravity,
            Self::OpenCode => WorkModelProvider::OpenCode,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Antigravity => "Antigravity",
            Self::OpenCode => "OpenCode",
        }
    }

    fn commands(self) -> &'static AppCommands {
        APP_COMMANDS
            .iter()
            .find(|commands| commands.app == self)
            .unwrap_or(&APP_COMMANDS[0])
    }

    fn of(provider: WorkModelProvider) -> Option<Self> {
        Self::ALL.into_iter().find(|app| app.provider() == provider)
    }
}

/// One app's row before a scan, or in a build without Work.
fn row(app: WorkAiAppIdV1, state: WorkAiAppStateV1, version: Option<String>) -> WorkAiAppV1 {
    let commands = app.commands();
    WorkAiAppV1 {
        app,
        provider: app.provider(),
        name: app.name().to_owned(),
        state,
        version,
        install: WorkAiAppInstallV1 {
            command: commands.install.map(str::to_owned),
            url: commands.url.to_owned(),
        },
    }
}

#[cfg(feature = "work-product")]
fn rows(statuses: &[zephium_app::work_models::cli::CliAppStatus]) -> Vec<WorkAiAppV1> {
    use zephium_app::work_models::cli::CliAppState;
    WorkAiAppIdV1::ALL
        .into_iter()
        .map(|app| {
            match statuses
                .iter()
                .find(|status| status.app.provider() == app.provider())
            {
                Some(status) => row(
                    app,
                    match status.state {
                        CliAppState::Missing => WorkAiAppStateV1::Missing,
                        CliAppState::SignInNeeded => WorkAiAppStateV1::SignInNeeded,
                        CliAppState::Ready => WorkAiAppStateV1::Ready,
                        CliAppState::Unknown => WorkAiAppStateV1::Unknown,
                    },
                    status.version.clone(),
                ),
                None => row(app, WorkAiAppStateV1::Unknown, None),
            }
        })
        .collect()
}

/// The app row Settings shows for `provider`, from the last scan.
pub(crate) async fn app_of(provider: WorkModelProvider) -> Option<WorkAiAppV1> {
    let app = WorkAiAppIdV1::of(provider)?;
    #[cfg(feature = "work-product")]
    {
        let statuses = zephium_app::work_models::cli::statuses(false).await;
        rows(&statuses).into_iter().find(|row| row.app == app)
    }
    #[cfg(not(feature = "work-product"))]
    Some(row(app, WorkAiAppStateV1::Unknown, None))
}

/// Lets the model registry look for the apps; called once at startup.
pub(crate) fn install() {
    #[cfg(feature = "work-product")]
    zephium_app::work_models::cli::enable();
}

/// The apps on this computer; `rescan: false` returns the last scan.
#[tauri::command]
#[specta::specta]
pub(crate) async fn work_ai_apps(caller: WebviewWindow, rescan: bool) -> WorkAiAppsV1 {
    if !authorize(&caller, CallerPolicy::Main, "work_ai_apps") {
        return WorkAiAppsV1 { apps: Vec::new() };
    }
    #[cfg(feature = "work-product")]
    {
        // Emits `zephium:work-models-changed` itself when the scan changed.
        let statuses = zephium_app::work_models::cli::statuses(rescan).await;
        WorkAiAppsV1 {
            apps: rows(&statuses),
        }
    }
    #[cfg(not(feature = "work-product"))]
    {
        let _ = rescan;
        WorkAiAppsV1 {
            apps: WorkAiAppIdV1::ALL
                .into_iter()
                .map(|app| row(app, WorkAiAppStateV1::Unknown, None))
                .collect(),
        }
    }
}

/// Opens a visible terminal running the app's official install command, or
/// its download page in a tab when it has no scriptable installer.
#[tauri::command]
#[specta::specta]
pub(crate) async fn work_ai_app_install(
    caller: WebviewWindow,
    app: WorkAiAppIdV1,
) -> Result<(), WorkModelsFaultV1> {
    if !authorize(&caller, CallerPolicy::Main, "work_ai_app_install") {
        return Err(WorkModelsFaultV1::Unavailable);
    }
    let commands = app.commands();
    let result = match commands.install {
        Some(command) => open_terminal(command),
        None => open_page(&caller, commands.url),
    };
    log(format_args!(
        "work: phase=ai_apps action=install app={app:?} ok={}",
        result.is_ok()
    ));
    result
}

/// Opens a visible terminal running the app so the person can sign in once.
#[tauri::command]
#[specta::specta]
pub(crate) async fn work_ai_app_sign_in(
    caller: WebviewWindow,
    app: WorkAiAppIdV1,
) -> Result<(), WorkModelsFaultV1> {
    if !authorize(&caller, CallerPolicy::Main, "work_ai_app_sign_in") {
        return Err(WorkModelsFaultV1::Unavailable);
    }
    let result = open_terminal(app.commands().sign_in);
    log(format_args!(
        "work: phase=ai_apps action=sign_in app={app:?} ok={}",
        result.is_ok()
    ));
    result
}

fn log(arguments: std::fmt::Arguments<'_>) {
    #[cfg(feature = "work-product")]
    super::work_provider::record_diagnostic(arguments);
    #[cfg(not(feature = "work-product"))]
    super::write_diagnostic(arguments);
}

fn open_page(caller: &WebviewWindow, url: &str) -> Result<(), WorkModelsFaultV1> {
    let app = caller.app_handle();
    let shell = app.state::<Handle>();
    let admission = dispatch_operation(
        app,
        &shell,
        Command::OpenUrl {
            input: url.to_owned(),
            new_tab: true,
        },
    );
    if admission.accepted {
        Ok(())
    } else {
        Err(WorkModelsFaultV1::Unavailable)
    }
}

/// `command` as PowerShell's `-EncodedCommand`: UTF-16LE in base64, so no
/// shell re-parses the text on the way.
#[cfg(any(target_os = "windows", test))]
fn encoded_command(command: &str) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Runs one of the fixed commands above in a new, visible PowerShell window.
#[cfg(target_os = "windows")]
fn open_terminal(command: &str) -> Result<(), WorkModelsFaultV1> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let root = std::env::var_os("SystemRoot").ok_or(WorkModelsFaultV1::Unavailable)?;
    let shell = std::path::PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    std::process::Command::new(shell)
        .args([
            "-NoExit",
            "-NoLogo",
            "-EncodedCommand",
            &encoded_command(command),
        ])
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(drop)
        .map_err(|_| WorkModelsFaultV1::Unavailable)
}

/// Runs one of the fixed commands above in a new Terminal window.
#[cfg(target_os = "macos")]
fn open_terminal(command: &str) -> Result<(), WorkModelsFaultV1> {
    let quoted = command.replace('\\', "\\\\").replace('"', "\\\"");
    std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            &format!("tell application \"Terminal\" to do script \"{quoted}\""),
            "-e",
            "tell application \"Terminal\" to activate",
        ])
        .spawn()
        .map(drop)
        .map_err(|_| WorkModelsFaultV1::Unavailable)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_terminal(_command: &str) -> Result<(), WorkModelsFaultV1> {
    Err(WorkModelsFaultV1::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_rows_have_stable_wire_shapes_and_one_command_table() {
        let value = serde_json::to_value(row(
            WorkAiAppIdV1::OpenCode,
            WorkAiAppStateV1::SignInNeeded,
            Some("1.0.0".into()),
        ))
        .unwrap();
        assert_eq!(value["app"], "open_code");
        assert_eq!(value["provider"], "open_code");
        assert_eq!(value["state"], "sign_in_needed");
        assert_eq!(value["install"]["command"], "npm install -g @opencode/cli");
        for app in WorkAiAppIdV1::ALL {
            assert_eq!(app.commands().app, app);
            assert!(app.commands().url.starts_with("https://"));
            assert_eq!(WorkAiAppIdV1::of(app.provider()), Some(app));
        }
        assert_eq!(WorkAiAppIdV1::of(WorkModelProvider::Anthropic), None);
        // "irm" as UTF-16LE.
        assert_eq!(encoded_command("irm"), "aQByAG0A");
    }
}
