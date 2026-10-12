use serde::{Deserialize, Serialize};

use super::BrowserError;
use super::validation::{text, uuid_text};

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
}

impl Selection {
    pub(super) fn validate(&self) -> Result<(), BrowserError> {
        if [
            &self.episode,
            &self.season,
            &self.translation_id,
            &self.player,
        ]
        .into_iter()
        .flatten()
        .any(|value| !text(value, 200))
            || self
                .capture_id
                .as_ref()
                .is_some_and(|value| !uuid_text(value))
        {
            return Err(BrowserError::InvalidRequest);
        }
        Ok(())
    }
}
