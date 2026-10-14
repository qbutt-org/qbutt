//! Public two-phase orchestration through owned app-server processes; no paid AI.
//! The recording renderer checks the typed boundary. Real PNG geometry belongs to Qt QA.

use std::future::Future;
use std::io::Write;
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
use qbutt_content::translation::manga_imagegen::{cache_key, translate_page};
use qbutt_content::translation::page::{
    PageJob, PageRegion, PageRenderer, PageWindow, RegionDiagnostic, RenderStatus, RenderedPage,
    RenderedWindow,
};
use qbutt_content::{Chapter, ChapterRef, PublicationRef, Source, VariantRef, VariantSelector};
use serde_json::Value;

#[path = "manga_imagegen/host.rs"]
mod host;
#[path = "manga_imagegen/long.rs"]
mod long;
#[path = "manga_imagegen/submission.rs"]
mod submission;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ORIGINAL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAFUlEQVR4nGP89eEZAwMDEwMDA4ICADXYAtaW6UM3AAAAAElFTkSuQmCC";
const NEIGHBOR: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAADCAIAAAA2iEnWAAAAFElEQVR4nGPkCtjCwMDAxMDAgKAAE7QBFLr5udsAAAAASUVORK5CYII=";
const SOURCE_TEXT: &str = "レンは姉です。";
const NEIGHBOR_ID: &str = "33333333-3333-3333-3333-333333333333";

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--prepare-ahead") {
        if args.len() != 4 {
            return Err("pass an authored original PNG and isolated profile directory".into());
        }
        let original = std::fs::read(&args[2])?;
        for name in ["first", "second"] {
            let path = Path::new(&args[3]).join(format!("{name}.cbz"));
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?;
            let mut archive = zip::ZipWriter::new(file);
            for page in 1..=5 {
                archive.start_file(
                    format!("{name}/page{page}.png"),
                    zip::write::SimpleFileOptions::default(),
                )?;
                archive.write_all(&original)?;
            }
            archive.finish()?;
        }
        return Ok(());
    }
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

struct ContractRenderer {
    crops: AtomicUsize,
    compositions: AtomicUsize,
    prepares: AtomicUsize,
    renders: AtomicUsize,
    cancellation_observed: AtomicBool,
    case: String,
}

impl ContractRenderer {
    fn diagnostic(&self, region: &PageRegion, stage: &str) -> RegionDiagnostic {
        RegionDiagnostic {
            id: region.id.clone(),
            status: if self.case == format!("{stage}-incomplete")
                || (stage == "prepare" && region.layout.width < 0.02)
            {
                RenderStatus::Overflow
            } else {
                RenderStatus::Rendered
            },
            font_pixels: 24,
            layout_lines: 2,
            cleanup_pixels: 2,
        }
    }

    fn wait_for_cancel(&self, cancelled: Arc<AtomicBool>) -> qbutt_codex::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancelled.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return Err(CodexError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.cancellation_observed.store(true, Ordering::Release);
        Err(CodexError::Cancelled)
    }
}

