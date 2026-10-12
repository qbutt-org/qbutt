//! Read-only local book import. Archive names never become filesystem paths.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use encoding_rs::Encoding;
use rbook::Epub;
use roxmltree::{Document, Node, ParsingOptions};
use sha2::{Digest, Sha256};
use url::Url;
use zip::ZipArchive;

use crate::{
    Category, Chapter, ChapterRef, Error, PublicationRef, Result, Source, TextChapter,
    TextImageAsset, Transport, Variant, VariantRef, VariantSelector, Work,
};

const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CHAPTER_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalBookFormat {
    Epub,
    Fb2,
    Txt,
}

#[derive(Clone, Debug)]
pub struct LocalBookChapter {
    pub id: String,
    pub title: String,
    resource: String,
}

#[derive(Clone, Debug)]
pub struct LocalBook {
    pub id: String,
    pub title: String,
    pub format: LocalBookFormat,
    pub chapters: Vec<LocalBookChapter>,
    path: PathBuf,
}

impl LocalBook {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().canonicalize().map_err(Error::LocalBookIo)?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let format = match extension.to_ascii_lowercase().as_str() {
            "epub" => LocalBookFormat::Epub,
            "fb2" => LocalBookFormat::Fb2,
            "txt" => LocalBookFormat::Txt,
            _ => return Err(Error::InvalidLocalBook("Choose an EPUB, FB2 or TXT file")),
        };
        let metadata = path.metadata().map_err(Error::LocalBookIo)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES {
            return Err(Error::InvalidLocalBook(
                "The book file is empty or too large",
            ));
        }
        if format == LocalBookFormat::Txt && metadata.len() > MAX_CHAPTER_BYTES as u64 {
            return Err(Error::ResponseTooLarge);
        }
        let id = hash_file(&path)?;
        let title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Book")
            .to_owned();
        let mut book = Self {
            id,
            title,
            format,
            chapters: Vec::new(),
            path,
        };
        match format {
            LocalBookFormat::Epub => book.inspect_epub()?,
            LocalBookFormat::Fb2 => book.inspect_fb2()?,
            LocalBookFormat::Txt => book.chapters.push(LocalBookChapter {
                id: "1".into(),
                title: book.title.clone(),
                resource: String::new(),
            }),
        }
        if book.chapters.is_empty() {
            return Err(Error::InvalidLocalBook("The book has no readable chapters"));
        }
        Ok(book)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn verify(&self) -> Result<()> {
        if hash_file(&self.path)? == self.id {
            Ok(())
        } else {
            Err(Error::InvalidLocalBook(
                "The book file changed since import",
            ))
        }
    }

    pub fn reference(&self) -> PublicationRef {
        PublicationRef {
            source: Source::LocalBook,
            id: self.id.clone(),
        }
    }

    pub fn variant(&self) -> VariantRef {
        VariantRef {
            publication: self.reference(),
            selector: VariantSelector::OriginalText,
        }
    }

    pub fn work(&self) -> Work {
        Work {
            reference: self.reference(),
            title: self.title.clone(),
            category: Category::Books,
            url: Url::parse(&format!("qbutt-local://{}", self.id)).unwrap(),
            cover: None,
            description: None,
            year: None,
            aliases: Vec::new(),
            identifiers: Default::default(),
            variants: vec![Variant {
                reference: self.variant(),
                title: None,
                transport: Transport::Text,
                episode: None,
                quality: None,
                translation: None,
                languages: Vec::new(),
                player: None,
                size: None,
                seeders: None,
                info_hash: None,
                reported_video_ads: false,
            }],
            needs_inspection: false,
        }
    }

    pub fn chapter_list(&self) -> Vec<Chapter> {
        self.chapters
            .iter()
            .map(|chapter| Chapter {
                reference: ChapterRef {
                    variant: self.variant(),
                    id: chapter.id.clone(),
                },
                title: Some(chapter.title.clone()),
                volume: None,
                number: None,
                pages: 0,
            })
            .collect()
    }

    pub fn read_chapter(&self, reference: &ChapterRef) -> Result<TextChapter> {
        reference.validate()?;
        if reference.variant != self.variant() {
            return Err(Error::InvalidReference);
        }
        let chapter = self
            .chapters
            .iter()
            .find(|chapter| chapter.id == reference.id)
            .ok_or(Error::InvalidReference)?;
        match self.format {
            LocalBookFormat::Epub => self.read_epub_chapter(reference, chapter),
            LocalBookFormat::Fb2 => self.read_fb2_chapter(reference, chapter),
            LocalBookFormat::Txt => self.read_txt_chapter(reference),
        }
    }

