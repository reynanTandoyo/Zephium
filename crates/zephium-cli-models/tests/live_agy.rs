//! One real call through the installed, signed-in Antigravity CLI (agy), shaped
//! like an Ask Zephium question: plain text, no tools. Ignored by
//! default: it uses the person's subscription. Run it on purpose with
//! `cargo test -p zephium-cli-models --test live_agy -- --ignored --nocapture`.

use std::sync::Mutex;
use std::time::Instant;

use zephium_cli_models::{locate, CliApp, CliModelClient};
use zephium_core::work::model::{
    WorkModelClient, WorkModelEvent, WorkModelMessage, WorkModelPart, WorkModelRef,
    WorkModelRequest, WorkModelSystemBlock, WorkModelToolChoice, WorkModelWire,
};

#[tokio::test]
#[ignore = "live: runs the installed Antigravity CLI on the person's account"]
async fn antigravity_answers_a_plain_question() {
    let program = locate(CliApp::Antigravity).expect("agy is installed");
    let client = CliModelClient::new(CliApp::Antigravity, program).unwrap();
    let request = WorkModelRequest {
        image_fence: None,
        model: WorkModelRef {
            provider: CliApp::Antigravity.provider(),
            wire: WorkModelWire::Cli,
            model: "gemini-4-hydrogen-medium".into(),
        },
        system: vec![WorkModelSystemBlock {
            text: "You help people find things on the web. Use the tools.".into(),
            cache: true,
        }],
        tools: Vec::new(),
        messages: vec![WorkModelMessage::User(vec![WorkModelPart::Text(
            "hi how are you".into(),
        )])],
        max_output_tokens: 1000,
        reasoning: Some(zephium_core::work::model::WorkModelReasoning::Medium),
        native_search: false,
        parallel_tools: false,
        tool_choice: WorkModelToolChoice::Auto,
        volatile_tail: 0,
        cache_key: None,
    };
    let seen = Mutex::new(Vec::new());
    let events = |event: WorkModelEvent| seen.lock().unwrap().push(event);
    let started = Instant::now();
    let outcome = client.call(request, &events).await;
    println!("elapsed: {:.1}s", started.elapsed().as_secs_f64());
    let outcome = outcome.unwrap();
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
    assert!(!seen.lock().unwrap().is_empty());
}
