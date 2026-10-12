use serde_json::{Value, json};

use super::{Client, CodexError, Result};

#[derive(Clone, Debug)]
pub struct CodexModel {
    pub id: String,
    pub efforts: Vec<String>,
    pub default_effort: String,
    pub is_default: bool,
}

impl Client {
    pub(super) async fn available_models(&mut self) -> Result<Vec<CodexModel>> {
        let mut cursor: Option<String> = None;
        let mut models = Vec::new();

        loop {
            let result = self
                .request(
                    "model/list",
                    json!({"limit": 100, "includeHidden": false, "cursor": cursor}),
                )
                .await?;
            let entries = result
                .get("data")
                .and_then(Value::as_array)
                .ok_or(CodexError::Protocol)?;

            for entry in entries {
                let image_capable = entry
                    .get("inputModalities")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.contains(&json!("image")));
                if !image_capable {
                    continue;
                }

                let id = entry
                    .get("model")
                    .and_then(Value::as_str)
                    .ok_or(CodexError::Protocol)?;
                let efforts = entry
                    .get("supportedReasoningEfforts")
                    .and_then(Value::as_array)
                    .ok_or(CodexError::Protocol)?
                    .iter()
                    .map(|effort| {
                        effort
                            .get("reasoningEffort")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .ok_or(CodexError::Protocol)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let default_effort = entry
                    .get("defaultReasoningEffort")
                    .and_then(Value::as_str)
                    .ok_or(CodexError::Protocol)?;
                if efforts.is_empty() || !efforts.iter().any(|effort| effort == default_effort) {
                    return Err(CodexError::Protocol);
                }
                models.push(CodexModel {
                    id: id.to_owned(),
                    efforts,
                    default_effort: default_effort.to_owned(),
                    is_default: entry.get("isDefault") == Some(&Value::Bool(true)),
                });
            }

            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
            if models.len() >= 1000 {
                return Err(CodexError::Protocol);
            }
        }

        Ok(models)
    }

    pub(super) async fn select_model(
        &mut self,
        requested_model: Option<&str>,
        requested_effort: Option<&str>,
    ) -> Result<(String, String)> {
        let models = self.available_models().await?;
        let selected = if let Some(id) = requested_model {
            models.iter().find(|model| model.id == id)
        } else {
            models
                .iter()
                .find(|model| model.is_default)
                .or_else(|| models.first())
        }
        .ok_or(CodexError::ModelUnavailable)?;

        let effort = if let Some(effort) = requested_effort {
            selected
                .efforts
                .iter()
                .find(|supported| supported.as_str() == effort)
                .ok_or(CodexError::ModelUnavailable)?
                .clone()
        } else if selected.efforts.iter().any(|effort| effort == "low") {
            "low".to_owned()
        } else {
            selected.default_effort.clone()
        };
        Ok((selected.id.clone(), effort))
    }
}
