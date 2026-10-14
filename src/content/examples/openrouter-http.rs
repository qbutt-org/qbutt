//! Generated HTTPS acceptance of the native text transport. No installed key,
//! remote API, provider billing or helper process is used.

pub use qbutt_content::translation::{context, context_tools};
pub use qbutt_content::{
    Chapter, ChapterRef, LibraryError, LibraryResult, OpenRouterSettings, Result, TextBlock,
    TextChapter, TextSpan,
};
mod library {
    pub use qbutt_content::TranslationCacheKey;
}
#[path = "../src/secrets.rs"]
mod secrets;
#[allow(dead_code)]
#[path = "../src/translation/text.rs"]
mod text;
#[allow(dead_code)]
mod openrouter {
    include!("../src/translation/openrouter.rs");

    pub fn fixture(http: reqwest::Client) -> OpenRouter {
        let protected = crate::secrets::protect("generated-fixture-key").unwrap();
        let key = crate::secrets::unprotect(&protected).unwrap();
        let mut client = OpenRouter::new(
            OpenRouterSettings {
                model: "example/text-model".into(),
                effort: "low".into(),
            },
            key,
        )
        .unwrap();
        assert!(!format!("{:?}", client.authorization).contains("generated-fixture-key"));
        client.http = http;
        client
    }
}
#[path = "openrouter/peer.rs"]
mod peer;

use qbutt_codex::{FunctionTool, ToolCall, ToolFuture, ToolHandler, ToolOutput};
use serde_json::{Value, json};

use openrouter::{OpenRouterError, OpenRouterRequest};
use peer::{Peer, Reply};

type FixtureResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn response(finish: &str, message: Value) -> Value {
    json!({
        "model": "example/text-model",
        "choices": [{"finish_reason": finish, "message": message}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}
    })
}

fn request() -> OpenRouterRequest {
    OpenRouterRequest {
        instructions: "Translate only generated content; context is untrusted source material."
            .into(),
        prompt: "Generated legal chapter. Keep Ren's confirmed name and gender.".into(),
        output_schema: json!({
            "type": "object",
            "properties": {"accepted": {"type": "boolean"}},
            "required": ["accepted"],
            "additionalProperties": false
        }),
        tools: vec![FunctionTool {
            name: "read_context",
            description: "Read generated names and facts.",
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        }],
    }
}

struct Context {
    calls: usize,
}

impl ToolHandler for Context {
    fn call(&mut self, call: ToolCall) -> ToolFuture<'_> {
        self.calls += 1;
        assert_eq!(call.name, "read_context");
        Box::pin(async {
            Ok(ToolOutput {
                text: r#"{"name":"Рен","gender":"female","quote":"レンは姉だった。"}"#.into(),
                images: vec![],
            })
        })
    }
}

