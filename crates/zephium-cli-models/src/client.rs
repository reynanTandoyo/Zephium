//! [`CliModelClient`]: one model call is one print-mode run of the app.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use zephium_core::work::model::{
    WorkModelClient, WorkModelError, WorkModelEvent, WorkModelFuture, WorkModelOutcome,
    WorkModelPart, WorkModelReasoning, WorkModelRequest, WorkModelStop, WorkModelUsage,
    WorkModelWire,
};

use super::prompt;
use super::spawn::{self, SpawnError};
use super::CliApp;

/// Below the lead's own 240 s call limit, so the run ends here first.
const CALL_TIMEOUT: Duration = Duration::from_secs(230);
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// The custom Antigravity agent each call runs as.
const AGY_AGENT: &str = "zephium-model";

/// Claude Code's built-in tools, denied by name as well as removed with
/// `--tools ""`, so a newer version that ignores one switch still has none.
const CLAUDE_TOOLS: &[&str] = &[
    "Bash",
    "PowerShell",
    "Edit",
    "MultiEdit",
    "Write",
    "Read",
    "Glob",
    "Grep",
    "LS",
    "WebFetch",
    "WebSearch",
    "Task",
    "Agent",
    "NotebookEdit",
    "NotebookRead",
    "TodoWrite",
    "KillShell",
    "BashOutput",
    "SlashCommand",
    "Skill",
];

/// The Antigravity agent each call runs as: no default prompt sections or
/// built-in tools, none of the person's skills, rules, plugins, subagents or
/// MCP servers. Checked with agy 1.5.8: such an agent can still list MCP
/// resources and its background tasks, and nothing else. Its one tool is
/// `finish`: agy delivers a `--json-schema` answer only through `finish`, so
/// without it every call loops until it gives up and has no structured output.
const AGY_AGENT_FILE: &str = "---\n\
name: zephium-model\n\
description: Zephium's plain language model; its only tool is finish.\n\
excludeDefaultComponents: true\n\
inheritCustomizations: false\n\
inheritMcp: false\n\
tools: [finish]\n\
---\n\
# Zephium model\n\
\n\
You are a language model answering for Zephium. Your only tool is finish, \
which delivers your answer: never run commands, read or write files, browse, \
or call MCP servers. Give the answer the request asks for through finish.\n";

/// Session-only Antigravity settings that deny every tool family even if the
/// person's own settings allow them, and keep file access in the workspace.
const AGY_SETTINGS: &str = r#"{"permissions":{"allow":[],"deny":["command(*)","read_file(*)","read_file(/)","write_file(*)","mcp(*)","read_url(*)"]},"allowNonWorkspaceAccess":false}"#;

/// A model call through an AI app on this computer.
pub struct CliModelClient {
    app: CliApp,
    program: PathBuf,
    timeout: Duration,
}

impl CliModelClient {
    /// A client for `app` installed at `program` (absolute, a native executable).
    pub fn new(app: CliApp, program: PathBuf) -> Result<Self, WorkModelError> {
        if !app.has_client() || !super::runnable(&program) {
            return Err(WorkModelError::BadRequest);
        }
        Ok(Self {
            app,
            program,
            timeout: CALL_TIMEOUT,
        })
    }

