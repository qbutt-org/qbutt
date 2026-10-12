use scraper::{Html, Selector};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use crate::{Error, Result};

#[derive(Deserialize)]
pub(super) struct NodeRef {
    #[serde(rename = "__ref")]
    pub(super) key: String,
}

pub(super) struct State {
    values: Map<String, Value>,
}

impl State {
    pub(super) fn parse(document: &Html) -> Result<Self> {
        let selector = Selector::parse("script#__NEXT_DATA__").expect("fixed state selector");
        let mut scripts = document.select(&selector);
        let script = scripts
            .next()
            .ok_or(Error::InvalidResponse("missing Kakuyomu state"))?;
        if scripts.next().is_some() {
            return Err(Error::InvalidResponse("duplicate Kakuyomu state"));
        }
        let json = script.text().collect::<String>();
        let mut data: Value = serde_json::from_str(&json).map_err(Error::Json)?;
        let values = data
            .pointer_mut("/props/pageProps/__APOLLO_STATE__")
            .and_then(Value::as_object_mut)
            .ok_or(Error::InvalidResponse("missing Kakuyomu Apollo state"))?;
        Ok(Self {
            values: std::mem::take(values),
        })
    }

    pub(super) fn record<T: DeserializeOwned>(&self, node: &NodeRef, kind: &str) -> Result<T> {
        let record = self
            .values
            .get(&node.key)
            .ok_or(Error::InvalidResponse("missing Kakuyomu state record"))?;
        if record.get("__typename").and_then(Value::as_str) != Some(kind) {
            return Err(Error::InvalidResponse("unexpected Kakuyomu record type"));
        }
        if let Some(id) = record.get("id").and_then(Value::as_str)
            && node.key != format!("{kind}:{id}")
        {
            return Err(Error::InvalidResponse("changed Kakuyomu state identity"));
        }
        serde_json::from_value(record.clone()).map_err(Error::Json)
    }

    pub(super) fn search_nodes(&self, query: &str) -> Result<Vec<NodeRef>> {
        let root = self
            .values
            .get("ROOT_QUERY")
            .and_then(Value::as_object)
            .ok_or(Error::InvalidResponse("missing Kakuyomu search root"))?;
        let mut searches = root
            .iter()
            .filter(|(key, _)| key.starts_with("searchWorks("));
        let (key, connection) = searches
            .next()
            .ok_or(Error::InvalidResponse("missing Kakuyomu search results"))?;
        if searches.next().is_some() {
            return Err(Error::InvalidResponse("ambiguous Kakuyomu search results"));
        }
        let arguments = key
            .strip_prefix("searchWorks(")
            .and_then(|key| key.strip_suffix(')'))
            .ok_or(Error::InvalidResponse("invalid Kakuyomu search arguments"))?;
        let arguments: Value = serde_json::from_str(arguments).map_err(Error::Json)?;
        if arguments.get("query").and_then(Value::as_str) != Some(query)
            || arguments.get("offset").and_then(Value::as_u64) != Some(0)
            || connection.get("__typename").and_then(Value::as_str) != Some("SearchWorkConnection")
        {
            return Err(Error::InvalidResponse("changed Kakuyomu search identity"));
        }
        let nodes: Vec<NodeRef> = serde_json::from_value(
            connection
                .get("nodes")
                .cloned()
                .ok_or(Error::InvalidResponse("missing Kakuyomu search nodes"))?,
        )
        .map_err(Error::Json)?;
        let total = connection
            .get("totalCount")
            .and_then(Value::as_u64)
            .ok_or(Error::InvalidResponse("missing Kakuyomu search count"))?;
        if nodes.len() > 60 || total < nodes.len() as u64 || (total == 0) != nodes.is_empty() {
            return Err(Error::InvalidResponse("invalid Kakuyomu search count"));
        }
        Ok(nodes)
    }
}
