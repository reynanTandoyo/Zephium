//! Fork-owned (T-0008, `docs/fork-cli-models.md`): AI apps installed on this
//! computer (Claude Code, Antigravity, OpenCode) as plain model backends.
//!
//! Each app is found on disk, asked for its version and sign-in state, and
//! run once per model call in print mode with JSON output. The app is a
//! *model*, not an agent: its own tools, MCP servers, skills and settings are
//! switched off for every call, the prompt goes on stdin only, and its output
//! is untrusted data. Failures cross the boundary as the closed
//! [`WorkModelError`]; no app text is kept in errors or logs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use zephium_core::work::model::{
    WorkModelEntry, WorkModelPrice, WorkModelProvider, WorkModelRef, WorkModelRole,
    WorkModelSupports, WorkModelWire,
};

mod client;
mod locate;
mod prompt;
mod spawn;

pub use client::CliModelClient;

/// How long a version or sign-in probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// `agy models` asks Google's servers, so it gets a little longer.
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_LISTED_MODELS: usize = 64;

/// An AI app Zephium can run as a model.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum CliApp {
    ClaudeCode,
    Antigravity,
    OpenCode,
}

impl CliApp {
    /// In the order Settings lists them and runs fall back to them.
    pub const ALL: [CliApp; 3] = [CliApp::ClaudeCode, CliApp::Antigravity, CliApp::OpenCode];

    pub fn provider(self) -> WorkModelProvider {
        match self {
            Self::ClaudeCode => WorkModelProvider::ClaudeCode,
            Self::Antigravity => WorkModelProvider::Antigravity,
            Self::OpenCode => WorkModelProvider::OpenCode,
        }
    }

    pub fn from_provider(provider: WorkModelProvider) -> Option<Self> {
        Self::ALL.into_iter().find(|app| app.provider() == provider)
    }

    /// The name people know the app by.
    pub fn name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Antigravity => "Antigravity",
            Self::OpenCode => "OpenCode",
        }
    }

    /// The command the app installs.
    pub fn program(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Antigravity => "agy",
            Self::OpenCode => "opencode",
        }
    }

    /// Whether Zephium can run model calls through this app yet. OpenCode's
    /// print mode is not qualified, so it is found and shown but never used.
    pub fn has_client(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Antigravity)
    }
}

/// What a scan found about one app.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CliAppState {
    /// Not installed.
    Missing,
    /// Installed, but the person must sign in once in the app.
    SignInNeeded,
    /// Installed, signed in and usable as a model.
    Ready,
    /// Installed, but its state could not be read (or it has no client yet).
    Unknown,
}

/// One app as a scan found it. Holds no account details.
#[derive(Clone, Debug, PartialEq)]
pub struct CliAppStatus {
    pub app: CliApp,
    /// The absolute path of the program, when installed.
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub state: CliAppState,
    /// The models this app offers, free of charge to Zephium.
    pub models: Vec<WorkModelEntry>,
}

impl CliAppStatus {
    pub fn ready(&self) -> bool {
        self.state == CliAppState::Ready && self.path.is_some()
    }
}

/// Where `app` is installed. Blocking, but only reads file metadata.
pub fn locate(app: CliApp) -> Option<PathBuf> {
    locate::locate(app.program(), &locate::extra_folders(app))
}

/// Scans every app, in parallel.
pub async fn scan_all() -> Vec<CliAppStatus> {
    let (claude, agy, opencode) = tokio::join!(
        scan(CliApp::ClaudeCode),
        scan(CliApp::Antigravity),
        scan(CliApp::OpenCode)
    );
    vec![claude, agy, opencode]
}

/// Finds `app` and asks it for its version and sign-in state.
pub async fn scan(app: CliApp) -> CliAppStatus {
    scan_at(app, locate(app)).await
}

