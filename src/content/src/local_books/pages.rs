//! Bounded comic archives and individual image files.

use std::cmp::Ordering;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use super::{
    LocalBook, LocalBookChapter, LocalBookFormat, MAX_PAGE_BYTES, epub::check_zip,
    image_media_type_for_name, valid_image_bytes,
};
use crate::{Error, Result};

impl LocalBook {
    pub(super) fn inspect_pages(&mut self) -> Result<()> {
        match self.format {
            LocalBookFormat::Image => {
                let bytes = self.read_verified_bytes()?;
                let media_type = image_media_type_for_name(&self.path.to_string_lossy())
                    .ok_or(Error::InvalidLocalBook("The image format is unsupported"))?;
                if !valid_image_bytes(media_type, &bytes) {
                    return Err(Error::InvalidLocalBook("The image is invalid"));
                }
                self.page_paths.push(String::new());
            }
            LocalBookFormat::ComicArchive => {
                let bytes = self.read_verified_bytes()?;
                check_zip(&bytes)?;
                let mut archive = ZipArchive::new(Cursor::new(bytes))
                    .map_err(|_| Error::InvalidLocalBook("The comic archive is invalid"))?;
                for index in 0..archive.len() {
                    let entry = archive
                        .by_index_raw(index)
                        .map_err(|_| Error::InvalidLocalBook("The comic archive is invalid"))?;
                    let name = entry.name().map_err(|_| {
                        Error::InvalidLocalBook("The comic archive has an invalid filename")
                    })?;
                    if image_media_type_for_name(&name).is_some() && !entry.is_dir() {
                        if entry.size() > MAX_PAGE_BYTES as u64 {
                            return Err(Error::ResponseTooLarge);
                        }
                        self.page_paths.push(name.into_owned());
                    }
                }
                self.page_paths
                    .sort_by(|left, right| natural_order(left, right));
            }
            _ => return Err(Error::InvalidReference),
        }
        if self.page_paths.is_empty() {
            return Err(Error::InvalidLocalBook("The comic has no readable pages"));
        }
        self.chapters.push(LocalBookChapter {
            id: "1".into(),
            title: self.title.clone(),
            resource: String::new(),
            anchor: None,
            end_anchor: None,
        });
        Ok(())
    }

    pub(super) fn read_page(&self, index: usize) -> Result<Vec<u8>> {
        let bytes = self.read_verified_bytes()?;
        let name = &self.page_paths[index];
        let media_type = if self.format == LocalBookFormat::Image {
            image_media_type_for_name(&self.path.to_string_lossy())
        } else {
            image_media_type_for_name(name)
        }
        .ok_or(Error::InvalidReference)?;
        let bytes = if self.format == LocalBookFormat::Image {
            bytes
        } else {
            let mut archive = ZipArchive::new(Cursor::new(bytes))
                .map_err(|_| Error::InvalidLocalBook("The comic archive is invalid"))?;
            let mut entry = archive
                .by_name(name)
                .map_err(|_| Error::InvalidLocalBook("A comic page is missing"))?;
            let mut page = Vec::new();
            entry
                .by_ref()
                .take(MAX_PAGE_BYTES as u64 + 1)
                .read_to_end(&mut page)
                .map_err(|_| Error::InvalidLocalBook("A comic page could not be read"))?;
            if page.len() > MAX_PAGE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            page
        };
        if !valid_image_bytes(media_type, &bytes) {
            return Err(Error::InvalidLocalBook("A comic page is invalid"));
        }
        Ok(bytes)
    }
}

fn natural_order(left: &str, right: &str) -> Ordering {
    let mut left = left.as_bytes();
    let mut right = right.as_bytes();
    while !left.is_empty() && !right.is_empty() {
        let left_digits = left[0].is_ascii_digit();
        let right_digits = right[0].is_ascii_digit();
        if left_digits && right_digits {
            let left_end = left.iter().take_while(|byte| byte.is_ascii_digit()).count();
            let right_end = right
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            let left_zeroes = left[..left_end]
                .iter()
                .take_while(|byte| **byte == b'0')
                .count();
            let right_zeroes = right[..right_end]
                .iter()
                .take_while(|byte| **byte == b'0')
                .count();
            let left_number = &left[left_zeroes..left_end];
            let right_number = &right[right_zeroes..right_end];
            let order = left_number
                .len()
                .cmp(&right_number.len())
                .then_with(|| left_number.cmp(&right_number));
            if order != Ordering::Equal {
                return order;
            }
            left = &left[left_end..];
            right = &right[right_end..];
        } else {
            let order = left[0]
                .to_ascii_lowercase()
                .cmp(&right[0].to_ascii_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            left = &left[1..];
            right = &right[1..];
        }
    }
    left.len().cmp(&right.len())
}
