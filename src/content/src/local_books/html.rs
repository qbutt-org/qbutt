//! Local HTML uses the text reader and resolves images only below its folder.

use std::fs::File;
use std::io::Read;
use std::path::{Component, PathBuf};

use scraper::{Html, Selector};
use sha2::{Digest, Sha256};

use super::{
    LocalBook, MAX_IMAGE_BYTES, decode_text, image_media_type_for_name, valid_image_bytes,
};
use crate::{ChapterRef, Error, Result, TextChapter, TextImageAsset};

impl LocalBook {
    pub(super) fn read_html_chapter(&self, reference: &ChapterRef) -> Result<TextChapter> {
        let html = decode_text(&self.read_verified_bytes()?)?;
        let document = Html::parse_document(&html);
        let body = document
            .select(&Selector::parse("body").expect("static body selector"))
            .next()
            .ok_or(Error::InvalidLocalBook("The HTML document has no body"))?
            .inner_html();
        let folder = self.path.parent().ok_or(Error::InvalidReference)?;
        let mut resolve_image = |source: &str| -> Result<Option<TextImageAsset>> {
            let source = source.split(['?', '#']).next().unwrap_or_default();
            if source.is_empty()
                || source.starts_with('/')
                || source.contains('\\')
                || source.contains(':')
            {
                return Ok(None);
            }
            let decoded = match percent_encoding::percent_decode_str(source).decode_utf8() {
                Ok(value) => value,
                Err(_) => return Ok(None),
            };
            let relative = PathBuf::from(decoded.as_ref());
            if relative.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return Ok(None);
            }
            let path = match folder.join(relative).canonicalize() {
                Ok(path) if path.starts_with(folder) && path.is_file() => path,
                _ => return Ok(None),
            };
            let Some(media_type) = image_media_type_for_name(&path.to_string_lossy()) else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            File::open(path)
                .map_err(Error::LocalBookIo)?
                .take(MAX_IMAGE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(Error::LocalBookIo)?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            if !valid_image_bytes(media_type, &bytes) {
                return Err(Error::InvalidLocalBook("A local HTML image is invalid"));
            }
            Ok(Some(TextImageAsset {
                id: format!("{:x}", Sha256::digest(&bytes)),
                media_type: media_type.into(),
                bytes: bytes.into(),
            }))
        };
        TextChapter::from_html_with_images(reference.clone(), &body, &mut resolve_image, None, None)
    }
}