/// Like [`scan`] for an app at a known path (or none).
pub async fn scan_at(app: CliApp, path: Option<PathBuf>) -> CliAppStatus {
    let Some(program) = path.clone() else {
        return CliAppStatus {
            app,
            path,
            version: None,
            state: CliAppState::Missing,
            models: Vec::new(),
        };
    };
    let Ok(folder) = tempfile::Builder::new()
        .prefix("zephium-cli-scan-")
        .tempdir()
    else {
        return CliAppStatus {
            app,
            path,
            version: None,
            state: CliAppState::Unknown,
            models: static_models(app),
        };
    };
    let probe = |args: &'static [&'static str], timeout: Duration| {
        let program = program.clone();
        let cwd = folder.path().to_path_buf();
        async move {
            spawn::run(spawn::Run {
                program: &program,
                args: args.iter().map(Into::into).collect(),
                cwd: &cwd,
                stdin: None,
                timeout,
                max_stdout: PROBE_OUTPUT_BYTES,
                extra_env: &[],
            })
            .await
            .ok()
        }
    };
    let (version, state) = match app {
        CliApp::ClaudeCode => {
            let (version, auth) = tokio::join!(
                probe(&["--version"], PROBE_TIMEOUT),
                probe(&["auth", "status"], PROBE_TIMEOUT)
            );
            let state = match auth {
                Some(output) => claude_auth_state(&output.stdout),
                None => CliAppState::Unknown,
            };
            (version, (state, static_models(app)))
        }
        CliApp::Antigravity => {
            let (version, listed) = tokio::join!(
                probe(&["--version"], PROBE_TIMEOUT),
                probe(&["models"], LIST_TIMEOUT)
            );
            let state = match listed {
                Some(output) if output.success => {
                    let models = agy_models(&String::from_utf8_lossy(&output.stdout));
                    if models.is_empty() {
                        (CliAppState::Unknown, static_models(app))
                    } else {
                        (CliAppState::Ready, models)
                    }
                }
                Some(output) => {
                    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                    text.push_str(&String::from_utf8_lossy(&output.stderr));
                    let state = if signed_out_text(&text) {
                        CliAppState::SignInNeeded
                    } else {
                        CliAppState::Unknown
                    };
                    (state, static_models(app))
                }
                None => (CliAppState::Unknown, static_models(app)),
            };
            (version, state)
        }
        CliApp::OpenCode => (
            probe(&["--version"], PROBE_TIMEOUT).await,
            (CliAppState::Unknown, Vec::new()),
        ),
    };
    let (state, models) = state;
    CliAppStatus {
        app,
        path,
        version: version.and_then(|output| version_of(&String::from_utf8_lossy(&output.stdout))),
        state,
        models,
    }
}

/// "2.1.319" from the first line of a `--version`.
pub fn version_of(text: &str) -> Option<String> {
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    line.split_whitespace()
        .map(|word| word.trim_start_matches('v'))
        .find(|word| {
            word.chars().next().is_some_and(|c| c.is_ascii_digit())
                && word.contains('.')
                && word.len() <= 32
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        })
        .map(str::to_owned)
}

/// Claude Code's `auth status` JSON: signed in or not; no account details kept.
pub fn claude_auth_state(stdout: &[u8]) -> CliAppState {
    match serde_json::from_slice::<serde_json::Value>(stdout) {
        Ok(value) if value["loggedIn"] == serde_json::Value::Bool(true) => CliAppState::Ready,
        Ok(value) if value["loggedIn"] == serde_json::Value::Bool(false) => {
            CliAppState::SignInNeeded
        }
        _ => CliAppState::Unknown,
    }
}

fn signed_out_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "sign in",
        "sign-in",
        "signin",
        "log in",
        "login",
        "not authenticated",
        "unauthenticated",
    ]
    .iter()
    .any(|hint| text.contains(hint))
}

fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
}

/// `agy models` prints one `id<TAB>Name` per line.
pub fn agy_models(text: &str) -> Vec<WorkModelEntry> {
    let mut models: Vec<WorkModelEntry> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(id), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (id, name) = (id.trim(), name.trim());
        if !valid_model_id(id)
            || name.is_empty()
            || name.len() > 96
            || name.chars().any(char::is_control)
            || models.iter().any(|model| model.model.model == id)
        {
            continue;
        }
        models.push(agy_entry(id, name));
        if models.len() == MAX_LISTED_MODELS {
            break;
        }
    }
    models
}

fn agy_entry(id: &str, name: &str) -> WorkModelEntry {
    let context = if id.starts_with("gemini-") {
        1_000_000
    } else if id.starts_with("gpt-oss") {
        128_000
    } else {
        200_000
    };
    entry(CliApp::Antigravity, id, name, context, 32_000)
}

/// The models Settings shows before an app has been asked for its own list.
pub fn static_models(app: CliApp) -> Vec<WorkModelEntry> {
    match app {
        CliApp::ClaudeCode => [
            ("fable", "Fable"),
            ("opus", "Opus"),
            ("sonnet", "Sonnet"),
            ("haiku", "Haiku"),
        ]
        .into_iter()
        // Claude Code runs every alias with a 200k window unless asked for more.
        .map(|(id, name)| entry(app, id, name, 200_000, 32_000))
        .collect(),
        CliApp::Antigravity => [
            ("gemini-4-argon-medium", "Gemini 4 Argon (Medium)"),
            ("gemini-3.8-flash-medium", "Gemini 3.8 Flash (Medium)"),
            ("gemini-3.8-flash-low", "Gemini 3.8 Flash (Low)"),
        ]
        .into_iter()
        .map(|(id, name)| agy_entry(id, name))
        .collect(),
        CliApp::OpenCode => Vec::new(),
    }
}

