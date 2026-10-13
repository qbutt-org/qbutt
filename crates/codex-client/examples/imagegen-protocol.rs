//! Process acceptance of the public Codex APIs against an owned mock CLI.
//! No installed account or model request is used in this fixture.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use qbutt_codex::{
    CodexError, CodexRequest, InputImage, MAX_GENERATED_IMAGE_BYTES, ModelModality, NoTools,
    complete, generate_image,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "app-server") {
        return host();
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-codex-protocol-{nonce}"));
    let native = profile
        .join("nodejs/node_modules/@openai/codex/node_modules")
        .join("@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin");
    std::fs::create_dir_all(&native)?;
    std::fs::copy(std::env::current_exe()?, native.join("codex.exe"))?;
    let original_program_files = std::env::var_os("ProgramFiles");
    let original_local_app_data = std::env::var_os("LOCALAPPDATA");
    // The fixture owns this single-threaded process before its Tokio runtime.
    unsafe {
        std::env::set_var("ProgramFiles", &profile);
        std::env::set_var("LOCALAPPDATA", &profile);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(exercise(&native));
    drop(runtime);
    unsafe {
        match original_program_files {
            Some(value) => std::env::set_var("ProgramFiles", value),
            None => std::env::remove_var("ProgramFiles"),
        }
        match original_local_app_data {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
    }
    std::fs::remove_dir_all(&profile)?;
    result
}

fn request() -> CodexRequest {
    CodexRequest {
        instructions: "Generated isolated protocol acceptance.".into(),
        prompt: "Generated legal one-pixel image.".into(),
        modality: ModelModality::Image,
        image: Some(InputImage::png(
            base64::engine::general_purpose::STANDARD
                .decode(PNG)
                .unwrap(),
        )),
        output_schema: json!({
            "type": "object",
            "properties": {
                "accepted": {
                    "type": "boolean"
                }
            },
            "required": ["accepted"],
            "additionalProperties": false
        }),
        model: Some("gpt-6.1-sol".into()),
        effort: Some("low".into()),
        tools: vec![],
    }
}

async fn exercise(native: &Path) -> Result<()> {
    let started = Instant::now();
    std::fs::write(native.join("case.txt"), "json-default")?;
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let structured = complete(request(), &mut NoTools, cancelled).await?;
    assert_eq!(structured.value, json!({"accepted": true}));

    for case in [
        "valid",
        "wrong-thread",
        "wrong-turn",
        "wrong-id",
        "missing-start",
        "double-start",
        "double-complete",
        "failed",
        "bad-base64",
        "bad-png",
        "large-image",
        "large-line",
        "malformed-final",
        "missing-image",
        "capability-off",
        "setup-timeout",
        "cancel",
    ] {
        std::fs::write(native.join("case.txt"), case)?;
        std::fs::write(native.join("trace.jsonl"), "")?;
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let mut tools = NoTools;
        let work = generate_image(request(), &mut tools, cancelled);
        let control = async {
            if case == "cancel" {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
                    if trace.contains("turn/start") {
                        cancel.send(true)?;
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err("fixture did not start its cancellable turn".into());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (result, controlled) = tokio::join!(work, control);
        controlled?;
        match (case, result) {
            ("valid", Ok(output)) => {
                assert_eq!(
                    output.png,
                    base64::engine::general_purpose::STANDARD.decode(PNG)?
                );
                assert_eq!(output.value, json!({"accepted": true}));
                assert_eq!(output.model, "gpt-6.1-sol");
                assert_eq!(output.effort, "low");
            }
            ("wrong-thread" | "wrong-turn", Err(CodexError::Isolation))
            | (
                "wrong-id" | "missing-start" | "double-start" | "double-complete"
                | "malformed-final",
                Err(CodexError::Protocol),
            )
            | ("failed", Err(CodexError::Provider))
            | ("bad-base64" | "bad-png" | "large-image", Err(CodexError::Image))
            | ("large-line", Err(CodexError::Protocol))
            | ("missing-image" | "capability-off", Err(CodexError::ImageGenerationUnavailable))
            | ("setup-timeout", Err(CodexError::Timeout))
            | ("cancel", Err(CodexError::Cancelled)) => {}
            (_, Err(error)) => return Err(format!("{case}: unexpected {error}").into()),
            (_, Ok(_)) => return Err(format!("{case}: invalid image job was accepted").into()),
        }
        let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
        if case == "cancel" {
            assert!(trace.contains("turn/interrupt"));
        }
        let first: Value =
            serde_json::from_str(trace.lines().next().ok_or("host trace is empty")?)?;
        assert!(process_exited(
            first["pid"].as_u64().ok_or("host PID missing")? as u32
        ));
        println!("PASS imagegen_protocol case={case} process_reaped=true");
    }
    // Structured consumers must keep the built-in disabled and reject its events.
    std::fs::write(native.join("case.txt"), "json-image-event")?;
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    assert!(matches!(
        complete(request(), &mut NoTools, cancelled).await,
        Err(CodexError::Isolation)
    ));
    println!(
        "PASS imagegen_protocol json_default_disabled saved_path_ignored metadata_atomic elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}

fn host() -> Result<()> {
    let native = std::env::current_exe()?
        .parent()
        .ok_or("host directory missing")?
        .to_owned();
    let case = std::fs::read_to_string(native.join("case.txt"))?;
    let mut trace = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(native.join("trace.jsonl"))?;
    writeln!(trace, "{}", json!({"pid": std::process::id()}))?;
    let args: Vec<_> = std::env::args().collect();
    let mut features = BTreeMap::new();
    for pair in args.windows(2) {
        if pair[0] == "-c"
            && let Some((key, value)) = pair[1].split_once('=')
            && let Some(name) = key.strip_prefix("features.")
        {
            features.insert(name.to_owned(), value == "true");
        }
    }
    assert_eq!(features["image_generation"], !case.starts_with("json-"));
    for (name, value) in &features {
        assert!(!value || name == "image_generation");
    }
    let mut output = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let method = request["method"].as_str().ok_or("request method missing")?;
        writeln!(trace, "{}", json!({"method": method}))?;
        trace.flush()?;
        let id = &request["id"];
        match method {
            "initialize" if case == "setup-timeout" => {}
            "initialize" => send(&mut output, json!({"id": id, "result": {}}))?,
            "initialized" => {}
            "config/read" => send(
                &mut output,
                json!({
                    "id": id,
                    "result": {
                        "config": {
                            "features": features,
                            "web_search": "disabled",
                            "notify": [],
                            "mcp_servers": {}
                        }
                    }
                }),
            )?,
            "account/read" => send(
                &mut output,
                json!({
                    "id": id,
                    "result": {
                        "account": {"type": "chatgpt"}
                    }
                }),
            )?,
            "modelProvider/capabilities/read" => send(
                &mut output,
                json!({
                    "id": id,
                    "result": {
                        "imageGeneration": case != "capability-off"
                    }
                }),
            )?,
            "model/list" => send(
                &mut output,
                json!({
                    "id": id,
                    "result": {
                        "data": [{
                            "model": "gpt-6.1-sol",
                            "inputModalities": ["text", "image"],
                            "supportedReasoningEfforts": [{"reasoningEffort": "low"}],
                            "defaultReasoningEffort": "low",
                            "isDefault": true
                        }],
                        "nextCursor": null
                    }
                }),
            )?,
            "thread/start" => {
                assert_eq!(request["params"]["sandbox"], "read-only");
                assert_eq!(request["params"]["approvalPolicy"], "never");
                assert_eq!(request["params"]["ephemeral"], true);
                assert_eq!(request["params"]["selectedCapabilityRoots"], json!([]));
                assert_eq!(request["params"]["environments"], json!([]));
                send(
                    &mut output,
                    json!({
                        "id": id,
                        "result": {
                            "thread": {
                                "id": "fixture-thread",
                                "ephemeral": true,
                                "path": null
                            }
                        }
                    }),
                )?;
            }
            "turn/start" => {
                assert_eq!(
                    request["params"]["sandboxPolicy"],
                    json!({"type": "readOnly", "networkAccess": false})
                );
                send(
                    &mut output,
                    json!({
                        "id": id,
                        "result": {
                            "turn": {
                                "id": "fixture-turn",
                                "status": "inProgress"
                            }
                        }
                    }),
                )?;
                if case == "cancel" {
                    continue;
                }
                turn(&mut output, &case)?;
            }
            "turn/interrupt" => send(
                &mut output,
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "fixture-thread",
                        "turn": {
                            "id": "fixture-turn",
                            "status": "interrupted"
                        }
                    }
                }),
            )?,
            _ => return Err(format!("unexpected method {method}").into()),
        }
    }
    Ok(())
}

fn turn(output: &mut impl Write, case: &str) -> Result<()> {
    if case == "large-line" {
        writeln!(output, "{}", "x".repeat(33 * 1024 * 1024))?;
        output.flush()?;
        return Ok(());
    }
    let scope = |item| {
        json!({
            "threadId": if case == "wrong-thread" { "foreign" } else { "fixture-thread" },
            "turnId": if case == "wrong-turn" { "foreign" } else { "fixture-turn" },
            "item": item
        })
    };
    let start = json!({
        "method": "item/started",
        "params": scope(json!({
            "type": "imageGeneration",
            "id": "image-1",
            "status": "in_progress"
        }))
    });
    if case != "json-default" && case != "missing-image" {
        if case != "missing-start" {
            send(output, start.clone())?;
        }
        if case == "double-start" {
            send(output, start)?;
        }
        let mut encoded = PNG.to_owned();
        if case == "bad-base64" {
            encoded = "!not-base64!".into();
        }
        if case == "bad-png" {
            encoded = base64::engine::general_purpose::STANDARD.encode(b"not a PNG");
        }
        if case == "large-image" {
            encoded =
                base64::engine::general_purpose::STANDARD
                    .encode(vec![0; MAX_GENERATED_IMAGE_BYTES + 1]);
        }
        let done = json!({
            "method": "item/completed",
            "params": scope(json!({
                "type": "imageGeneration",
                "id": if case == "wrong-id" { "image-2" } else { "image-1" },
                "status": if case == "failed" { "failed" } else { "completed" },
                "result": encoded,
                "revisedPrompt": "fixture edit",
                "savedPath": "C:/never-read-this-path/generated.png"
            }))
        });
        send(output, done.clone())?;
        if case == "double-complete" {
            send(output, done)?;
        }
    }
    let text = if case == "malformed-final" {
        "not JSON"
    } else {
        "{\"accepted\":true}"
    };
    let agent = json!({"type": "agentMessage", "id": "agent-1", "text": text});
    send(
        output,
        json!({"method": "item/completed", "params": scope(agent.clone())}),
    )?;
    send(
        output,
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": "fixture-thread",
                "turn": {
                    "id": "fixture-turn",
                    "status": "completed",
                    "items": [agent]
                }
            }
        }),
    )?;
    Ok(())
}

fn send(output: &mut impl Write, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *output, &value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}

#[cfg(windows)]
fn process_exited(pid: u32) -> bool {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    unsafe {
        let handle = OpenProcess(0x1000, 0, pid);
        if handle.is_null() {
            return true;
        }
        let mut code = 259;
        let read = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        read != 0 && code != 259
    }
}

#[cfg(not(windows))]
fn process_exited(_pid: u32) -> bool {
    true
}
