//! Process acceptance of the public Codex APIs against an owned mock CLI.
//! No installed account or model request is used in this fixture.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
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
    let original_codex_home = std::env::var_os("CODEX_HOME");
    // The fixture owns this single-threaded process before its Tokio runtime.
    unsafe {
        std::env::set_var("ProgramFiles", &profile);
        std::env::set_var("LOCALAPPDATA", &profile);
        std::env::set_var("CODEX_HOME", &profile);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(exercise(&native, &profile));
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
        match original_codex_home {
            Some(value) => std::env::set_var("CODEX_HOME", value),
            None => std::env::remove_var("CODEX_HOME"),
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

async fn exercise(native: &Path, profile: &Path) -> Result<()> {
    let png = base64::engine::general_purpose::STANDARD.decode(PNG)?;
    let images = profile.join("generated_images");
    let preserved = images.join("user-existing/image-1.png");
    std::fs::create_dir_all(preserved.parent().unwrap())?;
    std::fs::write(&preserved, &png)?;
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
        "metadata-rejected",
        "cancel-after-image",
        "locked-swap",
        "preexisting-thread",
        "foreign-saved-path",
        "file-symlink",
        "directory-symlink",
        "hardlink",
        "different-bytes",
        "locked-file",
        "cleanup-failure",
        "late-tool",
        "late-tool-after-image",
    ] {
        std::fs::write(native.join("case.txt"), case)?;
        std::fs::write(native.join("trace.jsonl"), "")?;
        let artifact = images.join(format!("fixture-{case}/image-1.png"));
        if case == "preexisting-thread" {
            std::fs::create_dir_all(artifact.parent().unwrap())?;
            std::fs::write(&artifact, &png)?;
        }
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let mut tools = NoTools;
        let work = generate_image(request(), &mut tools, cancelled);
        let control = async {
            if matches!(case, "cancel" | "cancel-after-image" | "locked-swap") {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
                    let ready = if case == "cancel" {
                        trace.contains("turn/start")
                    } else {
                        artifact_is_locked(&artifact)
                    };
                    if ready {
                        if case == "locked-swap" {
                            assert!(
                                std::fs::rename(&artifact, images.join("replaced.png")).is_err()
                            );
                            assert!(
                                std::fs::rename(
                                    artifact.parent().unwrap(),
                                    images.join("moved-thread")
                                )
                                .is_err()
                            );
                        }
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
            ("valid" | "metadata-rejected", Ok(output)) => {
                assert_eq!(
                    output.png,
                    base64::engine::general_purpose::STANDARD.decode(PNG)?
                );
                assert_eq!(
                    output.value,
                    json!({"accepted": case != "metadata-rejected"})
                );
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
            | ("cancel" | "cancel-after-image" | "locked-swap", Err(CodexError::Cancelled)) => {}
            (
                "cleanup-failure",
                Err(CodexError::ArtifactCleanup {
                    primary: Some(primary),
                    reason,
                }),
            ) => {
                assert!(matches!(*primary, CodexError::Protocol));
                assert!(reason.contains("original request failed"));
            }
            (
                "preexisting-thread" | "foreign-saved-path" | "file-symlink" | "directory-symlink"
                | "hardlink" | "different-bytes" | "locked-file",
                Err(CodexError::ArtifactCleanup {
                    primary: None,
                    reason,
                }),
            ) => assert!(!reason.is_empty()),
            ("late-tool" | "late-tool-after-image", Err(CodexError::Tool(reason))) => {
                assert!(reason.contains("before image generation starts"));
            }
            (_, Err(error)) => return Err(format!("{case}: unexpected {error}").into()),
            (_, Ok(_)) => return Err(format!("{case}: invalid image job was accepted").into()),
        }
        let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
        if matches!(
            case,
            "cancel" | "cancel-after-image" | "locked-swap" | "late-tool" | "late-tool-after-image"
        ) {
            assert!(trace.contains("turn/interrupt"));
        }
        assert_eq!(std::fs::read(&preserved)?, png);
        if matches!(
            case,
            "valid"
                | "metadata-rejected"
                | "malformed-final"
                | "cancel-after-image"
                | "locked-swap"
                | "late-tool-after-image"
        ) {
            assert!(!artifact.exists(), "{case}: completed PNG leaked");
            assert!(
                artifact.parent().unwrap().is_dir(),
                "thread directories must be preserved"
            );
        }
        if matches!(
            case,
            "preexisting-thread"
                | "foreign-saved-path"
                | "file-symlink"
                | "directory-symlink"
                | "hardlink"
                | "different-bytes"
                | "locked-file"
                | "cleanup-failure"
        ) {
            assert!(artifact.exists(), "{case}: unsafe artifact was removed");
        }
        if case == "preexisting-thread" {
            assert!(!trace.contains("turn/start"));
            assert_eq!(std::fs::read(&artifact)?, png);
        }
        if case == "cleanup-failure" {
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    FILE_ATTRIBUTE_NORMAL, SetFileAttributesW,
                };
                let path: Vec<_> = artifact.as_os_str().encode_wide().chain(Some(0)).collect();
                if unsafe { SetFileAttributesW(path.as_ptr(), FILE_ATTRIBUTE_NORMAL) } == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
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
        "PASS imagegen_protocol json_default_disabled owned_artifact_cleanup metadata_atomic elapsed_ms={}",
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
    let thread = format!("fixture-{case}");
    let codex_home = Path::new(&std::env::var_os("CODEX_HOME").ok_or("mock CODEX_HOME missing")?)
        .canonicalize()?;
    let mut artifact_lock = None;
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
                                "id": thread,
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
                turn(&mut output, &case, &thread, &codex_home, &mut artifact_lock)?;
            }
            "turn/interrupt" => send(
                &mut output,
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": thread,
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

fn turn(
    output: &mut impl Write,
    case: &str,
    thread: &str,
    codex_home: &Path,
    artifact_lock: &mut Option<std::fs::File>,
) -> Result<()> {
    if case == "large-line" {
        writeln!(output, "{}", "x".repeat(33 * 1024 * 1024))?;
        output.flush()?;
        return Ok(());
    }
    let scope = |item| {
        json!({
            "threadId": if case == "wrong-thread" { "foreign" } else { thread },
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
        if case == "late-tool" {
            send(
                output,
                json!({
                    "id": 901,
                    "method": "item/tool/call",
                    "params": {
                        "threadId": thread,
                        "turnId": "fixture-turn",
                        "namespace": null,
                        "tool": "prepare_translation",
                        "arguments": {}
                    }
                }),
            )?;
            return Ok(());
        }
        let saved = if matches!(
            case,
            "valid"
                | "metadata-rejected"
                | "malformed-final"
                | "cancel-after-image"
                | "locked-swap"
                | "foreign-saved-path"
                | "file-symlink"
                | "directory-symlink"
                | "hardlink"
                | "different-bytes"
                | "locked-file"
                | "cleanup-failure"
                | "late-tool-after-image"
        ) {
            let path = codex_home
                .join("generated_images")
                .join(thread)
                .join("image-1.png");
            let preserved = codex_home.join("generated_images/user-existing/image-1.png");
            let png = base64::engine::general_purpose::STANDARD.decode(PNG)?;
            if case == "directory-symlink" {
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(
                    preserved.parent().unwrap(),
                    path.parent().unwrap(),
                )?;
            } else {
                std::fs::create_dir_all(path.parent().unwrap())?;
                match case {
                    "file-symlink" => {
                        #[cfg(windows)]
                        std::os::windows::fs::symlink_file(&preserved, &path)?;
                    }
                    "hardlink" => std::fs::hard_link(&preserved, &path)?,
                    "different-bytes" => {
                        let mut different = png.clone();
                        *different.last_mut().unwrap() ^= 1;
                        std::fs::write(&path, different)?;
                    }
                    _ => std::fs::write(&path, png)?,
                }
            }
            if case == "locked-file" {
                #[cfg(windows)]
                {
                    *artifact_lock = Some(
                        std::fs::OpenOptions::new()
                            .read(true)
                            .share_mode(1)
                            .open(&path)?,
                    );
                }
            }
            if case == "cleanup-failure" {
                let mut permissions = std::fs::metadata(&path)?.permissions();
                permissions.set_readonly(true);
                std::fs::set_permissions(&path, permissions)?;
            }
            Some(if case == "foreign-saved-path" {
                preserved
            } else {
                path
            })
        } else {
            None
        };
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
                "savedPath": saved
            }))
        });
        send(output, done.clone())?;
        if case == "double-complete" {
            send(output, done)?;
        }
        if matches!(case, "cancel-after-image" | "locked-swap") {
            return Ok(());
        }
        if case == "late-tool-after-image" {
            send(
                output,
                json!({
                    "id": 902,
                    "method": "item/tool/call",
                    "params": {
                        "threadId": thread,
                        "turnId": "fixture-turn",
                        "namespace": null,
                        "tool": "prepare_translation",
                        "arguments": {}
                    }
                }),
            )?;
            return Ok(());
        }
    }
    let text = if matches!(case, "malformed-final" | "cleanup-failure") {
        "not JSON"
    } else if case == "metadata-rejected" {
        "{\"accepted\":false}"
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
                "threadId": thread,
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
fn artifact_is_locked(path: &Path) -> bool {
    let result = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(windows_sys::Win32::Storage::FileSystem::DELETE)
        .share_mode(7)
        .open(path);
    matches!(result, Err(error) if error.raw_os_error() == Some(32))
}

#[cfg(not(windows))]
fn artifact_is_locked(_: &Path) -> bool {
    false
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
