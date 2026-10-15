//! Generated search labels, descriptive identity and late-metadata association.

use qbutt_content::{SharedWork, Source, Work, WorkAssociation};
use url::Url;

use super::fixture::Fixture;
use super::{Category, CheckResult, Rutube};

pub(super) async fn verify(fixture: &Fixture, source: &Rutube) -> CheckResult<()> {
    fixture.case("search-relevance");
    for (query, expected) in [
        ("generated story", vec![0, 1, 2, 3, 4, 17, 28, 29]),
        ("generated story opening", vec![5, 6]),
        ("generated story опенинг", vec![7]),
        ("generated story amv", vec![8, 9]),
        ("generated story edit", vec![4, 8, 10]),
        ("generated story ending", vec![11]),
        ("generated story эндинг", vec![12]),
        ("generated story амв", vec![13]),
        ("generated story эдит", vec![14]),
        ("generated story trailer", vec![15]),
        ("generated story обзор", vec![16]),
        ("generated story clip", vec![17, 18, 19]),
        ("generated story клип", vec![20]),
        ("generated story recap", vec![21]),
        ("generated story пересказ", vec![22]),
        ("generated story slowed", vec![23]),
        ("generated story за 48 минут", vec![24]),
        ("generated story за 10 минут", vec![25]),
        ("generated story за 11 минут", vec![26]),
        ("generated story in 8 minutes", vec![27]),
    ] {
        let rows = source.search(query, Category::Anime).await?;
        let ids: Vec<_> = rows.iter().map(|work| work.reference.id.clone()).collect();
        let expected: Vec<_> = expected.iter().map(|id| format!("{id:032x}")).collect();
        assert_eq!(ids, expected, "search relevance changed for {query}");
    }

    fixture.case("normal");
    let shallow = source
        .search("legal sample", Category::Movies)
        .await?
        .remove(0);
    let mut unknown: Work = serde_json::from_value(serde_json::to_value(&shallow)?)?;
    unknown.description = None;
    let mut catalogue = unknown.clone();
    catalogue.reference.source = Source::RuTor;
    catalogue.reference.id = "1".into();
    catalogue.url = Url::parse("https://rutor.info/torrent/1/")?;
    catalogue.description = Some("The complete generated film".into());
    catalogue.variants.clear();

    fixture.case("trailer-announcement");
    assert!(
        source
            .search("legal sample", Category::Movies)
            .await?
            .is_empty()
    );
    let explicit = source
        .search("legal sample трейлер", Category::Movies)
        .await?;
    assert_eq!(explicit.len(), 1);
    assert_eq!(explicit[0].title, shallow.title);
    for query in ["unrelated трейлер", "legal sample trailer"] {
        assert!(source.search(query, Category::Movies).await?.is_empty());
    }
    let inspected = source.inspect(&shallow.reference).await?;
    assert_eq!(inspected.title, shallow.title);
    assert_eq!(inspected.reference, shallow.reference);
    source.resolve(&shallow.variants[0].reference).await?;
    let query = format!("https://rutube.ru/video/{}/", inspected.reference.id);
    let exact = source.search(&query, Category::Movies).await?;
    assert_eq!(exact[0], inspected);
    let promotional: Work = serde_json::from_value(serde_json::to_value(&inspected)?)?;

    for reverse in [false, true] {
        let mut publications = [unknown.clone(), catalogue.clone()];
        if reverse {
            publications.reverse();
        }
        let mut association = WorkAssociation::default();
        for work in publications {
            association.update(work);
        }
        assert_eq!(association.groups().len(), 1);

        association.update(promotional.clone());
        let groups = association.groups();
        assert_eq!(
            groups.len(),
            2,
            "late promotional identity must split the group"
        );
        let trailer = groups
            .iter()
            .find(|group| group.publication(&promotional.reference).is_some())
            .unwrap();
        assert_eq!(
            trailer.publications.as_slice(),
            std::slice::from_ref(&promotional)
        );
        let full = groups
            .iter()
            .find(|group| group.publication(&catalogue.reference).is_some())
            .unwrap();
        assert_eq!(
            full.publications.as_slice(),
            std::slice::from_ref(&catalogue)
        );
        assert!(groups.iter().all(|group| group.is_confirmed()));

        let shared = SharedWork::new(
            trailer.references.clone(),
            Some(promotional.variants[0].reference.clone()),
        )?;
        assert_eq!(SharedWork::from_url(&shared.to_url()?)?, shared);
    }

    for case in ["trailer-link", "trailer-reference"] {
        fixture.case(case);
        assert_eq!(
            source.search("legal sample", Category::Movies).await?.len(),
            1
        );
        assert!(source.search("example", Category::Movies).await?.is_empty());
        let work = source.inspect(&shallow.reference).await?;
        let work: Work = serde_json::from_value(serde_json::to_value(work)?)?;
        let mut association = WorkAssociation::default();
        association.update(catalogue.clone());
        association.update(work);
        assert_eq!(
            association.groups().len(),
            1,
            "a trailer reference is not video identity"
        );
    }
    fixture.case("normal");
    println!(
        "PASS rutube_relevance ordinary_search=labelled_promotions_excluded explicit_clips=retained leading_announcement=identity trailer_links=retained late_metadata=split_both_orders direct_and_share=exact"
    );
    Ok(())
}
