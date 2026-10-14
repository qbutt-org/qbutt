use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use qbutt_codex::{self as codex, CodexError, InputImage};
use qbutt_content::translation::page::{
    FontFamily, PageRegion, PageRenderer, RegionDiagnostic, RenderStatus, RenderedPage,
};

pub struct RenderCancellation {
    flag: Arc<AtomicBool>,
}

fn render_cancelled(cancellation: &RenderCancellation) -> bool {
    cancellation.flag.load(Ordering::Acquire)
}

#[cxx::bridge(namespace = "Qbutt")]
mod ffi {
    enum MangaFontFamily {
        Comic,
        Serif,
        Condensed,
    }

    struct MangaTextRun {
        text: String,
        bold: bool,
        italic: bool,
    }
    struct MangaRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    struct MangaRegion {
        id: String,
        translation: Vec<MangaTextRun>,
        family: MangaFontFamily,
        angle: f64,
        text_color: String,
        outline_color: String,
        outline_width: f64,
        bounds: MangaRect,
        layout: MangaRect,
        cleanup_rects: Vec<MangaRect>,
    }
    struct MangaDiagnostic {
        region_id: String,
        status: u8,
        font_pixels: u32,
        layout_lines: u32,
        cleanup_pixels: u32,
    }
    struct MangaTypographyOutput {
        diagnostics: Vec<MangaDiagnostic>,
        error: String,
    }
    struct MangaRenderOutput {
        png: Vec<u8>,
        diagnostics: Vec<MangaDiagnostic>,
        error: String,
    }

    unsafe extern "C++" {
        include!("qbutt_quick/translation_render.h");
        fn prepare_manga_typography(
            original: &[u8],
            regions: &[MangaRegion],
            cancelled: &RenderCancellation,
        ) -> MangaTypographyOutput;
        fn render_cleaned_manga_page(
            original: &[u8],
            cleaned: &[u8],
            regions: &[MangaRegion],
            cancelled: &RenderCancellation,
        ) -> MangaRenderOutput;
    }
    extern "Rust" {
        type RenderCancellation;
        fn render_cancelled(cancelled: &RenderCancellation) -> bool;
    }
}

pub struct QtMangaRenderer;

impl PageRenderer for QtMangaRenderer {
    fn prepare(
        &self,
        original: &InputImage,
        regions: &[PageRegion],
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<Vec<RegionDiagnostic>> {
        let cancellation = RenderCancellation { flag: cancelled };
        let output =
            ffi::prepare_manga_typography(&original.bytes, &native_regions(regions), &cancellation);
        check_error(&output.error, &cancellation)?;
        diagnostics(output.diagnostics)
    }

    fn render(
        &self,
        original: &InputImage,
        cleaned: &[u8],
        regions: &[PageRegion],
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<RenderedPage> {
        let cancellation = RenderCancellation { flag: cancelled };
        let output = ffi::render_cleaned_manga_page(
            &original.bytes,
            cleaned,
            &native_regions(regions),
            &cancellation,
        );
        check_error(&output.error, &cancellation)?;
        if output.png.len() > codex::MAX_GENERATED_IMAGE_BYTES {
            return Err(CodexError::Image);
        }
        Ok(RenderedPage {
            png: output.png,
            diagnostics: diagnostics(output.diagnostics)?,
        })
    }
}

fn check_error(error: &str, cancellation: &RenderCancellation) -> codex::Result<()> {
    if cancellation.flag.load(Ordering::Acquire) {
        return Err(CodexError::Cancelled);
    }
    if !error.is_empty() {
        return Err(CodexError::Tool(error.into()));
    }
    Ok(())
}

fn diagnostics(native: Vec<ffi::MangaDiagnostic>) -> codex::Result<Vec<RegionDiagnostic>> {
    native
        .into_iter()
        .map(|item| {
            let status = match item.status {
                0 => RenderStatus::Rendered,
                1 => RenderStatus::EmptyTranslation,
                2 => RenderStatus::InvalidGeometry,
                3 => RenderStatus::Overflow,
                4 => RenderStatus::MissingGlyph,
                _ => return Err(CodexError::Protocol),
            };
            Ok(RegionDiagnostic {
                id: item.region_id,
                status,
                font_pixels: item.font_pixels,
                layout_lines: item.layout_lines,
                cleanup_pixels: item.cleanup_pixels,
            })
        })
        .collect()
}

fn native_regions(regions: &[PageRegion]) -> Vec<ffi::MangaRegion> {
    regions
        .iter()
        .map(|region| ffi::MangaRegion {
            id: region.id.clone(),
            translation: region
                .translation
                .iter()
                .map(|run| ffi::MangaTextRun {
                    text: run.text.clone(),
                    bold: run.bold,
                    italic: run.italic,
                })
                .collect(),
            family: match region.family {
                FontFamily::Comic => ffi::MangaFontFamily::Comic,
                FontFamily::Serif => ffi::MangaFontFamily::Serif,
                FontFamily::Condensed => ffi::MangaFontFamily::Condensed,
            },
            angle: region.angle,
            text_color: region.text_color.clone(),
            outline_color: region.outline_color.clone().unwrap_or_default(),
            outline_width: region.outline_width,
            bounds: native_rect(&region.bounds),
            layout: native_rect(&region.layout),
            cleanup_rects: region.cleanup_rects.iter().map(native_rect).collect(),
        })
        .collect()
}

fn native_rect(rect: &qbutt_content::translation::page::Rect) -> ffi::MangaRect {
    ffi::MangaRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
