//! Purpose-built app-server peer for the public two-phase page fixture.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use serde_json::{Value, json};

use super::{NEIGHBOR, NEIGHBOR_ID, ORIGINAL, Result, SOURCE_TEXT};

pub fn run() -> Result<()> {
    let native = std::env::current_exe()?
        .parent()
        .ok_or("host path missing")?
        .to_owned();
    let case = std::fs::read_to_string(native.join("case.txt"))?;
    let native_fixture = case == "native";
    let fixture_directory = if native_fixture {
        Some(std::path::PathBuf::from(
            std::fs::read_to_string(native.join("fixture-directory.txt"))?.trim(),
        ))
    } else {
        None
    };
    let mut trace = std::fs::OpenOptions::new()
        .append(true)
        .open(native.join("trace.jsonl"))?;
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
    let image_phase = features["image_generation"];
    let phase = if image_phase { "image" } else { "translation" };
    let thread = format!("page-fixture-{case}-{phase}");
    writeln!(
        trace,
        "{}",
        json!({"pid": std::process::id(), "phase": phase})
    )?;
    assert!(
        features
            .iter()
            .all(|(name, enabled)| !enabled || name == "image_generation")
    );
    let mut output = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let id = &request["id"];

        if request.get("method").is_none() {
            assert!(!image_phase);
            let items = &request["result"]["contentItems"];
            let images: Vec<_> = items
                .as_array()
                .ok_or("tool content missing")?
                .iter()
                .filter(|item| item["type"] == "inputImage")
                .collect();
            match id.as_u64().ok_or("tool reply ID missing")? {
                901 => {
                    assert_eq!(request["result"]["success"], true);
                    assert_eq!(images.len(), 1);
                    assert_eq!(
                        images[0]["imageUrl"],
                        format!("data:image/png;base64,{NEIGHBOR}")
                    );
                    finish(&mut output, &thread, proposal(&case), Some(100))?;
                }
                _ => return Err("unexpected tool reply".into()),
            }
            continue;
        }

        let method = request["method"].as_str().ok_or("method missing")?;
        writeln!(trace, "{}", json!({"method": method}))?;
        trace.flush()?;
        match method {
            "initialize" => send(&mut output, json!({"id": id, "result": {}}))?,
            "initialized" => {}
            "config/read" => send(
                &mut output,
                json!({
                    "id": id,
                    "result": {"config": {
                        "features": features,
                        "web_search": "disabled",
                        "notify": [],
                        "mcp_servers": {},
                    }},
                }),
            )?,
            "account/read" => send(
                &mut output,
                json!({"id": id, "result": {"account": {"type": "chatgpt"}}}),
            )?,
            "modelProvider/capabilities/read" => send(
                &mut output,
                json!({"id": id, "result": {"imageGeneration": true}}),
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
                            "isDefault": true,
                        }],
                        "nextCursor": null,
                    },
                }),
            )?,
            "thread/start" => {
                assert_eq!(request["params"]["sandbox"], "read-only");
                assert_eq!(request["params"]["approvalPolicy"], "never");
                assert_eq!(request["params"]["ephemeral"], true);
                let names: Vec<_> = request["params"]["dynamicTools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tool| tool["name"].as_str().unwrap())
                    .collect();
                assert_eq!(request["params"]["model"], "gpt-6.1-sol");
                if image_phase {
                    assert!(names.is_empty());
                } else {
                    assert_eq!(names, ["list_chapters", "get_chapter", "read_context"]);
                }
                send(
                    &mut output,
                    json!({
                        "id": id,
                        "result": {"thread": {
                            "id": thread,
                            "ephemeral": true,
                            "path": null,
                        }},
                    }),
                )?;
            }
            "turn/start" => {
                assert_eq!(
                    request["params"]["sandboxPolicy"],
                    json!({"type": "readOnly", "networkAccess": false})
                );
                assert_eq!(request["params"]["effort"], "low");
                let input = request["params"]["input"].as_array().unwrap();
                assert_eq!(input.len(), 2);
                if !native_fixture {
                    assert_eq!(input[1]["url"], format!("data:image/png;base64,{ORIGINAL}"));
                }
                let prompt: Value = serde_json::from_str(input[0]["text"].as_str().unwrap())?;
                send(
                    &mut output,
                    json!({
                        "id": id,
                        "result": {"turn": {"id": "fixture-turn", "status": "inProgress"}},
                    }),
                )?;
                if native_fixture {
                    let directory = fixture_directory.as_ref().unwrap();
                    if image_phase {
                        use base64::Engine;
                        let bytes = std::fs::read(directory.join("generated-cleanup.png"))?;
                        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                        generate_png(&mut output, &thread, &encoded)?;
                        writeln!(trace, "{}", json!({"event":"image_generation_started"}))?;
                        finish(
                            &mut output,
                            &thread,
                            json!({"accepted":true,"reason":""}),
                            Some(200),
                        )?;
                    } else {
                        let proposal: Value = serde_json::from_slice(&std::fs::read(
                            directory.join("proposal.json"),
                        )?)?;
                        finish(&mut output, &thread, proposal, Some(100))?;
                    }
                    trace.flush()?;
                } else if image_phase {
                    assert!(!case.starts_with("invalid-"));
                    let cleanup = &prompt["cleanupRegions"][0];
                    assert_eq!(cleanup["sourceText"], SOURCE_TEXT);
                    assert_eq!(
                        cleanup["cleanupRects"],
                        json!([{"x":0.2,"y":0.2,"width":0.3,"height":0.3}])
                    );
                    assert!(cleanup.get("translation").is_none());
                    assert!(cleanup.get("family").is_none());
                    assert!(prompt.get("targetLanguage").is_none());
                    let instructions = request["params"]["input"][0]["text"].as_str().unwrap();
                    assert!(!instructions.contains("reader") && !instructions.contains("читатель"));
                    assert_eq!(
                        request["params"]["outputSchema"]["required"],
                        json!(["accepted", "reason"])
                    );
                    writeln!(trace, "{}", json!({"event": "image_generation_started"}))?;
                    trace.flush()?;
                    generate(&mut output, &thread)?;
                    let accepted = case != "metadata-rejected";
                    finish(
                        &mut output,
                        &thread,
                        json!({
                            "accepted": accepted,
                            "reason": if accepted {""} else {"Fixture cleanup is incomplete"},
                        }),
                        if case == "partial-usage" {
                            None
                        } else {
                            Some(200)
                        },
                    )?;
                } else {
                    assert_eq!(
                        request["params"]["outputSchema"]["required"],
                        json!(["regions", "terms", "facts"])
                    );
                    let schema = &request["params"]["outputSchema"]["properties"]["regions"]["items"]
                        ["properties"]["translation"];
                    assert_eq!(schema["type"], "array");
                    assert_eq!(schema["minItems"], 1);
                    assert_eq!(schema["maxItems"], 32);
                    assert_eq!(
                        schema["items"]["required"],
                        json!(["text", "bold", "italic"])
                    );
                    send(
                        &mut output,
                        json!({
                            "id": 901,
                            "method": "item/tool/call",
                            "params": {
                                "threadId": thread,
                                "turnId": "fixture-turn",
                                "namespace": null,
                                "tool": "get_chapter",
                                "arguments": {"chapter_id": NEIGHBOR_ID, "offset": 0},
                            },
                        }),
                    )?;
                }
            }
            _ => return Err(format!("unexpected method {method}").into()),
        }
    }
    Ok(())
}

