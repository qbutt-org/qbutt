use std::collections::HashMap;

use scraper::{ElementRef, Html};

use crate::{Category, Error, Result};

use super::response::{decimal, required, selector, unique};

pub(crate) type Categories = HashMap<u64, Option<Category>>;

/// Classify each forum by its ancestors in the site's nested forum map.
pub(crate) fn categories(body: &str) -> Result<Categories> {
    let document = Html::parse_document(body);
    let map = unique(document.root_element(), "#f-map")?;
    let mut categories = HashMap::new();
    for link in map.select(&selector("a[href]")) {
        let href = required(link, "href")?;
        if !href.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            continue;
        }
        if categories.len() >= 10_000 {
            return Err(Error::InvalidResponse("excessive RuTracker forums"));
        }
        let id = decimal(href)?;
        let mut category = root_category(id);
        for ancestor in link.ancestors().filter_map(ElementRef::wrap) {
            if category.is_some() {
                break;
            }
            if ancestor.value().name() != "li" {
                continue;
            }
            let Some(label) = ancestor
                .child_elements()
                .find(|child| child.value().name() == "span")
            else {
                continue;
            };
            if let Some(parent) = label.select(&selector("a[href]")).find(|candidate| {
                candidate
                    .value()
                    .attr("href")
                    .is_some_and(|href| href.as_bytes().first().is_some_and(u8::is_ascii_digit))
            }) {
                category = root_category(decimal(required(parent, "href")?)?);
            } else if let Some(section) = label.select(&selector(".c-title")).next() {
                category = match section.attr("title") {
                    Some("Книги и журналы" | "Аудиокниги") => {
                        Some(Category::Books)
                    }
                    _ => None,
                };
            }
        }
        if categories.insert(id, category).is_some() {
            return Err(Error::InvalidResponse("duplicate RuTracker forum"));
        }
    }
    if categories.is_empty() {
        return Err(Error::InvalidResponse("empty RuTracker forum map"));
    }
    Ok(categories)
}

fn root_category(id: u64) -> Option<Category> {
    match id {
        22 | 7 | 124 | 511 | 93 | 2198 | 718 | 4 => Some(Category::Movies),
        921 | 9 | 189 | 2366 | 119 | 911 | 2100 | 24 => Some(Category::Tv),
        33 => Some(Category::Anime),
        5 | 139 | 2142 | 960 | 899 | 548 | 650 => Some(Category::Games),
        1012 | 1376 | 1013 | 1014 | 1052 | 1016 | 1503 | 285 | 1366 | 1933 => {
            Some(Category::Software)
        }
        2057 | 2413 => Some(Category::Books),
        862 => Some(Category::Manga),
        _ => None,
    }
}