impl PageRenderer for ContractRenderer {
    fn dimensions(
        &self,
        _original: &InputImage,
        _cancelled: Arc<AtomicBool>,
    ) -> qbutt_codex::Result<(u32, u32)> {
        if _original.bytes == image(NEIGHBOR).bytes {
            return Ok(match self.case.as_str() {
                "long-context-oversized" => (800, 20_000),
                "long-context-budget" => (4_000, 4_000),
                _ => (2, 3),
            });
        }
        Ok(if self.case.starts_with("long-") {
            (800, 20_000)
        } else {
            (2, 3)
        })
    }
    fn crop(
        &self,
        _original: &InputImage,
        _window: PageWindow,
        _cancelled: Arc<AtomicBool>,
    ) -> qbutt_codex::Result<InputImage> {
        assert!(self.case.starts_with("long-"));
        self.crops.fetch_add(1, Ordering::Release);
        assert_eq!(_window.width, 800);
        assert!(matches!(
            (_window.top, _window.height),
            (0, 16000) | (13952, 6048)
        ));
        if matches!(self.case.as_str(), "long-cancel-crop" | "long-abort-crop")
            || (self.case == "long-cancel-second-crop" && _window.top > 0)
        {
            self.wait_for_cancel(_cancelled)?;
        }
        Ok(_original.clone())
    }
    fn compose(
        &self,
        _original: &InputImage,
        _windows: &[RenderedWindow],
        _cancelled: Arc<AtomicBool>,
    ) -> qbutt_codex::Result<Vec<u8>> {
        assert!(self.case.starts_with("long-"));
        self.compositions.fetch_add(1, Ordering::Release);
        if matches!(self.case.as_str(), "long-cancel-final" | "long-abort-final") {
            self.wait_for_cancel(_cancelled)?;
        }
        if self.case == "long-final-rejected" {
            return Err(CodexError::Image);
        }
        if self.case == "long-final-oversized" {
            return Ok(vec![0; qbutt_codex::MAX_GENERATED_IMAGE_BYTES + 1]);
        }
        assert!(!_windows.is_empty());
        Ok(_original.bytes.clone())
    }
    fn prepare(
        &self,
        original: &InputImage,
        regions: &[PageRegion],
        cancelled: Arc<AtomicBool>,
    ) -> qbutt_codex::Result<Vec<RegionDiagnostic>> {
        assert_eq!(original.bytes, image(ORIGINAL).bytes);
        assert!(regions.len() == 1 || self.case.starts_with("long-"));
        assert_eq!(regions[0].text_color, "#314455");
        assert_eq!(regions[0].outline_color.as_deref(), Some("#ffffff"));
        assert_eq!(regions[0].outline_width, 4.0);
        assert!(regions[0].translation[1].italic);
        assert!(!regions[0].translation[1].bold);
        assert!(regions[0].translation[3].italic);
        assert!(regions[0].translation[3].bold);
        self.prepares.fetch_add(1, Ordering::Release);
        if self.case == "prepare-rejected" {
            return Err(CodexError::Image);
        }
        if matches!(self.case.as_str(), "cancel-prepare" | "abort-prepare") {
            self.wait_for_cancel(cancelled)?;
        }
        Ok(regions
            .iter()
            .map(|region| self.diagnostic(region, "prepare"))
            .collect())
    }