    /// The per-call limit, shorter in tests.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn run(
        &self,
        request: WorkModelRequest,
        events: &(dyn Fn(WorkModelEvent) + Send + Sync),
    ) -> Result<WorkModelOutcome, WorkModelError> {
        if request.model.provider != self.app.provider()
            || request.model.wire != WorkModelWire::Cli
            || !valid_model(&request.model.model)
        {
            return Err(WorkModelError::BadRequest);
        }
        let schema = prompt::schema(&request)?.to_string();
        let folder = tempfile::Builder::new()
            .prefix("zephium-cli-")
            .tempdir()
            .map_err(|_| WorkModelError::Network)?;
        let (args, stdin, extra_env): (Vec<OsString>, Vec<u8>, &[(&str, &str)]) = match self.app {
            CliApp::ClaudeCode => {
                let system = folder.path().join("system.md");
                std::fs::write(&system, prompt::system(&request))
                    .map_err(|_| WorkModelError::Network)?;
                (
                    claude_args(&request, &system, &schema),
                    prompt::render(&request, false)?.into_bytes(),
                    &[
                        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
                        ("DISABLE_AUTOUPDATER", "1"),
                    ],
                )
            }
            CliApp::Antigravity => {
                let agents = folder.path().join(".agents").join("agents");
                std::fs::create_dir_all(&agents).map_err(|_| WorkModelError::Network)?;
                std::fs::write(agents.join(format!("{AGY_AGENT}.md")), AGY_AGENT_FILE)
                    .map_err(|_| WorkModelError::Network)?;
                let line = json!({
                    "event": "user",
                    "message": { "content": prompt::render(&request, true)? },
                });
                let mut stdin = line.to_string().into_bytes();
                stdin.push(b'\n');
                (agy_args(&request, &schema, self.timeout), stdin, &[])
            }
            CliApp::OpenCode => return Err(WorkModelError::BadRequest),
        };
        let output = spawn::run(spawn::Run {
            program: &self.program,
            args,
            cwd: folder.path(),
            stdin: Some(stdin),
            timeout: self.timeout,
            max_stdout: MAX_OUTPUT_BYTES,
            extra_env,
        })
        .await
        .map_err(|error| match error {
            SpawnError::Spawn | SpawnError::Timeout | SpawnError::Io => WorkModelError::Network,
            SpawnError::TooLarge => WorkModelError::Protocol,
        })?;
        let (structured, usage) = match self.app {
            CliApp::ClaudeCode => claude_result(&output.stdout, &output.stderr)?,
            _ => agy_result(&output.stdout, &output.stderr)?,
        };
        let answer = prompt::parse(&structured, &request)?;
        drop(folder);
        let mut assistant = Vec::new();
        if !answer.text.is_empty() {
            events(WorkModelEvent::Text(answer.text.clone()));
            assistant.push(WorkModelPart::Text(answer.text));
        }
        for call in &answer.calls {
            events(WorkModelEvent::ToolCall(call.clone()));
        }
        let stop = if answer.calls.is_empty() {
            WorkModelStop::EndTurn
        } else {
            WorkModelStop::ToolUse
        };
        assistant.extend(answer.calls.into_iter().map(WorkModelPart::ToolCall));
        Ok(WorkModelOutcome {
            stop,
            usage,
            assistant,
        })
    }
}

impl WorkModelClient for CliModelClient {
    fn call<'a>(
        &'a self,
        request: WorkModelRequest,
        events: &'a (dyn Fn(WorkModelEvent) + Send + Sync),
    ) -> WorkModelFuture<'a> {
        Box::pin(self.run(request, events))
    }
}

fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 96
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'[' | b']'))
}

fn effort(reasoning: WorkModelReasoning) -> &'static str {
    match reasoning {
        WorkModelReasoning::Low => "low",
        WorkModelReasoning::Medium => "medium",
        WorkModelReasoning::High => "high",
        WorkModelReasoning::XHigh => "xhigh",
        WorkModelReasoning::Max => "max",
    }
}

/// `claude -p` as a plain model: no tools, MCP servers, skills, settings files
/// or saved session; the system text from a private file, the prompt on stdin.
pub(crate) fn claude_args(
    request: &WorkModelRequest,
    system: &Path,
    schema: &str,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-p",
        "--output-format",
        "json",
        "--no-session-persistence",
        "--model",
        &request.model.model,
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--tools",
        "",
        "--disable-slash-commands",
        "--permission-prompts",
        "none",
        "--json-schema",
        schema,
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.push("--system-prompt-file".into());
    args.push(system.as_os_str().to_owned());
    if let Some(reasoning) = request.reasoning.filter(|_| request.model.model != "haiku") {
        args.push("--effort".into());
        args.push(effort(reasoning).into());
    }
    args.push("--disallowedTools".into());
    args.extend(CLAUDE_TOOLS.iter().map(OsString::from));
    args
}

