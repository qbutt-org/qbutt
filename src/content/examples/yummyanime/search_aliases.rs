//! Bounded search metadata uses the same generated TLS server as inspection.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;
use url::Url;

use super::{CheckResult, Work, yummyanime};

#[derive(Default)]
pub(super) struct State {
    requests: Mutex<Vec<u64>>,
    started: Notify,
    closed: Notify,
}

impl State {
    fn take_requests(&self) -> Vec<u64> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

pub(super) async fn exercise(catalog: &yummyanime::YummyAnime, state: &State) -> CheckResult<()> {
    let mut batches = Vec::new();
    let works = catalog
        .search_with_updates("Aliases", |works| batches.push(works.to_vec()))
        .await?;
    assert_eq!(batches[0].len(), 40, "raw batch was held for metadata");
    assert!(
        batches[0]
            .iter()
            .take(6)
            .all(|work| work.aliases.is_empty())
    );
    assert_eq!(state.take_requests(), [101, 102, 103, 104]);
    assert_eq!(batches.len(), 5);
    assert!(
        works
            .iter()
            .all(|work| work.needs_inspection && work.variants.is_empty())
    );
    assert!(
        works
            .iter()
            .find(|work| work.reference.id == "105")
            .unwrap()
            .aliases
            .is_empty()
    );
    let inspected = catalog.inspect(&works[1].reference).await?;
    association_orders(&batches[0], &works, &inspected)?;

    let guarded = catalog.search("Guards").await?;
    assert_eq!(state.take_requests(), [201, 202, 203, 204]);
    assert!(guarded.iter().all(|work| work.aliases.is_empty()));
    assert!(guarded.iter().all(|work| work.year == Some(2016)));

    let mut partial_batches = Vec::new();
    let partial = catalog
        .search_with_updates("Partial", |works| partial_batches.push(works.to_vec()))
        .await?;
    assert_eq!(state.take_requests(), [301, 302, 303]);
    assert_eq!(partial_batches.len(), 3);
    assert!(!partial[0].aliases.is_empty());
    assert!(partial[1].aliases.is_empty());
    assert!(!partial[2].aliases.is_empty());

    let started = Instant::now();
    let mut budget_batches = Vec::new();
    let budget = catalog
        .search_with_updates("Budget", |works| budget_batches.push(works.to_vec()))
        .await?;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "metadata exceeded total budget"
    );
    tokio::time::timeout(Duration::from_secs(2), state.closed.notified()).await?;
    assert_eq!(state.take_requests(), [401, 402]);
    assert_eq!(budget_batches.len(), 2);
    assert!(!budget[0].aliases.is_empty());
    assert!(budget[1..].iter().all(|work| work.aliases.is_empty()));
    // Consume the start signal from the preceding bounded timeout.
    tokio::time::timeout(Duration::from_secs(2), state.started.notified()).await?;