fn proposal(case: &str) -> Value {
    let mut proposal = json!({
        "regions": [{
            "id": "r1",
            "sourceText": SOURCE_TEXT,
            "translation": translation(case),
            "family":"condensed",
            "angle":12,
            "textColor":"#314455",
            "outlineColor":"#ffffff",
            "outlineWidth":4,
            "bounds": {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8},
            "layout": {"x":0.2,"y":0.2,"width":0.6,"height":0.6},
            "cleanupRects":[{"x":0.2,"y":0.2,"width":0.3,"height":0.3}],
        }],
        "terms": [{
            "source": if case == "invalid-term" {"ミナ"} else {"レン"},
            "target": if case == "en" {"Ren"} else {"Рен"},
            "forms": [],
            "evidence": {
                "region_id": "r1",
                "quote": if case == "invalid-quote" {"レンは妹です。"} else {"レン"},
            },
        }],
        "facts": [{
            "subject": "レン",
            "attribute": "gender",
            "value": "female",
            "evidence": {"region_id": "r1", "quote": SOURCE_TEXT},
        }],
    });
    let region = &mut proposal["regions"][0];
    match case {
        "invalid-layout" => region["layout"]["x"] = json!(0.9),
        "invalid-cleanup" => region["cleanupRects"] = json!([]),
        "invalid-color" => region["textColor"] = json!("#12zz33"),
        "invalid-outline-negative" => region["outlineWidth"] = json!(-1),
        "invalid-outline-range" => region["outlineWidth"] = json!(17),
        "invalid-outline-color" => region["outlineColor"] = Value::Null,
        "invalid-outline-zero" => region["outlineWidth"] = json!(0),
        _ => {}
    }
    proposal
}

