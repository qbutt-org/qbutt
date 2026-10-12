//! Generated public metadata exercises RuTor persistence through real SQLite.
//! No network, torrent payload, live profile or unit-test harness is involved.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, Library, LibraryError, LibraryView, PublicationRef, Source, Transport, Variant,
    VariantFilters, VariantPreferences, VariantRef, VariantSelector, Work,
};
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
const CATEGORIES: [Category; 6] = [
    Category::Movies,
    Category::Tv,
    Category::Anime,
    Category::Games,
    Category::Software,
    Category::Books,
];

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        let profile = arguments.next().ok_or("missing isolated profile")?;
        return verify_reopened(Path::new(&profile));
    }

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!(
        "qbutt-rutor-library-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&root)?;
    let result = exercise(&root);
    std::fs::remove_dir_all(&root)?;
    result?;
    println!("rutor_categories=6 exact_hash=preserved version_title=preserved reopen=ok");
    println!(
        "invalid_source_category_selector_transport_hash=rejected state=unchanged history=empty"
    );
    Ok(())
}

fn exercise(profile: &Path) -> Result<()> {
    let mut library = Library::open(profile)?;
    for (index, category) in CATEGORIES.into_iter().enumerate() {
        let work = fixture((index + 1) as u64, category);
        library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
        library.save_preferences(
            &qbutt_content::WorkGroup::single(work.clone()),
            preferences(&work),
        )?;
    }
    assert!(library.entries(LibraryView::History)?.is_empty());
    drop(library);

    let status = Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(profile)
        .status()?;
    assert!(
        status.success(),
        "the second process failed to reopen RuTor entries"
    );

    let mut library = Library::open(profile)?;
    let before = library.entries(LibraryView::All)?;
    let work = fixture(5, Category::Software);
    let mut invalid = Vec::new();
    let mut wrong_category = work.clone();
    wrong_category.category = Category::Manga;
    invalid.push(wrong_category);

    let mut wrong_source = work.clone();
    wrong_source.reference.source = Source::AniLiberty;
    wrong_source.variants[0].reference.publication = wrong_source.reference.clone();
    invalid.push(wrong_source);

    let mut wrong_selector = work.clone();
    wrong_selector.variants[0].reference.selector = VariantSelector::Torrent { id: 42 };
    invalid.push(wrong_selector);

    let mut wrong_transport = work.clone();
    wrong_transport.variants[0].transport = Transport::Video;
    invalid.push(wrong_transport);

    for hash in [
        None,
        Some("abcdef0123456789abcdef0123456789abcdef0123"),
        Some("invalid"),
    ] {
        let mut wrong_hash = work.clone();
        wrong_hash.variants[0].info_hash = hash.map(str::to_owned);
        invalid.push(wrong_hash);
    }
    let mut noncanonical_hash = work.clone();
    noncanonical_hash.variants[0].reference.selector = VariantSelector::TorrentHash {
        info_hash: HASH.to_ascii_uppercase(),
    };
    noncanonical_hash.variants[0].info_hash = Some(HASH.to_ascii_uppercase());
    invalid.push(noncanonical_hash);

    for rejected in invalid {
        assert!(matches!(
            library.set_favorite(&qbutt_content::WorkGroup::single(rejected.clone()), false),
            Err(LibraryError::InvalidEntry(_))
        ));
        assert_eq!(library.entries(LibraryView::All)?, before);
    }
    let mut rejected_preferences = preferences(&work);
    rejected_preferences.selected.as_mut().unwrap().selector = VariantSelector::TorrentHash {
        info_hash: "abcdef0123456789abcdef0123456789abcdef0123".into(),
    };
    assert!(matches!(
        library.save_preferences(
            &qbutt_content::WorkGroup::single(work.clone()),
            rejected_preferences
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.entries(LibraryView::All)?, before);

    let mut refreshed = work.clone();
    refreshed.title = "Generated tool 2.0 [Windows]".into();
    library.set_favorite(&qbutt_content::WorkGroup::single(refreshed.clone()), true)?;
    let entry = library
        .get(&work.reference)?
        .ok_or("missing refreshed entry")?;
    assert_eq!(entry.group.anchor_work().unwrap(), &refreshed);
    assert_eq!(entry.preferences, preferences(&work));
    assert!(entry.last_opened.is_none());
    assert!(library.entries(LibraryView::History)?.is_empty());
    Ok(())
}

fn verify_reopened(profile: &Path) -> Result<()> {
    let library = Library::open(profile)?;
    assert_eq!(
        library.entries(LibraryView::Favorites)?.len(),
        CATEGORIES.len()
    );
    assert!(library.entries(LibraryView::History)?.is_empty());
    for (index, category) in CATEGORIES.into_iter().enumerate() {
        let work = fixture((index + 1) as u64, category);
        let entry = library
            .get(&work.reference)?
            .ok_or("missing reopened entry")?;
        assert_eq!(entry.group.anchor_work().unwrap(), &work);
        assert_eq!(entry.preferences, preferences(&work));
        assert!(entry.favorite && entry.last_opened.is_none());
    }
    Ok(())
}

fn preferences(work: &Work) -> VariantPreferences {
    VariantPreferences {
        selected: Some(work.variants[0].reference.clone()),
        filters: VariantFilters {
            source: Some(Source::RuTor),
            transport: Some(Transport::Torrent),
            ..VariantFilters::default()
        },
    }
}

fn fixture(id: u64, category: Category) -> Work {
    let reference = PublicationRef {
        source: Source::RuTor,
        id: id.to_string(),
    };
    Work {
        reference: reference.clone(),
        title: "Generated tool 1.0 [Windows]".into(),
        category,
        url: Url::parse(&format!("https://rutor.info/torrent/{id}")).unwrap(),
        cover: None,
        description: None,
        year: None,
        aliases: Vec::new(),
        identifiers: BTreeMap::new(),
        variants: vec![Variant {
            reference: VariantRef {
                publication: reference,
                selector: VariantSelector::TorrentHash {
                    info_hash: HASH.into(),
                },
            },
            title: Some("Generated tool 1.0 [Windows]".into()),
            transport: Transport::Torrent,
            episode: None,
            quality: None,
            translation: None,
            languages: Vec::new(),
            player: None,
            size: Some(1024),
            seeders: Some(3),
            info_hash: Some(HASH.into()),
            reported_video_ads: false,
        }],
        needs_inspection: false,
    }
}
