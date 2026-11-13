//! Runs the client and the scan against the crate's fake app, never a real one.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zephium_cli_models::{scan_at, CliApp, CliAppState, CliModelClient};
use zephium_core::work::model::{
    WorkModelClient, WorkModelError, WorkModelEvent, WorkModelMessage, WorkModelPart, WorkModelRef,
    WorkModelRequest, WorkModelStop, WorkModelSystemBlock, WorkModelTool, WorkModelToolChoice,
    WorkModelWire,
};

const FAKE: &str = env!("CARGO_BIN_EXE_zephium-cli-models-fake");

fn request(app: CliApp, prompt: &str, choice: WorkModelToolChoice) -> WorkModelRequest {
    WorkModelRequest {
        image_fence: None,
        model: WorkModelRef {
            provider: app.provider(),
            wire: WorkModelWire::Cli,
            model: "sonnet".into(),
        },
        system: vec![WorkModelSystemBlock {
            text: "SYSTEM_MARKER be brief.".into(),
            cache: true,
        }],
        tools: vec![WorkModelTool {
            name: "open_page".into(),
            description: "Opens a page.".into(),
            schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            strict: false,
        }],
        messages: vec![WorkModelMessage::User(vec![WorkModelPart::Text(
            prompt.into(),
        )])],
        max_output_tokens: 1000,
        reasoning: None,
        native_search: false,
        parallel_tools: true,
        tool_choice: choice,
        volatile_tail: 0,
        cache_key: None,
    }
}

async fn call(
    app: CliApp,
    prompt: &str,
    choice: WorkModelToolChoice,
    timeout: Duration,
) -> (
    Result<zephium_core::work::model::WorkModelOutcome, WorkModelError>,
    Vec<WorkModelEvent>,
) {
    let client = CliModelClient::new(app, PathBuf::from(FAKE))
        .unwrap()
        .with_timeout(timeout);
    let seen = Mutex::new(Vec::new());
    let events = |event: WorkModelEvent| seen.lock().unwrap().push(event);
    let result = client.call(request(app, prompt, choice), &events).await;
    (result, seen.into_inner().unwrap())
}

const LONG: Duration = Duration::from_secs(60);