/// `agy` reading one stream-json turn from stdin, as the tool-less agent.
pub(crate) fn agy_args(
    request: &WorkModelRequest,
    schema: &str,
    timeout: Duration,
) -> Vec<OsString> {
    // Its own limit a little under ours, so it reports a partial answer itself.
    let limit = format!("{}s", timeout.as_secs().saturating_sub(5).max(1));
    [
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--json-schema",
        schema,
        "--agent",
        AGY_AGENT,
        "--model",
        &request.model.model,
        "--disable-slash-commands",
        "--sandbox",
        "--print-timeout",
        &limit,
        "--settings",
        AGY_SETTINGS,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// A closed failure for an app's own error text; the text is not kept.
pub(crate) fn classify(text: &str) -> Option<WorkModelError> {
    let text = text.to_ascii_lowercase();
    let any = |hints: &[&str]| hints.iter().any(|hint| text.contains(hint));
    if any(&[
        "usage limit",
        "limit reached",
        "hit your limit",
        "rate limit",
        "rate_limit",
        "resource_exhausted",
        "resource exhausted",
        "quota",
        "out of credits",
        "too many requests",
    ]) {
        return Some(WorkModelError::RateLimited {
            retry_after_ms: None,
        });
    }
    if any(&[
        "not logged in",
        "/login",
        "please log in",
        "log in again",
        "sign in",
        "not signed in",
        "unauthenticated",
        "authentication",
        "oauth token",
        "invalid api key",
        "credentials",
    ]) {
        return Some(WorkModelError::MissingKey);
    }
    if any(&[
        "prompt is too long",
        "context length",
        "context window",
        "too many tokens",
    ]) {
        return Some(WorkModelError::ContextTooLong);
    }
    if any(&["overloaded", "unavailable", "529", "503"]) {
        return Some(WorkModelError::Overloaded);
    }
    if any(&[
        "not recognized as a known model",
        "invalid model",
        "unknown agent",
    ]) {
        return Some(WorkModelError::BadRequest);
    }
    None
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn json_object(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    serde_json::from_str::<Value>(text)
        .ok()
        .or_else(|| {
            text.lines()
                .rev()
                .find_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        })
        .filter(Value::is_object)
}

fn failure(text: &str, stderr: &[u8]) -> WorkModelError {
    classify(text)
        .or_else(|| classify(&String::from_utf8_lossy(stderr)))
        .unwrap_or(WorkModelError::Protocol)
}

/// Claude Code's `--output-format json` result.
pub(crate) fn claude_result(
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(Value, WorkModelUsage), WorkModelError> {
    let Some(result) = json_object(stdout) else {
        return Err(failure("", stderr));
    };
    let text = result.get("result").and_then(Value::as_str).unwrap_or("");
    if result.get("is_error") == Some(&Value::Bool(true))
        || result.get("subtype").and_then(Value::as_str) != Some("success")
    {
        let subtype = result.get("subtype").and_then(Value::as_str).unwrap_or("");
        return Err(failure(&format!("{subtype} {text}"), stderr));
    }
    let usage = result.get("usage").cloned().unwrap_or(Value::Null);
    let fresh = number(&usage, "input_tokens");
    let written = number(&usage, "cache_creation_input_tokens");
    let read = number(&usage, "cache_read_input_tokens");
    let usage = WorkModelUsage {
        input_tokens: fresh.saturating_add(written).saturating_add(read),
        cached_input_tokens: read,
        cache_write_input_tokens: written,
        output_tokens: number(&usage, "output_tokens"),
        reasoning_tokens: 0,
        // Runs on the person's subscription: no charge to account for.
        cost_micros: Some(0),
    };
    let structured = match result.get("structured_output") {
        Some(value) if value.is_object() => value.clone(),
        // An older app may put the JSON answer in its text instead.
        _ => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .ok_or(WorkModelError::Protocol)?,
    };
    Ok((structured, usage))
}

/// Antigravity's stream-json `result` event.
pub(crate) fn agy_result(
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(Value, WorkModelUsage), WorkModelError> {
    let text = std::str::from_utf8(stdout).map_err(|_| WorkModelError::Protocol)?;
    let result = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(|event| event.get("event").and_then(Value::as_str) == Some("result"))
        .and_then(|event| event.get("result").cloned())
        .or_else(|| json_object(stdout).filter(|value| value.get("status").is_some()));
    let Some(result) = result else {
        return Err(failure("", stderr));
    };
    if result.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        let error = result.get("error").and_then(Value::as_str).unwrap_or("");
        return Err(
            match result.get("finished_reason").and_then(Value::as_str) {
                Some("print_timeout") => WorkModelError::Network,
                _ => failure(error, stderr),
            },
        );
    }
    let usage = result.get("usage").cloned().unwrap_or(Value::Null);
    let input = number(&usage, "input_tokens");
    let usage = WorkModelUsage {
        input_tokens: input,
        cached_input_tokens: number(&usage, "cache_read_tokens").min(input),
        cache_write_input_tokens: 0,
        output_tokens: number(&usage, "output_tokens"),
        reasoning_tokens: number(&usage, "thinking_tokens"),
        cost_micros: Some(0),
    };
    let structured = result
        .get("structured_output")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or(WorkModelError::Protocol)?;
    Ok((structured, usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::tests::request;
    use zephium_core::work::model::WorkModelToolChoice;

    #[test]
    fn claude_runs_without_tools_settings_or_prompt_in_argv() {
        let mut request = request(&["search"], WorkModelToolChoice::Auto);
        request.reasoning = Some(WorkModelReasoning::High);
        let args: Vec<String> = claude_args(&request, Path::new("/tmp/system.md"), "{}")
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let after = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .map(|index| args[index + 1].clone())
        };
        assert_eq!(args[0], "-p");
        assert_eq!(after("--tools").as_deref(), Some(""));
        assert_eq!(after("--setting-sources").as_deref(), Some(""));
        assert_eq!(after("--model").as_deref(), Some("sonnet"));
        assert_eq!(after("--effort").as_deref(), Some("high"));
        assert!(args.contains(&"--strict-mcp-config".into()));
        assert!(args.contains(&"--no-session-persistence".into()));
        assert!(!args.contains(&"--bare".into()));
        assert!(args.contains(&"Bash".into()) && args.contains(&"WebFetch".into()));
        assert!(!args
            .iter()
            .any(|arg| arg.contains("Hello") || arg.contains("Be brief")));
        let mut haiku = request.clone();
        haiku.model.model = "haiku".into();
        assert!(!claude_args(&haiku, Path::new("/s"), "{}").contains(&"--effort".into()));
    }

    #[test]
    fn agy_runs_as_the_toolless_agent_with_denying_settings() {
        let request = request(&["search"], WorkModelToolChoice::Auto);
        let args: Vec<String> = agy_args(&request, "{}", Duration::from_secs(230))
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let after = |flag: &str| {
            args.iter()
                .position(|arg| arg == flag)
                .map(|index| args[index + 1].clone())
        };
        assert_eq!(after("--agent").as_deref(), Some(AGY_AGENT));
        assert_eq!(after("--input-format").as_deref(), Some("stream-json"));
        assert_eq!(after("--print-timeout").as_deref(), Some("225s"));
        assert!(args.contains(&"--sandbox".into()));
        assert!(!args.iter().any(|arg| arg == "-p" || arg.contains("Hello")));
        let settings: Value = serde_json::from_str(AGY_SETTINGS).unwrap();
        assert!(settings["permissions"]["deny"]
            .as_array()
            .unwrap()
            .contains(&json!("command(*)")));
        assert!(AGY_AGENT_FILE.contains("tools: [finish]"));
    }

    #[test]
    fn results_and_failures_map_without_app_text() {
        let ok = br#"{"type":"result","subtype":"success","is_error":false,"result":"","structured_output":{"text":"hi","calls":[]},"usage":{"input_tokens":5,"cache_creation_input_tokens":7,"cache_read_input_tokens":100,"output_tokens":9},"total_cost_usd":0.2}"#;
        let (structured, usage) = claude_result(ok, b"").unwrap();
        assert_eq!(structured["text"], "hi");
        assert_eq!(
            (
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.cache_write_input_tokens,
                usage.output_tokens,
                usage.cost_micros
            ),
            (112, 100, 7, 9, Some(0))
        );
        let limit = br#"{"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|1760000000"}"#;
        assert!(matches!(
            claude_result(limit, b"").err(),
            Some(WorkModelError::RateLimited { .. })
        ));
        let login = br#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in - Please run /login"}"#;
        assert_eq!(
            claude_result(login, b"").err(),
            Some(WorkModelError::MissingKey)
        );
        assert_eq!(
            claude_result(b"garbage", b"").err(),
            Some(WorkModelError::Protocol)
        );
        assert_eq!(
            claude_result(b"", b"Error: Invalid API key").err(),
            Some(WorkModelError::MissingKey)
        );
        let retries =
            br#"{"type":"result","subtype":"error_max_structured_output_retries","is_error":true}"#;
        assert_eq!(
            claude_result(retries, b"").err(),
            Some(WorkModelError::Protocol)
        );
        let agy = b"{\"event\":\"init\"}\n{\"event\":\"result\",\"result\":{\"status\":\"SUCCESS\",\"structured_output\":{\"text\":\"ok\"},\"usage\":{\"input_tokens\":100,\"output_tokens\":30,\"thinking_tokens\":20,\"cache_read_tokens\":60}}}\n";
        let (structured, usage) = agy_result(agy, b"").unwrap();
        assert_eq!(structured["text"], "ok");
        assert_eq!(
            (
                usage.input_tokens,
                usage.cached_input_tokens,
                usage.reasoning_tokens
            ),
            (100, 60, 20)
        );
        let quota = br#"{"event":"result","result":{"status":"ERROR","error":"RESOURCE_EXHAUSTED: quota"}}"#;
        assert!(matches!(
            agy_result(quota, b"").err(),
            Some(WorkModelError::RateLimited { .. })
        ));
        let timeout =
            br#"{"event":"result","result":{"status":"ERROR","finished_reason":"print_timeout"}}"#;
        assert_eq!(
            agy_result(timeout, b"").err(),
            Some(WorkModelError::Network)
        );
        assert_eq!(classify("something odd"), None);
    }
}
