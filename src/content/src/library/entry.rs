use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Source, Transport, VariantRef, Work};

/// Raw provider labels are retained so reopening a work restores its dropdowns.
/// Filters never replace the identity of an explicitly selected variant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantFilters {
    pub episode: Option<String>,
    pub translation: Option<String>,
    pub source: Option<Source>,
    pub language: Option<String>,
    pub player: Option<String>,
    pub transport: Option<Transport>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantPreferences {
    pub selected: Option<VariantRef>,
    pub filters: VariantFilters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LastOpened {
    pub variant: VariantRef,
    pub at_unix_millis: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryEntry {
    pub work: Work,
    pub preferences: VariantPreferences,
    pub favorite: bool,
    pub last_opened: Option<LastOpened>,
}

impl LibraryEntry {
    pub(super) fn validate(&self) -> std::result::Result<(), &'static str> {
        validate_work(&self.work)?;

        for selected in self
            .preferences
            .selected
            .iter()
            .chain(self.last_opened.iter().map(|opened| &opened.variant))
        {
            if !self
                .work
                .variants
                .iter()
                .any(|variant| &variant.reference == selected)
            {
                return Err("the exact selected variant is absent from the work");
            }
        }
        if self
            .last_opened
            .as_ref()
            .is_some_and(|opened| opened.at_unix_millis < 0)
        {
            return Err("the history timestamp is invalid");
        }
        Ok(())
    }

    pub(super) fn refresh(&mut self, work: &Work) {
        let mut refreshed = work.clone();
        let known = work
            .variants
            .iter()
            .map(|variant| &variant.reference)
            .collect::<HashSet<_>>();
        for variant in &self.work.variants {
            if !known.contains(&variant.reference) {
                refreshed.variants.push(variant.clone());
            }
        }
        self.work = refreshed;
    }
}

fn validate_work(work: &Work) -> std::result::Result<(), &'static str> {
    if work.reference.id.trim().is_empty() || work.title.trim().is_empty() {
        return Err("the work identity or title is missing");
    }
    if !public_url(&work.url) || work.cover.as_ref().is_some_and(|url| !public_url(url)) {
        return Err("the work contains a nonpublic URL");
    }

    let mut known = HashSet::new();
    for variant in &work.variants {
        if variant.reference.publication != work.reference {
            return Err("a variant belongs to another publication");
        }
        if !known.insert(&variant.reference) {
            return Err("the work contains duplicate exact variants");
        }
    }
    Ok(())
}

// The current catalog's persistent page and poster URLs contain no query or
// fragment. Reject acquisition credentials instead of trying to strip them.
fn public_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}
