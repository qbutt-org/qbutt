use std::collections::HashMap;

use scraper::{ElementRef, Html};

use crate::{Category, Error, Result};

use super::response::{decimal, required, selector, unique};

pub(crate) type Categories = HashMap<u64, Option<Category>>;

/// The public map exposes numeric forum IDs in a nested list before the site's
/// JavaScript turns them into viewforum links. Classify descendants by their
/// actual ancestors, never by query text or the requested product category.
pub(crate) fn categories(body: &str) -> Result<Categories> {
    let document = Html::parse_document(body);
    let map = unique(document.root_element(), "#f-map")?;
    let mut categories = HashMap::new();
    for (index, link) in map.select(&selector("a")).enumerate() {
        if index >= 10_000 {
            return Err(Error::InvalidResponse("excessive RuTracker forums"));
        }
        let id = decimal(required(link, "href")?)?;
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
            if let Some(parent) = label.select(&selector("a")).next() {
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