fn translation(case: &str) -> Value {
    match case {
        "invalid-runs-empty" => json!([]),
        "invalid-runs-count" => json!(vec![
            json!({"text": "x", "bold": false, "italic": false});
            33
        ]),
        "invalid-runs-text" => json!([{"text": "", "bold": false, "italic": false}]),
        "invalid-runs-bytes" => {
            json!([{"text": "文".repeat(1001), "bold": false, "italic": false}])
        }
        "invalid-runs-flags" => json!([{"text": "Ren", "bold": false}]),
        "invalid-runs-plain" => json!("Ren is my elder sister."),
        "en" => json!([
            {"text": "Ren is my elder sister. ", "bold": false, "italic": false},
            {"text": "DOKJA", "bold": false, "italic": true},
            {"text": " means ", "bold": false, "italic": false},
            {"text": "reader", "bold": true, "italic": true},
            {"text": ".", "bold": false, "italic": false},
        ]),
        _ => json!([
            {"text": "Рен — моя старшая сестра. ", "bold": false, "italic": false},
            {"text": "докча", "bold": false, "italic": true},
            {"text": " означает ", "bold": false, "italic": false},
            {"text": "читатель", "bold": true, "italic": true},
            {"text": ".", "bold": false, "italic": false},
        ]),
    }
}

fn generate(output: &mut impl Write, thread: &str) -> Result<()> {
    generate_png(output, thread, ORIGINAL)
}

fn generate_png(output: &mut impl Write, thread: &str, encoded: &str) -> Result<()> {
    send(
        output,
        json!({
            "method": "item/started",
            "params": {
                "threadId": thread,
                "turnId": "fixture-turn",
                "item": {"type": "imageGeneration", "id": "image-1", "status": "in_progress"},
            },
        }),
    )?;
    send(
        output,
        json!({
            "method": "item/completed",
            "params": {
                "threadId": thread,
                "turnId": "fixture-turn",
                "item": {
                    "type": "imageGeneration",
                    "id": "image-1",
                    "status": "completed",
                    "result": encoded,
                    "savedPath": null,
                },
            },
        }),
    )
}

fn finish(output: &mut impl Write, thread: &str, value: Value, usage: Option<u64>) -> Result<()> {
    if let Some(tokens) = usage {
        send(
            output,
            json!({
                "method": "thread/tokenUsage/updated",
                "params": {
                    "threadId": thread,
                    "turnId": "fixture-turn",
                    "tokenUsage": {"total": {
                        "totalTokens": tokens,
                        "inputTokens": tokens * 6 / 10,
                        "cachedInputTokens": tokens / 10,
                        "cacheWriteInputTokens": tokens / 20,
                        "outputTokens": tokens * 4 / 10,
                        "reasoningOutputTokens": tokens / 5,
                    }},
                },
            }),
        )?;
    }
    let agent = json!({
        "type": "agentMessage",
        "id": "agent-1",
        "text": value.to_string(),
    });
    send(
        output,
        json!({
            "method": "item/completed",
            "params": {"threadId": thread, "turnId": "fixture-turn", "item": agent},
        }),
    )?;
    send(
        output,
        json!({
            "method": "turn/completed",
            "params": {
                "threadId": thread,
                "turn": {"id": "fixture-turn", "status": "completed", "items": [agent]},
            },
        }),
    )
}

fn send(output: &mut impl Write, value: Value) -> Result<()> {
    serde_json::to_writer(&mut *output, &value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