    fn inspect_epub(&mut self) -> Result<()> {
        check_zip(&self.path)?;
        let epub = Epub::open(&self.path)
            .map_err(|_| Error::InvalidLocalBook("The EPUB package is invalid"))?;
        if let Some(title) = epub.metadata().title() {
            let title = title.value().trim();
            if !title.is_empty() {
                self.title = title.chars().take(300).collect();
            }
        }
        let toc_titles = epub
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
                                label.chars().take(300).collect::<String>(),
                            )
                        })
                    })
                    .collect::<HashMap<_, _>>()
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
            let id = (self.chapters.len() + 1).to_string();
            let title = toc_titles
                .get(&path)
                .cloned()
                .unwrap_or_else(|| format!("Chapter {id}"));
            self.chapters.push(LocalBookChapter {
                id,
                title,
                resource: resource.id().to_owned(),
            });
        }
        Ok(())
    }

    fn read_epub_chapter(
        &self,
        reference: &ChapterRef,
        chapter: &LocalBookChapter,
    ) -> Result<TextChapter> {
        self.verify()?;
        let epub = Epub::open(&self.path)
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
                safe_archive_path(item.href().path().decode().as_ref())
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
        TextChapter::from_local_html(reference.clone(), &body, &mut resolve_image)
    }

    fn inspect_fb2(&mut self) -> Result<()> {
        let xml = read_fb2(&self.path)?;
        let document = parse_fb2(&xml)?;
        let title = document
            .descendants()
            .find(|node| node.has_tag_name("book-title"))
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|title| !title.is_empty());
        if let Some(title) = title {
            self.title = title.chars().take(300).collect();
        }
        let body = document
            .descendants()
            .find(|node| node.has_tag_name("body") && node.attribute("name").is_none())
            .ok_or(Error::InvalidLocalBook("The FB2 has no main body"))?;
        for (index, section) in body
            .children()
            .filter(|node| node.has_tag_name("section"))
            .enumerate()
        {
            let title = section
                .children()
                .find(|node| node.has_tag_name("title"))
                .map(|node| {
                    node.descendants()
                        .filter(|child| child.is_text())
                        .filter_map(|child| child.text())
                        .collect::<String>()
                })
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| format!("Chapter {}", index + 1));
            self.chapters.push(LocalBookChapter {
                id: (index + 1).to_string(),
                title: title.trim().chars().take(300).collect(),
                resource: index.to_string(),
            });
        }
        if self.chapters.is_empty() {
            self.chapters.push(LocalBookChapter {
                id: "1".into(),
                title: self.title.clone(),
                resource: "body".into(),
            });
        }
        Ok(())
    }

    fn read_fb2_chapter(
        &self,
        reference: &ChapterRef,
        chapter: &LocalBookChapter,
    ) -> Result<TextChapter> {
        self.verify()?;
        let xml = read_fb2(&self.path)?;
        let document = parse_fb2(&xml)?;
        let body = document
            .descendants()
            .find(|node| node.has_tag_name("body") && node.attribute("name").is_none())
            .ok_or(Error::InvalidLocalBook("The FB2 has no main body"))?;
        let section = if chapter.resource == "body" {
            body
        } else {
            body.children()
                .filter(|node| node.has_tag_name("section"))
                .nth(
                    chapter
                        .resource
                        .parse::<usize>()
                        .map_err(|_| Error::InvalidReference)?,
                )
                .ok_or(Error::InvalidReference)?
        };
        let mut html = String::new();
        render_fb2(section, &mut html, 0)?;
        let binary = document
            .descendants()
            .filter(|node| node.has_tag_name("binary"))
            .filter_map(|node| node.attribute("id").map(|id| (id, node)))
            .collect::<HashMap<_, _>>();
        let mut resolve_image = |source: &str| -> Result<Option<TextImageAsset>> {
            let Some(id) = source.strip_prefix('#') else {
                return Ok(None);
            };
            let Some(node) = binary.get(id) else {
                return Ok(None);
            };
            let Some(media_type) = node.attribute("content-type").and_then(image_media_type) else {
                return Ok(None);
            };
            let encoded = node.text().unwrap_or_default();
            if encoded.len() > MAX_IMAGE_BYTES * 2 {
                return Err(Error::ResponseTooLarge);
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded.split_ascii_whitespace().collect::<String>())
                .map_err(|_| Error::InvalidLocalBook("An FB2 image is corrupt"))?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            if !valid_image_bytes(media_type, &bytes) {
                return Err(Error::InvalidLocalBook("An FB2 image is invalid"));
            }
            Ok(Some(TextImageAsset {
                id: format!("{:x}", Sha256::digest(&bytes)),
                media_type: media_type.into(),
                bytes: bytes.into(),
            }))
        };
        TextChapter::from_local_html(reference.clone(), &html, &mut resolve_image)
    }

    fn read_txt_chapter(&self, reference: &ChapterRef) -> Result<TextChapter> {
        self.verify()?;
        let bytes = std::fs::read(&self.path).map_err(Error::LocalBookIo)?;
        let text = decode_text(&bytes)?;
        let mut html = String::new();
        for paragraph in text.split("\n\n") {
            html.push_str("<p>");
            escape_html(paragraph.trim(), &mut html);
            html.push_str("</p>");
        }
        TextChapter::from_html(reference.clone(), &html)
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(Error::LocalBookIo)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(Error::LocalBookIo)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn check_zip(path: &Path) -> Result<()> {
    let file = File::open(path).map_err(Error::LocalBookIo)?;
    let mut archive = ZipArchive::new(file)
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

fn image_media_type(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        _ => None,
    }
}

fn valid_image_bytes(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()),
        _ => false,
    }
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