#[tokio::test]
async fn tool_calls_and_text_come_back_as_parts_and_events() {
    for app in [CliApp::ClaudeCode, CliApp::Antigravity] {
        let (outcome, events) =
            call(app, "please FAKE_CALL", WorkModelToolChoice::Required, LONG).await;
        let outcome = outcome.unwrap();
        assert_eq!(outcome.stop, WorkModelStop::ToolUse, "{app:?}");
        let calls: Vec<_> = outcome
            .assistant
            .iter()
            .filter_map(|part| match part {
                WorkModelPart::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "open_page");
        assert_eq!(calls[0].arguments, json!({"q": "fake"}));
        assert!(!calls[0].id.is_empty());
        assert!(matches!(&events[0], WorkModelEvent::Text(text) if text == "Calling."));
        assert!(matches!(&events[1], WorkModelEvent::ToolCall(call) if call.id == calls[0].id));
        assert_eq!(outcome.usage.cost_micros, Some(0));
        assert!(outcome.usage.input_tokens > 0);

        let (outcome, events) = call(app, "just talk", WorkModelToolChoice::Auto, LONG).await;
        let outcome = outcome.unwrap();
        assert_eq!(outcome.stop, WorkModelStop::EndTurn);
        assert!(
            matches!(&events[..], [WorkModelEvent::Text(text)] if text == "Hello from the fake.")
        );
    }
}

#[tokio::test]
async fn prompt_goes_on_stdin_with_a_private_folder_and_allowlisted_environment() {
    for app in [CliApp::ClaudeCode, CliApp::Antigravity] {
        let (outcome, events) = call(app, "FAKE_ECHO", WorkModelToolChoice::Auto, LONG).await;
        outcome.unwrap();
        let [WorkModelEvent::Text(text)] = &events[..] else {
            panic!("{app:?}: one text event");
        };
        let echo: Value = serde_json::from_str(text).unwrap();
        assert_eq!(echo["prompt_in_argv"], false, "{app:?}");
        let args = echo["args"].as_array().unwrap();
        assert!(!args
            .iter()
            .any(|arg| arg.as_str().unwrap().contains("SYSTEM_MARKER")));
        let env: Vec<&str> = echo["env"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        assert!(env.iter().any(|name| name.eq_ignore_ascii_case("PATH")));
        assert!(!env
            .iter()
            .any(|name| name.to_ascii_uppercase().ends_with("API_KEY")));
        let cwd = echo["cwd"].as_str().unwrap();
        assert!(cwd.contains("zephium-cli-"), "{cwd}");
        match app {
            CliApp::ClaudeCode => {
                assert!(echo["system"].as_str().unwrap().contains("SYSTEM_MARKER"));
                assert!(args.iter().any(|arg| arg == "--tools"));
            }
            _ => {
                assert!(echo["agent"].as_str().unwrap().contains("tools: []"));
                assert!(args.iter().any(|arg| arg == "zephium-model"));
            }
        }
        // The private folder is removed after the call.
        assert!(!std::path::Path::new(cwd).exists());
    }
}

#[tokio::test]
async fn failures_are_closed_and_runs_are_bounded() {
    for app in [CliApp::ClaudeCode, CliApp::Antigravity] {
        let (result, events) = call(app, "FAKE_LIMIT", WorkModelToolChoice::Auto, LONG).await;
        assert!(
            matches!(result.err(), Some(WorkModelError::RateLimited { .. })),
            "{app:?}"
        );
        assert!(events.is_empty());
        let (result, _) = call(app, "FAKE_LOGIN", WorkModelToolChoice::Auto, LONG).await;
        assert_eq!(result.err(), Some(WorkModelError::MissingKey), "{app:?}");
        let (result, _) = call(app, "FAKE_GARBAGE", WorkModelToolChoice::Auto, LONG).await;
        assert_eq!(result.err(), Some(WorkModelError::Protocol), "{app:?}");
        let (result, _) = call(app, "FAKE_BADARGS", WorkModelToolChoice::Auto, LONG).await;
        assert_eq!(result.err(), Some(WorkModelError::Protocol), "{app:?}");
        let (result, _) = call(app, "FAKE_HUGE", WorkModelToolChoice::Auto, LONG).await;
        assert_eq!(result.err(), Some(WorkModelError::Protocol), "{app:?}");
        let started = Instant::now();
        let (result, _) = call(
            app,
            "FAKE_SLEEP",
            WorkModelToolChoice::Auto,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(result.err(), Some(WorkModelError::Network), "{app:?}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }
    // A request for another provider or wire never runs the app.
    let client = CliModelClient::new(CliApp::ClaudeCode, PathBuf::from(FAKE)).unwrap();
    let mut wrong = request(CliApp::Antigravity, "x", WorkModelToolChoice::Auto);
    let ignore = |_: WorkModelEvent| {};
    assert_eq!(
        client.call(wrong.clone(), &ignore).await.err(),
        Some(WorkModelError::BadRequest)
    );
    wrong.model.provider = CliApp::ClaudeCode.provider();
    wrong.model.model = "sonnet; rm".into();
    assert_eq!(
        client.call(wrong, &ignore).await.err(),
        Some(WorkModelError::BadRequest)
    );
    assert!(CliModelClient::new(CliApp::OpenCode, PathBuf::from(FAKE)).is_err());
    assert!(CliModelClient::new(CliApp::ClaudeCode, PathBuf::from("fake.exe")).is_err());
}

#[tokio::test]
async fn dropping_a_call_ends_the_app() {
    let client = CliModelClient::new(CliApp::ClaudeCode, PathBuf::from(FAKE)).unwrap();
    let ignore = |_: WorkModelEvent| {};
    let started = Instant::now();
    let dropped = tokio::time::timeout(
        Duration::from_millis(500),
        client.call(
            request(CliApp::ClaudeCode, "FAKE_SLEEP", WorkModelToolChoice::Auto),
            &ignore,
        ),
    )
    .await;
    assert!(dropped.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn fake_copy(name: &str, mode: &str) -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let program = folder
        .path()
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(FAKE, &program).unwrap();
    let mut mode_file = program.clone().into_os_string();
    mode_file.push(".mode");
    std::fs::write(mode_file, mode).unwrap();
    (folder, program)
}

#[tokio::test]
async fn scans_read_version_sign_in_and_models() {
    let missing = scan_at(CliApp::ClaudeCode, None).await;
    assert_eq!(missing.state, CliAppState::Missing);
    assert!(!missing.ready());

    let (_keep, claude) = fake_copy("claude", "");
    let status = scan_at(CliApp::ClaudeCode, Some(claude)).await;
    assert_eq!(status.state, CliAppState::Ready);
    assert_eq!(status.version.as_deref(), Some("9.8.7"));
    assert_eq!(status.models.len(), 4);

    let (_keep, claude) = fake_copy("claude", "signed_out");
    let status = scan_at(CliApp::ClaudeCode, Some(claude)).await;
    assert_eq!(status.state, CliAppState::SignInNeeded);

    let (_keep, agy) = fake_copy("agy", "");
    let status = scan_at(CliApp::Antigravity, Some(agy)).await;
    assert_eq!(status.state, CliAppState::Ready);
    let ids: Vec<_> = status
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["antigravity/fake-model-high", "antigravity/fake-model-low"]
    );

    let (_keep, agy) = fake_copy("agy", "signed_out");
    let status = scan_at(CliApp::Antigravity, Some(agy)).await;
    assert_eq!(status.state, CliAppState::SignInNeeded);
    assert!(!status.models.is_empty(), "the static list stands in");

    let (_keep, opencode) = fake_copy("opencode", "");
    let status = scan_at(CliApp::OpenCode, Some(opencode)).await;
    assert_eq!(status.state, CliAppState::Unknown);
    assert_eq!(status.version.as_deref(), Some("9.8.7"));

    let (_keep, hung) = fake_copy("claude", "hang");
    let started = Instant::now();
    let status = scan_at(CliApp::ClaudeCode, Some(hung)).await;
    assert_eq!(status.version, None);
    assert!(started.elapsed() < Duration::from_secs(15));
}
