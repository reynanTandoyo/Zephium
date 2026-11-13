//! A stand-in for Claude Code and Antigravity that the crate's tests run.
//! It never contacts a model. Probe answers come from a `<exe>.mode` file next
//! to it; model-call answers from a marker in the prompt it reads on stdin.

use std::io::Read;

fn mode() -> String {
    std::env::current_exe()
        .ok()
        .map(|exe| {
            let mut path = exe.into_os_string();
            path.push(".mode");
            std::fs::read_to_string(path).unwrap_or_default()
        })
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn after(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

fn first_tool(schema: &str) -> String {
    serde_json::from_str::<serde_json::Value>(schema)
        .ok()
        .and_then(|schema| {
            schema["properties"]["calls"]["items"]["properties"]["name"]["enum"][0]
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = mode();
    match args.first().map(String::as_str) {
        Some("--version") => {
            if mode == "hang" {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            println!("9.8.7 (Fake)");
            return;
        }
        Some("auth") => {
            println!("{{\"loggedIn\": {}}}", mode != "signed_out");
            return;
        }
        Some("models") => {
            if mode == "signed_out" {
                eprintln!("You are not signed in. Run agy to sign in.");
                std::process::exit(1);
            }
            println!("Fetching available models...\nfake-model-high\tFake Model (High)\nfake-model-low\tFake Model (Low)");
            return;
        }
        _ => {}
    }
    let agy = args.iter().any(|arg| arg == "--input-format");
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let prompt = if agy {
        serde_json::from_str::<serde_json::Value>(input.lines().next().unwrap_or_default())
            .ok()
            .and_then(|line| line["message"]["content"].as_str().map(str::to_owned))
            .unwrap_or_default()
    } else {
        input.clone()
    };
    let schema = after(&args, "--json-schema").unwrap_or_default();
    let structured = if prompt.contains("FAKE_CALL") {
        serde_json::json!({"text": "Calling.", "calls": [{"name": first_tool(&schema), "arguments": {"q": "fake"}}]})
    } else if prompt.contains("FAKE_BADARGS") {
        serde_json::json!({"text": "", "calls": [{"name": first_tool(&schema), "arguments": "nope"}]})
    } else if prompt.contains("FAKE_ECHO") {
        let system = after(&args, "--system-prompt-file")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        let agent = std::fs::read_to_string(".agents/agents/zephium-model.md").unwrap_or_default();
        let env: Vec<String> = std::env::vars().map(|(name, _)| name).collect();
        let cwd = std::env::current_dir().unwrap_or_default();
        serde_json::json!({"text": serde_json::json!({
            "args": args, "system": system, "agent": agent, "env": env,
            "cwd": cwd, "prompt_in_argv": args.iter().any(|arg| arg.contains("FAKE_ECHO")),
        }).to_string()})
    } else if prompt.contains("FAKE_SLEEP") {
        std::thread::sleep(std::time::Duration::from_secs(30));
        serde_json::json!({"text": "late"})
    } else if prompt.contains("FAKE_HUGE") {
        let chunk = "x".repeat(1024 * 1024);
        for _ in 0..10 {
            print!("{chunk}");
        }
        return;
    } else if prompt.contains("FAKE_GARBAGE") {
        println!("this is not json");
        return;
    } else {
        serde_json::json!({"text": "Hello from the fake.", "calls": []})
    };
    let failure = if prompt.contains("FAKE_LIMIT") {
        Some("Claude AI usage limit reached|1760000000 RESOURCE_EXHAUSTED")
    } else if prompt.contains("FAKE_LOGIN") {
        Some("Not logged in. Please run /login")
    } else {
        None
    };
    if agy {
        let result = match failure {
            Some(error) => {
                serde_json::json!({"status": "ERROR", "error": error, "finished_reason": "error"})
            }
            None => serde_json::json!({
                "status": "SUCCESS", "structured_output": structured,
                "usage": {"input_tokens": 50, "output_tokens": 7, "thinking_tokens": 3, "cache_read_tokens": 20},
            }),
        };
        println!(
            "{}",
            serde_json::json!({"event": "init", "init": {"tools": []}})
        );
        println!(
            "{}",
            serde_json::json!({"event": "result", "result": result})
        );
    } else {
        let result = match failure {
            Some(error) => {
                serde_json::json!({"type": "result", "subtype": "success", "is_error": true, "result": error})
            }
            None => serde_json::json!({
                "type": "result", "subtype": "success", "is_error": false, "result": "",
                "structured_output": structured,
                "usage": {"input_tokens": 10, "cache_creation_input_tokens": 2, "cache_read_input_tokens": 30, "output_tokens": 4},
                "total_cost_usd": 0.01,
            }),
        };
        println!("{result}");
    }
    if failure.is_some() {
        std::process::exit(1);
    }
}