    let mut raw_seen = false;
    let mut cancelled = Box::pin(catalog.search_with_updates("Cancellation", |works| {
        assert_eq!(works.len(), 2);
        raw_seen = true;
    }));
    tokio::select! {
        _ = state.started.notified() => {},
        result = &mut cancelled => panic!("metadata finished before cancellation: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("metadata request did not start"),
    }
    drop(cancelled);
    assert!(
        raw_seen,
        "initial rows were not published before delayed metadata"
    );
    tokio::time::timeout(Duration::from_secs(2), state.closed.notified()).await?;
    assert_eq!(
        state.take_requests(),
        [501],
        "cancelled search started another metadata request"
    );
    println!(
        "PASS yummyanime-search-aliases raw_immediate=40 metadata_limit=4 admitted_only=true budget=3s partial_failure=success cancellation=connection_closed identity_conflicts=id_year_catalog_title arrival_orders=8 late_search_preserves_inspection=true fresh_inspection_splits=true new_generation_accepts_raw=true"
    );
    Ok(())
}

fn association_orders(raw: &[Work], enriched: &[Work], inspected: &Work) -> CheckResult<()> {
    use qbutt_content::{PublicationRef, Source, WorkAssociation};

    let convert = |work: &Work| -> CheckResult<qbutt_content::Work> {
        Ok(serde_json::from_value(serde_json::to_value(work)?)?)
    };
    let original = convert(raw.iter().find(|work| work.reference.id == "101").unwrap())?;
    let detailed = convert(
        enriched
            .iter()
            .find(|work| work.reference.id == "101")
            .unwrap(),
    )?;
    let director = convert(
        enriched
            .iter()
            .find(|work| work.reference.id == "102")
            .unwrap(),
    )?;
    let season_two = convert(
        enriched
            .iter()
            .find(|work| work.reference.id == "103")
            .unwrap(),
    )?;
    let mut spirit = original.clone();
    spirit.reference = PublicationRef {
        source: Source::AnimeSpirit,
        id: "11037".into(),
    };
    spirit.title = "Generated Animation".into();
    spirit.url = "https://my.animespirit.ru/anime/11037-generated-animation.html".parse()?;
    spirit.identifiers.clear();

    for reverse in [false, true] {
        for offset in 0..4 {
            let mut ordered = [
                original.clone(),
                director.clone(),
                season_two.clone(),
                spirit.clone(),
            ];
            if reverse {
                ordered.reverse();
            }
            ordered.rotate_left(offset);
            let mut association = WorkAssociation::default();
            for work in ordered {
                association.update_search(work);
            }
            assert_eq!(association.groups().len(), 4);
            association.update_search(detailed.clone());
            let groups = association.groups();
            assert_eq!(groups.len(), 3);
            let original_group = groups
                .iter()
                .find(|group| group.references.contains(&original.reference))
                .unwrap();
            assert_eq!(original_group.references.len(), 2);
            assert!(original_group.references.contains(&spirit.reference));
            assert!(original_group.is_confirmed());
            for separate in [&director, &season_two] {
                assert_eq!(
                    groups
                        .iter()
                        .find(|group| group.references.contains(&separate.reference))
                        .unwrap()
                        .references
                        .len(),
                    1
                );
            }
        }
    }

    let inspected = convert(inspected)?;
    let selected = inspected.variants[1].reference.clone();
    let mut association = WorkAssociation::default();
    association.update_search(original.clone());
    association.update(spirit.clone());
    association.update(inspected.clone());
    // Late progress and the returned final batch both carry pending metadata.
    association.update_search(detailed.clone());
    association.update_search(detailed.clone());
    let groups = association.groups();
    let saved = groups
        .iter()
        .find_map(|group| group.publication(&original.reference))
        .unwrap();
    assert_eq!(saved, &inspected);
    assert_eq!(saved.variants[1].reference, selected);
    let mut changed = inspected;
    changed.year = Some(2020);
    association.update(changed.clone());
    assert_eq!(
        association.groups().len(),
        2,
        "fresh conflicting inspection was suppressed"
    );
    assert_eq!(
        association
            .groups()
            .iter()
            .find_map(|group| group.publication(&changed.reference)),
        Some(&changed)
    );
    association.clear();
    association.update_search(original.clone());
    assert_eq!(association.groups()[0].publications, [original]);
    Ok(())
}

pub(super) async fn response(
    state: &State,
    url: &Url,
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Option<Value> {
    if let Some(id) = url
        .path()
        .strip_prefix("/anime/")
        .and_then(|id| id.parse::<u64>().ok())
    {
        if !(101..=502).contains(&id) {
            return None;
        }
        if url.query() == Some("need_videos=true") {
            assert_eq!(id, 101);
            let mut full = anime(id);
            full["other_titles"] = json!(["Generated Animation"]);
            full["videos"] = super::anime(42)["videos"].clone();
            return Some(json!({"response": full}));
        }
        assert_eq!(url.query(), Some("need_videos=false"));
        state.requests.lock().unwrap().push(id);
        if matches!(id, 402 | 501) {
            state.started.notify_one();
            let mut byte = [0];
            if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                state.closed.notify_one();
            }
            return Some(Value::Null);
        }
        if id == 302 {
            return Some(json!({"invalid": "optional metadata refused"}));
        }
        let mut work = anime(id);
        work["other_titles"] = json!(["Generated Animation"]);
        match id {
            201 => work["anime_id"] = json!(999),
            202 => work["year"] = json!(2020),
            203 => work["remote_ids"]["myanimelist_id"] = json!(999),
            204 => work["title"] = json!("Another product"),
            _ => {}
        }
        return Some(json!({"response": work}));
    }
    let query = url.query_pairs().find(|(key, _)| key == "q")?.1;
    let ids: Vec<u64> = match query.as_ref() {
        "Aliases" => [100, 101, 102, 103, 104, 105, 110]
            .into_iter()
            .chain(1000..1033)
            .collect(),
        "Guards" => (201..=204).collect(),
        "Partial" => (301..=303).collect(),
        "Budget" => (401..=403).collect(),
        "Cancellation" => (501..=502).collect(),
        _ => return None,
    };
    Some(json!({"response": ids.into_iter().map(anime).collect::<Vec<_>>()}))
}

fn anime(id: u64) -> Value {
    let prefix = match id {
        101..=110 => "Aliases",
        201..=204 => "Guards",
        301..=303 => "Partial",
        401..=403 => "Budget",
        501..=502 => "Cancellation",
        _ => "Unrelated",
    };
    let (title, year, mal) = match id {
        102 => ("Aliases Original: Director's Cut".into(), 2020, 31240),
        103 => ("Aliases Original 2".into(), 2020, 39587),
        _ => (format!("{prefix} Original {id}"), 2016, 31240),
    };
    let mut work = json!({
        "anime_id": id,
        "anime_url": format!("generated-alias-{id}"),
        "title": title,
        "year": year,
        "remote_ids": {"myanimelist_id": mal}
    });
    if id == 110 {
        work["other_titles"] = json!(["Already reported"]);
    }
    work
}
