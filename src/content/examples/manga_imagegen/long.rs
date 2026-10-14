//! Process-level seam, context and cancellation scenarios through the public API.

use super::*;

pub(super) async fn exercise(
    native: &Path,
    chapter: &ChapterRef,
    chapters: &[Chapter],
) -> Result<()> {
    let original = image(ORIGINAL);
    let key = cache_key(
        chapter.clone(),
        0,
        &original,
        "ru",
        "gpt-6.1-sol",
        "low",
        true,
    );
    assert_eq!(key.pipeline, 4);
    assert_ne!(
        key,
        cache_key(
            chapter.clone(),
            0,
            &original,
            "ru",
            "gpt-6.1-sol",
            "low",
            false
        )
    );
    for case in [
        "long-ru",
        "long-en",
        "long-empty-last",
        "long-empty-context",
        "long-oversized-crossing",
        "long-explicit-unsupported",
        "long-context-oversized",
        "long-context-budget",
        "long-preview-budget",
        "long-edge",
        "long-missing-crossing",
        "long-duplicate",
        "long-late-overlap",
        "long-split-crossing",
        "long-merged-accepted",
        "long-final-rejected",
        "long-final-oversized",
        "long-cancel-crop",
        "long-cancel-second-crop",
        "long-cancel-final",
    ] {
        std::fs::write(native.join("case.txt"), case)?;
        std::fs::write(native.join("trace.jsonl"), "")?;
        let renderer = Arc::new(ContractRenderer {
            crops: AtomicUsize::new(0),
            compositions: AtomicUsize::new(0),
            prepares: AtomicUsize::new(0),
            renders: AtomicUsize::new(0),
            cancellation_observed: AtomicBool::new(false),
            case: case.into(),
        });
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let mut context = WorkContextTools::new(
            chapter,
            chapters,
            &ImageSource,
            TranslationContext::default(),
            cancelled.clone(),
        )?;
        let work = translate_page(
            PageJob {
                chapter: chapter.clone(),
                page: 0,
                original: original.clone(),
                target_language: if case == "long-en" { "en" } else { "ru" }.into(),
                work_context: TranslationContext::default(),
                model: Some("gpt-6.1-sol".into()),
                effort: Some("low".into()),
            },
            renderer.clone(),
            &mut context,
            cancelled,
        );
        let control = async {
            if matches!(
                case,
                "long-cancel-crop" | "long-cancel-second-crop" | "long-cancel-final"
            ) {
                let started = if case != "long-cancel-final" {
                    &renderer.crops
                } else {
                    &renderer.compositions
                };
                let deadline = Instant::now() + Duration::from_secs(10);
                let count = if case == "long-cancel-second-crop" {
                    2
                } else {
                    1
                };
                while started.load(Ordering::Acquire) < count {
                    if Instant::now() >= deadline {
                        return Err("long native stage did not start".into());
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                cancel.send(true)?;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (result, controlled) = tokio::join!(work, control);
        controlled?;
        if !case.starts_with("long-cancel") {
            use qbutt_codex::{ToolCall, ToolHandler};
            let restored = context
                .call(ToolCall {
                    name: "read_context".into(),
                    arguments: serde_json::json!({"query":"レン"}),
                })
                .await?;
            let restored: Value = serde_json::from_str(&restored.text)?;
            assert!(restored["terms"].as_array().unwrap().is_empty());
        }
        match (case, result) {
            ("long-ru" | "long-en" | "long-empty-last", Ok(page)) => {
                assert_eq!(page.png, original.bytes);
                if case == "long-empty-last" {
                    assert_eq!(page.evidence.len(), 2);
                } else {
                    assert_eq!(page.evidence.len(), 3);
                    assert_eq!(page.evidence[2].id, "w1:crossing");
                    assert_eq!(page.evidence[2].text, "レンは姉です。続きも全部読みます。");
                }
                for segment in &page.evidence {
                    assert_eq!(
                        segment.image_observation.as_ref().unwrap().original_digest,
                        key.original_digest
                    );
                }
                if case != "long-empty-last" {
                    let crossing = page.evidence[2].image_observation.as_ref().unwrap();
                    assert_eq!(crossing.bounds, [100_000, 788_320, 900_000, 818_560]);
                }
                assert_eq!(page.additions.terms[0].evidence.segment_id, "w0:r1");
                let mut accepted = TranslationContext::default();
                accepted.apply(&chapter.variant.publication, page.additions, &page.evidence)?;
                assert_eq!(accepted.revision, 1);
                assert_eq!(
                    page.usage.unwrap().total_tokens,
                    if case == "long-empty-last" { 400 } else { 600 }
                );
                assert_eq!(renderer.crops.load(Ordering::Acquire), 2);
                assert_eq!(renderer.compositions.load(Ordering::Acquire), 1);
            }
            (
                "long-cancel-crop" | "long-cancel-second-crop" | "long-cancel-final",
                Err(CodexError::Cancelled),
            ) => {
                assert!(renderer.cancellation_observed.load(Ordering::Acquire));
            }
            ("long-final-rejected" | "long-final-oversized", Err(CodexError::Image)) => {}
            (_, Err(CodexError::Tool(_))) => {}
            (_, outcome) => {
                return Err(format!(
                    "{case}: unexpected outcome {}",
                    match outcome {
                        Ok(_) => "accepted".into(),
                        Err(error) => error.to_string(),
                    }
                )
                .into());
            }
        }
        let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
        let entries: Vec<Value> = trace
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let processes: Vec<_> = entries
            .iter()
            .filter(|entry| entry.get("pid").is_some())
            .collect();
        for process in &processes {
            assert!(process_exited(process["pid"].as_u64().unwrap() as u32));
        }
        let expected_processes = match case {
            "long-cancel-crop" => 0,
            "long-cancel-second-crop" => 2,
            "long-oversized-crossing"
            | "long-explicit-unsupported"
            | "long-context-oversized"
            | "long-context-budget"
            | "long-edge" => 1,
            "long-empty-last"
            | "long-empty-context"
            | "long-missing-crossing"
            | "long-duplicate"
            | "long-late-overlap"
            | "long-split-crossing"
            | "long-merged-accepted" => 3,
            "long-preview-budget" => 3,
            _ => 4,
        };
        assert_eq!(processes.len(), expected_processes);
        println!(
            "PASS manga_long case={case} processes_reaped={} crops={} final_compositions={}",
            processes.len(),
            renderer.crops.load(Ordering::Acquire),
            renderer.compositions.load(Ordering::Acquire)
        );
    }
    for case in ["long-abort-crop", "long-abort-final"] {
        super::exercise_abort(native, chapter, chapters, case).await?;
    }
    Ok(())
}