fn read_fb2(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(Error::LocalBookIo)?;
    decode_text(&bytes)
}

fn decode_text(bytes: &[u8]) -> Result<String> {
    let declaration = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
    let label = declaration
        .split("encoding=")
        .nth(1)
        .and_then(|value| {
            value
                .chars()
                .next()
                .and_then(|quote| value[1..].split(quote).next())
        })
        .unwrap_or("utf-8");
    let encoding = Encoding::for_label(label.as_bytes()).ok_or(Error::InvalidLocalBook(
        "The book text encoding is unsupported",
    ))?;
    let (text, _, errors) = encoding.decode(bytes);
    if errors {
        return Err(Error::InvalidLocalBook("The book text encoding is invalid"));
    }
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

fn parse_fb2(xml: &str) -> Result<Document<'_>> {
    Document::parse_with_options(
        xml,
        ParsingOptions {
            nodes_limit: 100_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|_| Error::InvalidLocalBook("The FB2 XML is invalid"))
}

fn render_fb2(node: Node<'_, '_>, output: &mut String, depth: usize) -> Result<()> {
    if depth > 128 {
        return Err(Error::ResponseTooLarge);
    }
    if node.is_text() {
        escape_html(node.text().unwrap_or_default(), output);
        return Ok(());
    }
    if !node.is_element() {
        return Ok(());
    }
    if node.has_tag_name("title") {
        output.push_str("<h2>");
        for child in node.descendants().filter(|child| child.is_text()) {
            escape_html(child.text().unwrap_or_default(), output);
        }
        output.push_str("</h2>");
        return Ok(());
    }
    let tag = match node.tag_name().name() {
        "subtitle" => "h2",
        "p" | "v" | "text-author" => "p",
        "strong" => "strong",
        "emphasis" => "em",
        "epigraph" | "cite" => "blockquote",
        "section" | "body" | "poem" | "stanza" => "section",
        "image" => {
            if let Some(source) = node
                .attributes()
                .find(|attribute| attribute.name() == "href")
            {
                output.push_str("<img src=\"");
                escape_html(source.value(), output);
                output.push_str("\">");
            }
            return Ok(());
        }
        "empty-line" => {
            output.push_str("<br>");
            return Ok(());
        }
        "binary" | "script" | "style" => return Ok(()),
        _ => {
            for child in node.children() {
                render_fb2(child, output, depth + 1)?;
            }
            return Ok(());
        }
    };
    output.push('<');
    output.push_str(tag);
    output.push('>');
    for child in node.children() {
        render_fb2(child, output, depth + 1)?;
    }
    output.push_str("</");
    output.push_str(tag);
    output.push('>');
    if output.len() > MAX_CHAPTER_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok(())
}

fn escape_html(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}
