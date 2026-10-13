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
                assert_eq!(input[1]["url"], format!("data:image/png;base64,{ORIGINAL}"));
                let prompt: Value = serde_json::from_str(input[0]["text"].as_str().unwrap())?;
                assert_eq!(
                    prompt["targetLanguage"],
                    if case == "en" { "en" } else { "ru" }
                );
                send(
                    &mut output,
                    json!({
                        "id": id,
                        "result": {"turn": {"id": "fixture-turn", "status": "inProgress"}},
                    }),
                )?;
                if image_phase {
                    assert!(!matches!(case.as_str(), "invalid-term" | "invalid-quote"));
                    assert_eq!(prompt["translations"][0]["translation"], translation(&case));
                    assert_eq!(prompt["translations"][0]["sourceText"], SOURCE_TEXT);
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
                            "reason": if accepted {""} else {"Fixture lettering is incomplete"},
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
    json!({
        "regions": [{
            "id": "r1",
            "sourceText": SOURCE_TEXT,
            "translation": translation(case),
            "bounds": {"x": 0.1, "y": 0.1, "width": 0.8, "height": 0.8},
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
    })
}

fn translation(case: &str) -> &'static str {
    if case == "en" {
        "Ren is my elder sister."
    } else {
        "Рен — моя старшая сестра."
    }
}

fn generate(output: &mut impl Write, thread: &str) -> Result<()> {
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
                    "result": ORIGINAL,
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
