//! One real call through the installed, signed-in Claude Code. Ignored by
//! default: it uses the person's subscription. Run it on purpose with
//! `cargo test -p zephium-cli-models --test live_claude -- --ignored --nocapture`.

use std::sync::Mutex;
use std::time::Instant;

use serde_json::json;
use zephium_cli_models::{locate, CliApp, CliModelClient};
use zephium_core::work::model::{
    WorkModelClient, WorkModelEvent, WorkModelMessage, WorkModelPart, WorkModelRef,
    WorkModelRequest, WorkModelSystemBlock, WorkModelTool, WorkModelToolChoice, WorkModelWire,
};

#[tokio::test]
#[ignore = "live: runs the installed Claude Code on the person's subscription"]
async fn claude_code_returns_one_tool_call() {
    let program = locate(CliApp::ClaudeCode).expect("Claude Code is installed");
    let client = CliModelClient::new(CliApp::ClaudeCode, program).unwrap();
    let request = WorkModelRequest {
        image_fence: None,
        model: WorkModelRef {
            provider: CliApp::ClaudeCode.provider(),
            wire: WorkModelWire::Cli,
            model: "haiku".into(),
        },
        system: vec![WorkModelSystemBlock {
            text: "You help people find things on the web. Use the tools.".into(),
            cache: true,
        }],
        tools: vec![WorkModelTool {
            name: "open_page".into(),
            description: "Opens a web page in the browser.".into(),
            schema: json!({
                "type": "object",
                "properties": {"url": {"type": "string", "description": "An https URL."}},
                "required": ["url"],
                "additionalProperties": false,
            }),
            strict: true,
        }],
        messages: vec![WorkModelMessage::User(vec![WorkModelPart::Text(
            "Open the Rust homepage.".into(),
        )])],
        max_output_tokens: 1000,
        reasoning: None,
        native_search: false,
        parallel_tools: false,
        tool_choice: WorkModelToolChoice::Required,
        volatile_tail: 0,
        cache_key: None,
    };
    let seen = Mutex::new(Vec::new());
    let events = |event: WorkModelEvent| seen.lock().unwrap().push(event);
    let started = Instant::now();
    let outcome = client.call(request, &events).await.unwrap();
    println!("elapsed: {:.1}s", started.elapsed().as_secs_f64());
    println!("stop: {:?}, usage: {:?}", outcome.stop, outcome.usage);
    for part in &outcome.assistant {
        match part {
            WorkModelPart::Text(text) => println!("text: {text}"),
            WorkModelPart::ToolCall(call) => {
                println!("call: {} {} {}", call.id, call.name, call.arguments)
            }
            _ => {}
        }
    }
    assert!(outcome
        .assistant
        .iter()
        .any(|part| matches!(part, WorkModelPart::ToolCall(call) if call.name == "open_page")));
    assert!(!seen.lock().unwrap().is_empty());
}
