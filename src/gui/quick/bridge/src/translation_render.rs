use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use qbutt_content::translation::{
    codex::{self, CodexError, InputImage},
    manga::{self, Ink, PageRegion, Rect, RenderLayers},
};
use serde_json::json;

pub struct RenderCancellation {
    flag: Arc<AtomicBool>,
}

fn render_cancelled(cancellation: &RenderCancellation) -> bool {
    cancellation.flag.load(Ordering::Acquire)
}

#[cxx::bridge(namespace = "Qbutt")]
mod ffi {
    struct MangaRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    struct MangaRegion {
        id: String,
        translation: String,
        bounds: MangaRect,
        layout: MangaRect,
        ink: u8,
        ink_boxes: Vec<MangaRect>,
    }

    struct MangaDiagnostic {
        region_id: String,
        status: String,
        font_pixels: u32,
        layout_lines: u32,
        masked_pixels: u32,
    }

    struct MangaRenderOutput {
        result_png: Vec<u8>,
        cleaned_png: Vec<u8>,
        mask_png: Vec<u8>,
        diagnostics: Vec<MangaDiagnostic>,
        width: u32,
        height: u32,
        error: String,
    }

    unsafe extern "C++" {
        include!("qbutt_quick/translation_render.h");

        fn render_manga_page(
            original: &[u8],
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

impl manga::PageRenderer for QtMangaRenderer {
    fn render(
        &self,
        original: &InputImage,
        regions: &[PageRegion],
        cancelled: Arc<AtomicBool>,
    ) -> codex::Result<RenderLayers> {
        let native_regions = regions
            .iter()
            .map(|region| ffi::MangaRegion {
                id: region.id.clone(),
                translation: region.translation.clone(),
                bounds: native_rect(&region.bounds),
                layout: native_rect(&region.layout),
                ink: match region.ink {
                    Ink::Dark => 0,
                    Ink::Light => 1,
                },
                ink_boxes: region.ink_boxes.iter().map(native_rect).collect(),
            })
            .collect::<Vec<_>>();
        let cancellation = RenderCancellation { flag: cancelled };
        let output = ffi::render_manga_page(&original.bytes, &native_regions, &cancellation);
        if !output.error.is_empty() {
            return Err(CodexError::Tool(output.error));
        }

        let diagnostics = output
            .diagnostics
            .iter()
            .map(|item| {
                json!({
                    "id": item.region_id,
                    "status": item.status,
                    "fontPixels": item.font_pixels,
                    "layoutLines": item.layout_lines,
                    "maskedPixels": item.masked_pixels,
                })
            })
            .collect::<Vec<_>>();
        Ok(RenderLayers {
            result_png: output.result_png,
            cleaned_png: output.cleaned_png,
            mask_png: output.mask_png,
            diagnostics: json!({
                "width": output.width,
                "height": output.height,
                "regions": diagnostics,
            }),
        })
    }
}

fn native_rect(rect: &Rect) -> ffi::MangaRect {
    ffi::MangaRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