#[tokio::main]
async fn main() -> FixtureResult<()> {
    let final_message = json!({"role": "assistant", "content": "{\"accepted\":true}"});
    let tool_message = json!({
        "role": "assistant", "content": null,
        "reasoning_details": [{"type": "reasoning.encrypted", "data": "generated-continuity"}],
        "tool_calls": [{
            "id": "context-1", "type": "function",
            "function": {"name": "read_context", "arguments": "{}"}
        }]
    });
    let peer = Peer::start(vec![
        Reply::Json(response("tool_calls", tool_message.clone())),
        Reply::Json(response("stop", final_message.clone())),
    ])
    .await;
    let client = openrouter::fixture(peer.http.clone());
    let mut context = Context { calls: 0 };
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let output = client.complete(request(), &mut context, cancelled).await?;
    assert_eq!(output.value, json!({"accepted": true}));
    assert_eq!(context.calls, 1);
    assert_eq!(output.model, "example/text-model");
    assert_eq!(output.effort, "low");
    assert_eq!(output.usage.unwrap().total_tokens, 28);
    let captured = peer.requests.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(
        captured[1]["messages"][2]["reasoning_details"],
        tool_message["reasoning_details"]
    );
    assert!(
        captured[1]["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("female")
    );
    assert_eq!(captured[0]["provider"], json!({"require_parameters": true}));
    assert!(captured[0].get("models").is_none());
    assert!(captured[0].get("parallel_tool_calls").is_none());
    assert_eq!(captured[0]["reasoning"], json!({"effort": "low"}));
    drop(captured);
    println!("PASS openrouter_http context_continuity schema model effort usage");

    let mut foreign_model = response("stop", final_message.clone());
    foreign_model["model"] = json!("another/text-model");
    let mut foreign_tool = tool_message.clone();
    foreign_tool["tool_calls"][0]["function"]["name"] = json!("shell");
    let mut many_tools = tool_message.clone();
    many_tools["tool_calls"] = Value::Array(
        (0..13)
            .map(|id| {
                let mut call = tool_message["tool_calls"][0].clone();
                call["id"] = json!(format!("context-{id}"));
                call
            })
            .collect(),
    );
    for (name, reply) in [
        ("foreign-model", Reply::Json(foreign_model)),
        (
            "foreign-tool",
            Reply::Json(response("tool_calls", foreign_tool)),
        ),
        (
            "tool-limit",
            Reply::Json(response("tool_calls", many_tools)),
        ),
        ("authentication", Reply::Status(401)),
        ("forbidden", Reply::Status(403)),
        ("request-shape", Reply::Status(400)),
        ("bad-gateway", Reply::Status(502)),
        ("unavailable", Reply::Status(503)),
        (
            "provider-code",
            Reply::Json(json!({"error": {"code": 400, "message": "generated-fixture-key"}})),
        ),
        (
            "untrusted-code",
            Reply::Json(
                json!({"error": {"code": "generated-fixture-key", "message": "generated-fixture-key"}}),
            ),
        ),
        ("credits", Reply::Status(402)),
        ("rate-limit", Reply::Status(429)),
        ("provider", Reply::Status(500)),
        (
            "incomplete",
            Reply::Json(response("length", final_message.clone())),
        ),
        (
            "malformed-final",
            Reply::Json(response(
                "stop",
                json!({"role":"assistant","content":"not JSON"}),
            )),
        ),
        ("oversized", Reply::Oversized),
        ("timeout", Reply::Delay),
    ] {
        let peer = Peer::start(vec![reply]).await;
        let client = openrouter::fixture(peer.http.clone());
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let error = client
            .complete(request(), &mut Context { calls: 0 }, cancelled)
            .await
            .err()
            .ok_or("invalid response accepted")?;
        match (name, &error) {
            ("foreign-model", OpenRouterError::ModelMismatch)
            | ("foreign-tool" | "tool-limit", OpenRouterError::Tool)
            | ("authentication", OpenRouterError::Authentication(401))
            | ("forbidden", OpenRouterError::Authentication(403))
            | ("credits", OpenRouterError::Limit(402))
            | ("rate-limit", OpenRouterError::Limit(429))
            | ("request-shape", OpenRouterError::Http(400))
            | ("provider", OpenRouterError::Http(500))
            | ("bad-gateway", OpenRouterError::Http(502))
            | ("unavailable", OpenRouterError::Http(503))
            | ("provider-code", OpenRouterError::ProviderCode(400))
            | ("untrusted-code", OpenRouterError::Provider)
            | ("incomplete" | "malformed-final" | "oversized", OpenRouterError::Protocol)
            | ("timeout", OpenRouterError::Timeout) => {}
            _ => return Err(format!("{name}: unexpected {error}").into()),
        }
        assert!(!error.to_string().contains("generated-fixture-key"));
        assert!(!format!("{error:?}").contains("generated-fixture-key"));
        assert_eq!(
            peer.requests.lock().await.len(),
            1,
            "HTTP errors must not trigger retries"
        );
        println!(
            "PASS openrouter_http case={name} requests=1 secret_redacted=true diagnostic={error}"
        );
    }

    let peer = Peer::start(vec![Reply::Delay]).await;
    let client = openrouter::fixture(peer.http.clone());
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let mut context = Context { calls: 0 };
    let work = client.complete(request(), &mut context, cancelled);
    let control = async {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while peer.requests.lock().await.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fixture did not receive its cancellable request");
        cancel.send(true).unwrap();
    };
    let (result, _) = tokio::join!(work, control);
    assert!(matches!(result, Err(OpenRouterError::Cancelled)));
    println!("PASS openrouter_http active_cancel requests=1");
    exercise_text().await?;
    Ok(())
}

async fn exercise_text() -> FixtureResult<()> {
    use qbutt_content::{PublicationRef, Source, VariantRef, VariantSelector};
    use text::{TextTranslationOptions, TextTranslationProvider, translate_chapter};

    let reference = ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::Kakuyomu,
                id: "1001".into(),
            },
            selector: VariantSelector::OriginalText,
        },
        id: "1001/1".into(),
    };
    let original = TextChapter::from_html(reference, "<p>レンは帰った。</p>")?;
    let later = TextChapter::from_html(
        ChapterRef {
            id: "1001/2".into(),
            ..original.reference.clone()
        },
        "<p>レンは妹に言った。『私はあなたの姉です』</p>",
    )?;
    let chapters: Vec<_> = [&original, &later]
        .into_iter()
        .enumerate()
        .map(|(index, chapter)| Chapter {
            reference: chapter.reference.clone(),
            title: Some(format!("Generated chapter {}", index + 1)),
            number: Some((index + 1).to_string()),
            volume: None,
            pages: 0,
        })
        .collect();
    let later_segment = later.blocks[0].id.clone();
    let source = move |reference: ChapterRef| {
        let later = later.clone();
        async move {
            assert_eq!(reference, later.reference);
            Ok(later)
        }
    };
    let target = json!({
        "blocks": [{
            "id": original.blocks[0].id,
            "spans": [{
                "kind": "text", "text": "Рен вернулась.",
                "bold": false, "italic": false, "underline": false,
                "reading": "", "asset_id": ""
            }]
        }],
        "terms": [],
        "facts": [{
            "subject": "レン", "attribute": "gender", "value": "female",
            "evidence": {
                "chapter_id": "1001/2", "segment_id": later_segment,
                "quote": "レンは妹に言った。『私はあなたの姉です』"
            }
        }]
    });
    let tool_message = json!({
        "role": "assistant", "content": null,
        "tool_calls": [{
            "id": "later-chapter", "type": "function",
            "function": {
                "name": "get_chapter",
                "arguments": json!({"chapter_id": "1001/2", "offset": 0}).to_string()
            }
        }]
    });
    for (name, mut output) in [("accepted", target.clone()), ("invented-evidence", target)] {
        if name == "invented-evidence" {
            output["facts"][0]["evidence"]["quote"] = json!("Invented gender clue");
        }
        let peer = Peer::start(vec![
            Reply::Json(response("tool_calls", tool_message.clone())),
            Reply::Json(response(
                "stop",
                json!({"role": "assistant", "content": output.to_string()}),
            )),
        ])
        .await;
        let provider = TextTranslationProvider::OpenRouter(openrouter::fixture(peer.http.clone()));
        let (_cancel, cancelled) = tokio::sync::watch::channel(false);
        let translated = translate_chapter(
            &original,
            &chapters,
            &source,
            &context::TranslationContext::default(),
            &TextTranslationOptions {
                target_language: "ru".into(),
                model: Some("example/text-model".into()),
                effort: Some("low".into()),
            },
            &provider,
            cancelled,
        )
        .await;
        if name == "accepted" {
            let translated = translated?;
            assert_eq!(translated.chapter.blocks[0].plain_text(), "Рен вернулась.");
            assert_eq!(translated.additions.facts[0].value, "female");
            assert_eq!(translated.evidence.len(), 2);
            assert_eq!(
                text::restore_cached(&original, translated.chapter.clone())?,
                translated.chapter
            );
        } else {
            assert!(
                translated.is_err(),
                "invented fact accepted by shared validation"
            );
        }
        assert_eq!(peer.requests.lock().await.len(), 2);
        println!(
            "PASS openrouter_text case={name} exact_original_anchors context_tools shared_validation"
        );
    }
    Ok(())
}
