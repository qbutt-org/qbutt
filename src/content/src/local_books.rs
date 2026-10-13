//! Read-only local book import. Archive names never become filesystem paths.

mod epub;
mod fb2;
mod html;
mod pages;

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use encoding_rs::Encoding;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    Category, Chapter, ChapterImage, ChapterPages, ChapterRef, Error, PublicationRef, Result,
    Source, TextChapter, Transport, Variant, VariantRef, VariantSelector, Work,
};

const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CHAPTER_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalBookFormat {
    Epub,
    Fb2,
    Txt,
    Html,
    ComicArchive,
    Image,
}

#[derive(Clone, Debug)]
pub struct LocalBookChapter {
    pub id: String,
    pub title: String,
    resource: String,
    anchor: Option<String>,
    end_anchor: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LocalBook {
    pub id: String,
    pub title: String,
    pub format: LocalBookFormat,
    pub chapters: Vec<LocalBookChapter>,
    path: PathBuf,
    page_paths: Vec<String>,
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
            "html" | "htm" => LocalBookFormat::Html,
            "cbz" | "zip" => LocalBookFormat::ComicArchive,
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => LocalBookFormat::Image,
            _ => return Err(Error::InvalidLocalBook("Choose a book or comic file")),
        };
        let metadata = path.metadata().map_err(Error::LocalBookIo)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES {
            return Err(Error::InvalidLocalBook(
                "The book file is empty or too large",
            ));
        }
        if matches!(format, LocalBookFormat::Txt | LocalBookFormat::Html)
            && metadata.len() > MAX_CHAPTER_BYTES as u64
        {
            return Err(Error::ResponseTooLarge);
        }
        if format == LocalBookFormat::Image && metadata.len() > MAX_PAGE_BYTES as u64 {
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
            page_paths: Vec::new(),
        };
        match format {
            LocalBookFormat::Epub => book.inspect_epub()?,
            LocalBookFormat::Fb2 => book.inspect_fb2()?,
            LocalBookFormat::ComicArchive | LocalBookFormat::Image => book.inspect_pages()?,
            LocalBookFormat::Txt | LocalBookFormat::Html => book.chapters.push(LocalBookChapter {
                id: "1".into(),
                title: book.title.clone(),
                resource: String::new(),
                anchor: None,
                end_anchor: None,
            }),
        }
        if book.chapters.is_empty() {
            return Err(Error::InvalidLocalBook("The book has no readable chapters"));
        }
        book.verify()?;
        Ok(book)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn verify(&self) -> Result<()> {
        if hash_file(&self.path)? == self.id {
            Ok(())
        } else {
            Err(Error::LocalBookChanged)
        }
    }

    fn read_verified_bytes(&self) -> Result<Vec<u8>> {
        let file = File::open(&self.path).map_err(Error::LocalBookIo)?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::LocalBookIo)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::ResponseTooLarge);
        }
        if format!("{:x}", Sha256::digest(&bytes)) != self.id {
            return Err(Error::LocalBookChanged);
        }
        Ok(bytes)
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
            selector: if self.is_paged() {
                VariantSelector::LocalPages
            } else {
                VariantSelector::OriginalText
            },
        }
    }

    fn is_paged(&self) -> bool {
        matches!(
            self.format,
            LocalBookFormat::ComicArchive | LocalBookFormat::Image
        )
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
                transport: if self.is_paged() {
                    Transport::Manga
                } else {
                    Transport::Text
                },
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
                pages: if self.is_paged() {
                    self.page_paths.len() as u32
                } else {
                    0
                },
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
            LocalBookFormat::Html => self.read_html_chapter(reference),
            LocalBookFormat::ComicArchive | LocalBookFormat::Image => Err(Error::InvalidReference),
        }
    }

    pub fn chapter_pages(&self, reference: &ChapterRef) -> Result<ChapterPages> {
        reference.validate()?;
        if !self.is_paged()
            || reference.variant != self.variant()
            || !self
                .chapters
                .iter()
                .any(|chapter| chapter.id == reference.id)
        {
            return Err(Error::InvalidReference);
        }
        self.verify()?;
        let images = (0..self.page_paths.len())
            .map(|index| ChapterImage {
                url: Url::parse(&format!("qbutt-local://{}/page/{index}", self.id)).unwrap(),
            })
            .collect();
        Ok(ChapterPages {
            reference: reference.clone(),
            images,
        })
    }

    pub fn image_bytes(&self, image: &ChapterImage) -> Result<Vec<u8>> {
        let url = image.url();
        let index = url
            .path()
            .strip_prefix("/page/")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|index| *index < self.page_paths.len());
        if !self.is_paged()
            || url.scheme() != "qbutt-local"
            || url.host_str() != Some(self.id.as_str())
            || url.query().is_some()
            || url.fragment().is_some()
            || index.is_none()
        {
            return Err(Error::InvalidReference);
        }
        self.read_page(index.unwrap())
    }

    fn read_txt_chapter(&self, reference: &ChapterRef) -> Result<TextChapter> {
        let bytes = self.read_verified_bytes()?;
        let text = decode_text(&bytes)?;
        let mut html = String::new();
        for paragraph in text.split("\n\n") {
            html.push_str("<p>");
            for (index, line) in paragraph.trim().split('\n').enumerate() {
                if index > 0 {
                    html.push_str("<br>");
                }
                escape_html(line, &mut html)?;
            }
            html.push_str("</p>");
            if html.len() > MAX_CHAPTER_BYTES {
                return Err(Error::ResponseTooLarge);
            }
        }
        let chapter = TextChapter::from_html(reference.clone(), &html)?;
        Ok(chapter)
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let file = File::open(path).map_err(Error::LocalBookIo)?;
    let mut hasher = Sha256::new();
    let bytes = std::io::copy(&mut file.take(MAX_FILE_BYTES + 1), &mut hasher)
        .map_err(Error::LocalBookIo)?;
    if bytes > MAX_FILE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn image_media_type(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        "image/bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn image_media_type_for_name(name: &str) -> Option<&'static str> {
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn valid_image_bytes(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()),
        "image/bmp" => bytes.starts_with(b"BM"),
        _ => false,
    }
}

fn decode_text(bytes: &[u8]) -> Result<String> {
    let declaration = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]);
    let label = declaration
        .split("encoding=")
        .nth(1)
        .and_then(|value| match value.chars().next() {
            Some(quote @ ('\'' | '"')) => value[quote.len_utf8()..].split(quote).next(),
            _ => None,
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

fn check_html_size(output: &str) -> Result<()> {
    if output.len() > MAX_CHAPTER_BYTES {
        Err(Error::ResponseTooLarge)
    } else {
        Ok(())
    }
}

fn escape_html(text: &str, output: &mut String) -> Result<()> {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
        check_html_size(output)?;
    }
    Ok(())
}