    fn render(
        &self,
        original: &InputImage,
        cleaned: &[u8],
        regions: &[PageRegion],
        cancelled: Arc<AtomicBool>,
    ) -> qbutt_codex::Result<RenderedPage> {
        assert_eq!(original.bytes, image(ORIGINAL).bytes);
        assert_eq!(cleaned, original.bytes);
        self.renders.fetch_add(1, Ordering::Release);
        if self.case == "renderer-rejected" {
            return Err(CodexError::Image);
        }
        if matches!(self.case.as_str(), "cancel-compose" | "abort-compose") {
            self.wait_for_cancel(cancelled)?;
        }
        Ok(RenderedPage {
            png: cleaned.to_vec(),
            diagnostics: regions
                .iter()
                .map(|region| self.diagnostic(region, "render"))
                .collect(),
        })
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
    let key = cache_key(
        chapter.clone(),
        0,
        &original,
        "ru",
        "gpt-6.1-sol",
        "low",
        false,
    );
    assert_eq!(key.provider, "codex-imagegen");
    assert_eq!(key.pipeline, 3);
    let mut previous_pipeline = key.clone();
    previous_pipeline.pipeline = 2;
    assert_ne!(key, previous_pipeline);
    for case in [
        "ru",
        "en",
        "partial-usage",
        "retry-geometry",
        "retry-third",
        "retry-typography",
        "retry-context",
        "invalid-geometry",
        "invalid-typography",
        "invalid-context",
        "submission-limit",
        "final-bypass",
        "exhaustion-bypass",
        "final-rejected-after-acceptance",
        "resubmit-accepted",
        "invalid-term",
        "invalid-quote",
        "invalid-runs-empty",
        "invalid-runs-count",
        "invalid-runs-text",
        "invalid-runs-bytes",
        "invalid-runs-flags",
        "invalid-runs-plain",
        "metadata-rejected",
        "invalid-layout",
        "invalid-cleanup",
        "invalid-color",
        "invalid-outline-negative",
        "invalid-outline-range",
        "invalid-outline-color",
        "invalid-outline-zero",
        "prepare-rejected",
        "prepare-incomplete",
        "cancel-prepare",
        "renderer-rejected",
        "render-incomplete",
        "cancel-compose",
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
        let renderer = Arc::new(ContractRenderer {
            crops: AtomicUsize::new(0),
            compositions: AtomicUsize::new(0),
            prepares: AtomicUsize::new(0),
            renders: AtomicUsize::new(0),
            cancellation_observed: AtomicBool::new(false),
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
            renderer.clone(),
            &mut context,
            cancelled,
        );
        let control = async {
            if case == "cancel-prepare" || case == "cancel-compose" {
                let deadline = Instant::now() + Duration::from_secs(5);
                while (if case == "cancel-prepare" {
                    &renderer.prepares
                } else {
                    &renderer.renders
                })
                .load(Ordering::Acquire)
                    == 0
                {
                    if Instant::now() >= deadline {
                        return Err("native operation did not start".into());
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
            (
                "ru" | "en" | "partial-usage" | "retry-geometry" | "retry-third"
                | "retry-typography" | "retry-context" | "resubmit-accepted",
                Ok(page),
            ) => {
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
            (failed, Err(CodexError::Tool(_)))
                if failed.starts_with("invalid-")
                    || matches!(
                        failed,
                        "metadata-rejected"
                            | "prepare-incomplete"
                            | "render-incomplete"
                            | "prepare-rejected"
                            | "submission-limit"
                            | "final-bypass"
                            | "exhaustion-bypass"
                            | "final-rejected-after-acceptance"
                    ) => {}
            ("renderer-rejected", Err(CodexError::Image))
            | ("cancel-prepare" | "cancel-compose", Err(CodexError::Cancelled)) => {}
            (_, Err(error)) => return Err(format!("{case}: unexpected {error}").into()),
            (_, Ok(_)) => return Err(format!("{case}: invalid page accepted").into()),
        }
        let prepares = match case {
            "invalid-typography" | "prepare-rejected" | "prepare-incomplete" => 3,
            "retry-typography" => 2,
            value if value.starts_with("invalid-") => 0,
            "submission-limit" | "final-bypass" | "exhaustion-bypass" => 0,
            _ => 1,
        };
        let generated = prepares > 0
            && !matches!(
                case,
                "prepare-rejected"
                    | "prepare-incomplete"
                    | "cancel-prepare"
                    | "invalid-typography"
                    | "final-rejected-after-acceptance"
            );
        let rendered = generated && case != "metadata-rejected";
        assert_eq!(
            renderer.prepares.load(Ordering::Acquire),
            prepares,
            "{case}"
        );
        assert_eq!(
            renderer.renders.load(Ordering::Acquire),
            usize::from(rendered)
        );
        let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
        let trace: Vec<Value> = trace
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let processes: Vec<_> = trace
            .iter()
            .filter(|entry| entry.get("pid").is_some())
            .collect();
        assert_eq!(processes.len(), if generated { 2 } else { 1 });
        assert_eq!(processes[0]["phase"], "translation");
        for process in &processes {
            assert!(process_exited(
                process["pid"].as_u64().ok_or("missing PID")? as u32
            ));
        }
        let submissions: Vec<_> = trace
            .iter()
            .filter(|entry| entry["event"] == "page_submission")
            .collect();
        let expected_submissions = match case {
            "final-bypass" | "cancel-prepare" => 0,
            "submission-limit" => 4,
            "retry-geometry" | "retry-typography" | "retry-context" | "resubmit-accepted" => 2,
            "prepare-rejected" | "prepare-incomplete" | "exhaustion-bypass" | "retry-third" => 3,
            value if value.starts_with("invalid-") => 3,
            _ => 1,
        };
        assert_eq!(submissions.len(), expected_submissions, "{case}");
        if case.starts_with("retry-") {
            assert_eq!(submissions[0]["accepted"], false);
            assert_eq!(submissions.last().unwrap()["accepted"], true);
            let feedback = submissions[0]["feedback"].as_str().unwrap();
            let expected = match case {
                "retry-geometry" | "retry-third" => "cleanup rectangle",
                "retry-typography" => "Overflow",
                _ => "Context evidence",
            };
            assert!(feedback.contains(expected), "{case}: {feedback}");
        }
        if case == "submission-limit" {
            assert_eq!(submissions[3]["accepted"], false);
            assert!(
                submissions[3]["feedback"]
                    .as_str()
                    .unwrap()
                    .contains("three page submissions")
            );
        }
        let starts = trace
            .iter()
            .filter(|entry| entry["event"] == "image_generation_started")
            .count();
        assert_eq!(starts, usize::from(generated));
        println!(
            "PASS manga_imagegen case={case} imagegen_starts={starts} native_prepares={} native_renders={} processes_reaped={}",
            prepares,
            usize::from(rendered),
            processes.len()
        );
    }
    for case in ["abort-prepare", "abort-compose"] {
        exercise_abort(native, &chapter, &chapters, case).await?;
    }
    long::exercise(native, &chapter, &chapters).await?;
    Ok(())
}

async fn exercise_abort(
    native: &Path,
    chapter: &ChapterRef,
    chapters: &[Chapter],
    case: &str,
) -> Result<()> {
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
    let chapter = chapter.clone();
    let chapters = chapters.to_vec();
    let native_renderer = renderer.clone();
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        let base = TranslationContext::default();
        let source = ImageSource;
        let mut context = WorkContextTools::new(
            &chapter,
            &chapters,
            &source,
            base.clone(),
            cancelled.clone(),
        )?;
        translate_page(
            PageJob {
                chapter,
                page: 0,
                original: image(ORIGINAL),
                target_language: "ru".into(),
                work_context: base,
                model: Some("gpt-6.1-sol".into()),
                effort: Some("low".into()),
            },
            native_renderer,
            &mut context,
            cancelled,
        )
        .await
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let started = match case {
        "abort-prepare" => &renderer.prepares,
        "long-abort-crop" => &renderer.crops,
        "long-abort-final" => &renderer.compositions,
        _ => &renderer.renders,
    };
    while started.load(Ordering::Acquire) == 0 {
        if Instant::now() >= deadline {
            task.abort();
            return Err("abort fixture native operation did not start".into());
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    // Match the old controller race: watch is set, then its owner aborts before
    // native_work can poll the changed receiver on this current-thread runtime.
    cancel.send(true)?;
    task.abort();
    assert!(task.await.is_err_and(|error| error.is_cancelled()));
    let deadline = Instant::now() + Duration::from_secs(1);
    while !renderer.cancellation_observed.load(Ordering::Acquire) {
        if Instant::now() >= deadline {
            return Err(format!("{case}: native work ignored owner abort").into());
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let trace = std::fs::read_to_string(native.join("trace.jsonl"))?;
    for line in trace.lines() {
        let entry: Value = serde_json::from_str(line)?;
        if let Some(pid) = entry["pid"].as_u64() {
            assert!(process_exited(pid as u32));
        }
    }
    println!("PASS manga_imagegen case={case} native_cancel_observed=true processes_reaped=true");
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
