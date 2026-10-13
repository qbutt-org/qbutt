//! Public two-phase orchestration through owned app-server processes; no paid AI.
//! The validator records the typed boundary. Real PNG geometry belongs to Qt QA.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use qbutt_codex::{CodexError, InputImage};
use qbutt_content::translation::context::TranslationContext;
use qbutt_content::translation::context_tools::{
    ChapterExcerpt, WorkChapterSource, WorkContextTools,
};
use qbutt_content::translation::manga::PageJob;
use qbutt_content::translation::manga_imagegen::{
    GeneratedPageValidator, cache_key, translate_page,
};
use qbutt_content::{Chapter, ChapterRef, PublicationRef, Source, VariantRef, VariantSelector};
use serde_json::Value;

#[path = "manga_imagegen/host.rs"]
mod host;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ORIGINAL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAFUlEQVR4nGP89eEZAwMDEwMDA4ICADXYAtaW6UM3AAAAAElFTkSuQmCC";
const NEIGHBOR: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAFElEQVR4nGPkCtjCwMDAxMDAgKAAE7QBFLr5udsAAAAASUVORK5CYII=";
const SOURCE_TEXT: &str = "レンは姉です。";
const NEIGHBOR_ID: &str = "33333333-3333-3333-3333-333333333333";

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "app-server") {
        return host::run();
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let profile = std::env::temp_dir().join(format!("qbutt-manga-imagegen-{nonce}"));
    let native = profile
        .join("nodejs/node_modules/@openai/codex/node_modules")
        .join("@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin");
    std::fs::create_dir_all(&native)?;
    std::fs::copy(std::env::current_exe()?, native.join("codex.exe"))?;
    let names = ["ProgramFiles", "LOCALAPPDATA", "CODEX_HOME"];
    let saved = names.map(std::env::var_os);
    // Only this owned single-threaded fixture process changes its locator.
    unsafe {
        for name in names {
            std::env::set_var(name, &profile);
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(exercise(&native));
    drop(runtime);
    unsafe {
        for (name, value) in names.into_iter().zip(saved) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    std::fs::remove_dir_all(&profile)?;
    result
}

struct ImageSource;

impl WorkChapterSource for ImageSource {
    fn load(
        &self,
        chapter: ChapterRef,
        offset: usize,
    ) -> Pin<Box<dyn Future<Output = qbutt_codex::Result<ChapterExcerpt>> + Send + '_>> {
        Box::pin(async move {
            Ok(ChapterExcerpt {
                chapter,
                offset,
                total_parts: 1,
                parts: vec![],
                image: Some(image(NEIGHBOR)),
            })
        })
    }
}

struct ContractValidator {
    calls: AtomicUsize,
    case: String,
}

impl GeneratedPageValidator for ContractValidator {
    fn normalize(
        &self,
        original: &InputImage,
        png: &[u8],
        cancelled: Arc<AtomicBool>,
    ) -> qbutt_codex::Result<Vec<u8>> {
        assert_eq!(original.bytes, image(ORIGINAL).bytes);
        assert_eq!(png, original.bytes);
        self.calls.fetch_add(1, Ordering::Release);
        if self.case == "validator-rejected" {
            return Err(CodexError::Image);
        }
        if self.case == "cancel-normalize" {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cancelled.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    return Err(CodexError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            return Err(CodexError::Cancelled);
        }
        Ok(png.to_vec())
    }
}

async fn exercise(native: &Path) -> Result<()> {
    let chapter = ChapterRef {
        variant: VariantRef {
            publication: PublicationRef {
                source: Source::MangaDex,
                id: "11111111-1111-1111-1111-111111111111".into(),
            },
            selector: VariantSelector::MangaTranslation {
                language: "ja".into(),
                groups: vec![],
            },
        },
        id: "22222222-2222-2222-2222-222222222222".into(),
    };
    let mut neighbor = chapter.clone();
    neighbor.id = NEIGHBOR_ID.into();
    let chapters: Vec<_> = [chapter.clone(), neighbor]
        .into_iter()
        .map(|reference| Chapter {
            reference,
            title: None,
            volume: None,
            number: None,
            pages: 1,
        })
        .collect();
    let original = image(ORIGINAL);
    let key = cache_key(chapter.clone(), 0, &original, "ru", "gpt-6.1-sol", "low");
    assert_eq!(key.provider, "codex-imagegen");
    assert_eq!(key.pipeline, 1);
    assert_ne!(
        key,
        qbutt_content::translation::manga::cache_key(
            chapter.clone(),
            0,
            &original,
            "ru",
            "gpt-6.1-sol",
            "low"
        )
    );
    for case in [
        "ru",
        "en",
        "partial-usage",
        "invalid-term",
        "invalid-quote",
        "metadata-rejected",
        "validator-rejected",
        "cancel-normalize",
    ] {
        std::fs::write(native.join("case.txt"), case)?;
        std::fs::write(native.join("trace.jsonl"), "")?;
        let language = if case == "en" { "en" } else { "ru" };
        let base = TranslationContext::default();
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let mut context = WorkContextTools::new(
            &chapter,
            &chapters,
            &ImageSource,
            base.clone(),
            cancelled.clone(),
        )?;
        let validator = Arc::new(ContractValidator {
            calls: AtomicUsize::new(0),
            case: case.into(),
        });
        let work = translate_page(
            PageJob {
                chapter: chapter.clone(),
                page: 0,
                original: original.clone(),
                target_language: language.into(),
                work_context: base.clone(),
                model: Some("gpt-6.1-sol".into()),
                effort: Some("low".into()),
            },
            validator.clone(),
            &mut context,
            cancelled,
        );
        let control = async {
            if case == "cancel-normalize" {
                let deadline = Instant::now() + Duration::from_secs(5);
                while validator.calls.load(Ordering::Acquire) == 0 {
                    if Instant::now() >= deadline {
                        return Err("normalization did not start".into());
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                cancel.send(true)?;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (result, controlled) = tokio::join!(work, control);
        controlled?;
        match (case, result) {
            ("ru" | "en" | "partial-usage", Ok(page)) => {
                assert_eq!(page.png, original.bytes);
                assert_eq!(page.model, "gpt-6.1-sol");
                assert_eq!(page.effort, "low");
                if case == "partial-usage" {
                    assert!(page.usage.is_none());
                } else {
                    let usage = page.usage.ok_or("combined usage missing")?;
                    assert_eq!(usage.total_tokens, 300);
                    assert_eq!(usage.input_tokens, 180);
                    assert_eq!(usage.cached_input_tokens, 30);
                    assert_eq!(usage.cache_write_input_tokens, 15);
                    assert_eq!(usage.output_tokens, 120);
                    assert_eq!(usage.reasoning_output_tokens, 60);
                }
                assert_eq!(page.additions.terms.len(), 1);
                assert_eq!(page.additions.terms[0].source, "レン");
                assert_eq!(
                    page.additions.terms[0].target,
                    if language == "en" { "Ren" } else { "Рен" }
                );
                assert_eq!(page.additions.facts.len(), 1);
                assert_eq!(page.additions.facts[0].subject, "レン");
                assert_eq!(page.additions.facts[0].attribute, "gender");
                assert_eq!(page.additions.facts[0].value, "female");
                assert_eq!(page.evidence.len(), 1);
                assert_eq!(page.evidence[0].text, SOURCE_TEXT);
                let observed = page.evidence[0].image_observation.as_ref().unwrap();
                assert_eq!(observed.original_digest, key.original_digest);
                assert_eq!(observed.bounds, [100_000, 100_000, 900_000, 900_000]);
                let mut accepted = base.clone();
                accepted.apply(&chapter.variant.publication, page.additions, &page.evidence)?;
                assert_eq!(accepted.revision, 1);
            }
            ("metadata-rejected" | "invalid-term" | "invalid-quote", Err(CodexError::Tool(_)))
            | ("validator-rejected", Err(CodexError::Image))
            | ("cancel-normalize", Err(CodexError::Cancelled)) => {}
            (_, Err(error)) => return Err(format!("{case}: unexpected {error}").into()),
            (_, Ok(_)) => return Err(format!("{case}: invalid page accepted").into()),
        }
        let called = matches!(
            case,
            "ru" | "en" | "partial-usage" | "validator-rejected" | "cancel-normalize"
        );
        assert_eq!(validator.calls.load(Ordering::Acquire), usize::from(called));
        let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
        let trace: Vec<Value> = trace
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let processes: Vec<_> = trace
            .iter()
            .filter(|entry| entry.get("pid").is_some())
            .collect();
        let generated = !matches!(case, "invalid-term" | "invalid-quote");
        assert_eq!(processes.len(), if generated { 2 } else { 1 });
        assert_eq!(processes[0]["phase"], "translation");
        for process in &processes {
            assert!(process_exited(
                process["pid"].as_u64().ok_or("missing PID")? as u32
            ));
        }
        let starts = trace
            .iter()
            .filter(|entry| entry["event"] == "image_generation_started")
            .count();
        assert_eq!(starts, usize::from(generated));
        println!(
            "PASS manga_imagegen case={case} imagegen_starts={starts} validator_calls={} processes_reaped={}",
            usize::from(called),
            processes.len()
        );
    }
    Ok(())
}

fn image(encoded: &str) -> InputImage {
    InputImage::png(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap(),
    )
}

#[cfg(windows)]
fn process_exited(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return true;
        }
        let mut code = STILL_ACTIVE as u32;
        let read = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        read != 0 && code != STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
fn process_exited(_: u32) -> bool {
    true
}
