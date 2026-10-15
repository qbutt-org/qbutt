//! Read-only search/inspection through the production RuTor, LimeTorrents and
//! association paths. Only public publication metadata is requested.

use qbutt_content::{Category, LimeTorrents, RuTor, Source, Work, WorkAssociation};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args().nth(1).ok_or("provide a search query")?;
    let rutor = RuTor::new()?;
    let lime = LimeTorrents::new()?;
    let (rutor_works, lime_works) = tokio::try_join!(
        rutor.search(&query, Category::Games),
        lime.search(&query, Category::Games),
    )?;
    let publications: Vec<_> = rutor_works.into_iter().chain(lime_works).collect();
    let mut association = WorkAssociation::default();
    for work in &publications {
        print_publication("SEARCH", work);
        association.update(work.clone());
    }
    print_groups("SEARCH", &association);

    for publication in &publications {
        let work = match publication.reference.source {
            Source::RuTor => rutor.inspect(&publication.reference).await?,
            Source::LimeTorrents => lime.inspect(&publication.reference).await?,
            _ => unreachable!(),
        };
        print_publication("INSPECT", &work);
        association.update(work);
    }
    print_groups("INSPECT", &association);
    Ok(())
}

fn print_publication(stage: &str, work: &Work) {
    println!(
        "{stage} source={} id={} year={:?} title={}",
        work.reference.source.id(),
        work.reference.id,
        work.year,
        work.title
    );
}

fn print_groups(stage: &str, association: &WorkAssociation) {
    let groups = association.groups();
    println!("{stage} groups={}", groups.len());
    for group in groups {
        println!(
            "{stage} group={} publications={} variants={} confirmed={}",
            group.display_title(),
            group.publications.len(),
            group.variants(None).len(),
            group.is_confirmed()
        );
    }
}