/// The models a role prefers from `app`, best first, when the person chose none.
pub fn preferred_models(app: CliApp, role: WorkModelRole) -> &'static [&'static str] {
    match (app, role) {
        (CliApp::ClaudeCode, WorkModelRole::Lead) => &["opus", "fable", "sonnet"],
        (CliApp::ClaudeCode, WorkModelRole::Page) => &["sonnet", "opus"],
        (CliApp::ClaudeCode, WorkModelRole::Light | WorkModelRole::Decision) => {
            &["haiku", "sonnet"]
        }
        (CliApp::Antigravity, WorkModelRole::Lead) => &[
            "gemini-4-argon-medium",
            "gemini-4-argon-high",
            "gemini-3.8-flash-high",
        ],
        (CliApp::Antigravity, WorkModelRole::Page) => {
            &["gemini-3.8-flash-medium", "gemini-3.8-flash-high"]
        }
        (CliApp::Antigravity, WorkModelRole::Light | WorkModelRole::Decision) => {
            &["gemini-3.8-flash-low", "gemini-3.8-flash-medium"]
        }
        (CliApp::OpenCode, _) => &[],
    }
}

/// A catalog entry for one model of `app`: free, tools, no images.
pub fn entry(app: CliApp, model: &str, name: &str, context: u32, output: u32) -> WorkModelEntry {
    let provider = app.provider();
    let slug = match app {
        CliApp::ClaudeCode => "claude-code",
        CliApp::Antigravity => "antigravity",
        CliApp::OpenCode => "opencode",
    };
    WorkModelEntry {
        id: format!("{slug}/{model}"),
        model: WorkModelRef {
            provider,
            wire: WorkModelWire::Cli,
            model: model.to_owned(),
        },
        display_name: name.to_owned(),
        roles: vec![
            WorkModelRole::Lead,
            WorkModelRole::Page,
            WorkModelRole::Light,
        ],
        recommended: false,
        context_window: context,
        max_output: output,
        supports: WorkModelSupports {
            tools: true,
            vision: false,
            prompt_cache: false,
            reasoning: true,
            native_search: false,
        },
        price: Some(WorkModelPrice {
            input: 0,
            cached_input: 0,
            output: 0,
        }),
    }
}

/// A program path the model client may run: absolute, and a native
/// executable rather than a batch script that would re-parse its arguments.
fn runnable(path: &Path) -> bool {
    path.is_absolute()
        && (cfg!(not(windows))
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("com")
                }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_auth_and_model_lists_parse_from_recorded_output() {
        assert_eq!(
            version_of("2.1.319 (Claude Code)\n"),
            Some("2.1.319".into())
        );
        assert_eq!(version_of("1.5.8\n"), Some("1.5.8".into()));
        assert_eq!(version_of("opencode v0.9.1"), Some("0.9.1".into()));
        assert_eq!(version_of("no version here"), None);
        assert_eq!(
            claude_auth_state(br#"{"loggedIn": true, "authMethod": "claude.ai"}"#),
            CliAppState::Ready
        );
        assert_eq!(
            claude_auth_state(br#"{"loggedIn": false}"#),
            CliAppState::SignInNeeded
        );
        assert_eq!(claude_auth_state(b"garbage"), CliAppState::Unknown);
        let listed = agy_models(
            "Fetching available models...\ngemini-4-argon-medium\tGemini 4 Argon (Medium)\n\
             bad id with spaces\tNope\ngemini-4-argon-medium\tDuplicate\n\
             claude-opus-5-5-low\tClaude Opus 5.5 (Low)\nno-tab-line\n",
        );
        let ids: Vec<_> = listed.iter().map(|entry| entry.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "antigravity/gemini-4-argon-medium",
                "antigravity/claude-opus-5-5-low"
            ]
        );
        assert_eq!(listed[0].context_window, 1_000_000);
        assert_eq!(listed[1].context_window, 200_000);
    }

    #[test]
    fn entries_are_free_tool_capable_and_text_only() {
        for app in [CliApp::ClaudeCode, CliApp::Antigravity] {
            let models = static_models(app);
            assert!(!models.is_empty());
            for model in models {
                assert_eq!(model.model.provider, app.provider());
                assert_eq!(model.model.wire, WorkModelWire::Cli);
                assert!(model.is_free_compatible());
                assert!(model.supports.tools && !model.supports.vision);
                for role in [
                    WorkModelRole::Lead,
                    WorkModelRole::Page,
                    WorkModelRole::Light,
                ] {
                    assert!(model.roles.contains(&role));
                }
            }
            for role in [
                WorkModelRole::Lead,
                WorkModelRole::Page,
                WorkModelRole::Light,
            ] {
                let preferred = preferred_models(app, role);
                assert!(static_models(app)
                    .iter()
                    .any(|entry| preferred.contains(&entry.model.model.as_str())));
            }
        }
        assert_eq!(
            CliApp::from_provider(WorkModelProvider::Antigravity),
            Some(CliApp::Antigravity)
        );
        assert_eq!(CliApp::from_provider(WorkModelProvider::Anthropic), None);
        assert!(!CliApp::OpenCode.has_client());
    }

    #[test]
    fn only_absolute_native_programs_are_runnable() {
        assert!(!runnable(Path::new("claude.exe")));
        #[cfg(windows)]
        {
            assert!(runnable(Path::new(r"C:\tools\claude.exe")));
            assert!(!runnable(Path::new(r"C:\tools\claude.cmd")));
            assert!(!runnable(Path::new(r"C:\tools\claude.bat")));
        }
    }
}
