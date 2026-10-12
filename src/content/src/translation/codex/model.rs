use serde_json::{Value, json};

use super::{Client, CodexError, Result};

impl Client {
    pub(super) async fn select_model(
        &mut self,
        requested_model: Option<&str>,
        requested_effort: Option<&str>,
        image: bool,
    ) -> Result<(String, String)> {
        let mut cursor: Option<String> = None;
        let mut models = Vec::new();
        loop {
            let result = self
                .request(
                    "model/list",
                    json!({
                        "limit": 100, "includeHidden": false, "cursor": cursor
                    }),
                )
                .await?;
            models.extend(
                result
                    .get("data")
                    .and_then(Value::as_array)
                    .ok_or(CodexError::Protocol)?
                    .iter()
                    .cloned(),
            );
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() || models.len() >= 1000 {
                break;
            }
        }
        let model = models
            .iter()
            .find(|entry| {
                let selected = requested_model.is_some_and(|name| {
                    entry.get("model").and_then(Value::as_str) == Some(name)
                        || entry.get("id").and_then(Value::as_str) == Some(name)
                });
                let default =
                    requested_model.is_none() && entry.get("isDefault") == Some(&Value::Bool(true));
                (selected || default)
                    && (!image
                        || entry
                            .get("inputModalities")
                            .and_then(Value::as_array)
                            .is_some_and(|items| items.contains(&json!("image"))))
            })
            .or_else(|| {
                if requested_model.is_none() {
                    models.iter().find(|entry| {
                        !image
                            || entry
                                .get("inputModalities")
                                .and_then(Value::as_array)
                                .is_some_and(|items| items.contains(&json!("image")))
                    })
                } else {
                    None
                }
            })
            .ok_or(CodexError::ModelUnavailable)?;
        let name = model
            .get("model")
            .and_then(Value::as_str)
            .ok_or(CodexError::Protocol)?
            .to_owned();
        let supported = model
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .ok_or(CodexError::Protocol)?;
        let preferred = requested_effort.unwrap_or("low");
        let effort = if supported
            .iter()
            .any(|entry| entry.get("reasoningEffort") == Some(&json!(preferred)))
        {
            preferred.to_owned()
        } else if requested_effort.is_none() {
            model
                .get("defaultReasoningEffort")
                .and_then(Value::as_str)
                .ok_or(CodexError::Protocol)?
                .to_owned()
        } else {
            return Err(CodexError::ModelUnavailable);
        };
        Ok((name, effort))
    }
}
