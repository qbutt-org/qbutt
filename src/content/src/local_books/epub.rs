//! EPUB spine, resources and archive boundaries.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::io::Write;

use rbook::Epub;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::{
    LocalBook, LocalBookChapter, MAX_ARCHIVE_BYTES, MAX_ARCHIVE_ENTRIES, MAX_CHAPTER_BYTES,
    MAX_ENTRY_BYTES, MAX_IMAGE_BYTES, image_media_type, valid_image_bytes,
};
use crate::{ChapterRef, Error, Result, TextChapter, TextImageAsset};

impl LocalBook {
    pub(super) fn inspect_epub(&mut self) -> Result<()> {
        let bytes = self.read_verified_bytes()?;
        check_zip(&bytes)?;
        let epub = Epub::read(Cursor::new(bytes))
            .map_err(|_| Error::InvalidLocalBook("The EPUB package is invalid"))?;
        if let Some(title) = epub.metadata().title() {
            let title = title.value().trim();
            if !title.is_empty() {
                self.title = title.chars().take(300).collect();
            }
        }
        let toc_entries = epub
            .toc()
            .contents()
            .map(|contents| {
                contents
                    .flatten()
                    .filter_map(|entry| {
                        let href = entry.href()?;
                        let label = entry.label().trim();
                        (!label.is_empty()).then(|| {
                            (
                                href.path().decode().into_owned(),
                                href.fragment().and_then(|fragment| {
                                    percent_encoding::percent_decode_str(fragment)
                                        .decode_utf8()
                                        .ok()
                                        .map(|value| value.into_owned())
                                }),
                                label.chars().take(300).collect::<String>(),
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for entry in epub.spine() {
            if !entry.is_linear() {
                continue;
            }
            let Some(resource) = entry.manifest_entry() else {
                continue;
            };
            if resource.media_type() != "application/xhtml+xml" {
                continue;
            }
            let path = resource.href().path().decode().into_owned();
            let matching = toc_entries
                .iter()
                .filter(|(target, _, _)| target == &path)
                .collect::<Vec<_>>();
            for (index, (_, anchor, title)) in matching.iter().enumerate() {
                self.chapters.push(LocalBookChapter {
                    id: (self.chapters.len() + 1).to_string(),
                    title: title.clone(),
                    resource: resource.id().to_owned(),
                    anchor: anchor.clone(),
                    end_anchor: matching.get(index + 1).and_then(|entry| entry.1.clone()),
                });
            }
            if matching.is_empty() {
                let id = (self.chapters.len() + 1).to_string();
                self.chapters.push(LocalBookChapter {
                    id: id.clone(),
                    title: format!("Chapter {id}"),
                    resource: resource.id().to_owned(),
                    anchor: None,
                    end_anchor: None,
                });
            }
        }
        Ok(())
    }

    pub(super) fn read_epub_chapter(
        &self,
        reference: &ChapterRef,
        chapter: &LocalBookChapter,
    ) -> Result<TextChapter> {
        let epub = Epub::read(Cursor::new(self.read_verified_bytes()?))
            .map_err(|_| Error::InvalidLocalBook("The EPUB package is invalid"))?;
        let manifest = epub.manifest();
        let resource = manifest
            .by_id(&chapter.resource)
            .ok_or(Error::InvalidLocalBook("An EPUB chapter is missing"))?;
        let body = bounded_resource(resource, MAX_CHAPTER_BYTES)?;
        let body = String::from_utf8(body)
            .map_err(|_| Error::InvalidLocalBook("An EPUB chapter is not UTF-8"))?;
        let chapter_path = resource.href().path().decode().into_owned();
        let image_resources = manifest
            .iter()
            .filter(|item| image_media_type(item.media_type()).is_some())
            .filter_map(|item| {
                safe_archive_path(item.href().path().decode().trim_start_matches('/'))
                    .ok()
                    .map(|path| (path, item.id().to_owned()))
            })
            .collect::<HashMap<_, _>>();
        let mut resolve_image = |source: &str| -> Result<Option<TextImageAsset>> {
            let Some(path) = resolve_archive_href(&chapter_path, source) else {
                return Ok(None);
            };
            let Some(id) = image_resources.get(&path) else {
                return Ok(None);
            };
            let Some(item) = manifest.by_id(id) else {
                return Ok(None);
            };
            let Some(media_type) = image_media_type(item.media_type()) else {
                return Ok(None);
            };
            let bytes = bounded_resource(item, MAX_IMAGE_BYTES)?;
            if !valid_image_bytes(media_type, &bytes) {
                return Err(Error::InvalidLocalBook("An EPUB image is invalid"));
            }
            Ok(Some(TextImageAsset {
                id: format!("{:x}", Sha256::digest(&bytes)),
                media_type: media_type.into(),
                bytes: bytes.into(),
            }))
        };
        let chapter = TextChapter::from_html_with_images(
            reference.clone(),
            &body,
            &mut resolve_image,
            chapter.anchor.as_deref(),
            chapter.end_anchor.as_deref(),
        )?;
        Ok(chapter)
    }
}

fn check_zip(bytes: &[u8]) -> Result<()> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| Error::InvalidLocalBook("The EPUB archive is invalid"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::ResponseTooLarge);
    }
    let mut total = 0u64;
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| Error::InvalidLocalBook("The EPUB archive is invalid"))?;
        let entry_name = entry
            .name()
            .map_err(|_| Error::InvalidLocalBook("The EPUB archive has an invalid filename"))?;
        let name = safe_archive_path(entry_name.trim_end_matches('/'))?;
        if !names.insert(name)
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(Error::InvalidLocalBook(
                "The EPUB has unsafe archive entries",
            ));
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        total = total.saturating_add(entry.size());
        if total > MAX_ARCHIVE_BYTES {
            return Err(Error::ResponseTooLarge);
        }
    }
    Ok(())
}

fn safe_archive_path(path: &str) -> Result<String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(Error::InvalidLocalBook(
            "The EPUB has an unsafe archive path",
        ));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part == ".." || part.is_empty() {
            return Err(Error::InvalidLocalBook(
                "The EPUB has an unsafe archive path",
            ));
        }
        if part != "." {
            parts.push(part);
        }
    }
    Ok(parts.join("/"))
}

fn resolve_archive_href(chapter_path: &str, href: &str) -> Option<String> {
    let href = href.split(['#', '?']).next()?;
    if href.is_empty() || href.starts_with('/') || href.contains('\\') || href.contains(':') {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(href)
        .decode_utf8()
        .ok()?;
    let base = chapter_path
        .trim_start_matches('/')
        .rsplit_once('/')
        .map_or("", |(base, _)| base);
    let mut parts = base
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ if part.contains(':') || part.contains('\\') => return None,
            _ => parts.push(part),
        }
    }
    let path = parts.join("/");
    safe_archive_path(&path).ok()
}

fn bounded_resource(
    entry: rbook::epub::manifest::EpubManifestEntry<'_>,
    limit: usize,
) -> Result<Vec<u8>> {
    struct BoundedBytes {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for BoundedBytes {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other(
                    "EPUB resource exceeds the size limit",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        limit,
    };
    entry
        .copy_bytes(&mut output)
        .map_err(|_| Error::InvalidLocalBook("An EPUB resource could not be read"))?;
    Ok(output.bytes)
}
