//! Isolated process integration scenario with generated public metadata only.
//! No network, media download, live profile or unit-test harness is involved.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use qbutt_content::{
    Category, Library, LibraryError, LibraryView, PublicationRef, Source, Transport, Variant,
    VariantFilters, VariantPreferences, VariantRef, VariantSelector, VideoQuality, Work,
};
use rusqlite::Connection;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--reopen")) {
        let profile = args.next().ok_or("missing isolated profile")?;
        let partial = args.next().as_deref() == Some(std::ffi::OsStr::new("partial"));
        verify_reopened(Path::new(&profile), partial)?;
        return Ok(());
    }

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = std::env::temp_dir().join(format!("qbutt-library-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root)?;
    let result = exercise(&root);
    std::fs::remove_dir_all(&root)?;
    result?;
    println!("reopen=ok partial_metadata=preserved full_metadata=authoritative full_variants=ok");
    println!("exact_refs=validated exact_selection=ok filters=ok favorite_history=independent");
    println!("rejected_update=unchanged sqlite_abort=unchanged corrupt_metadata=rejected");
    println!("unsupported_schema=rejected malformed_schema=rejected unrelated_data=preserved");
    Ok(())
}

fn exercise(root: &Path) -> Result<()> {
    let profile = root.join("profile");
    std::fs::create_dir(&profile)?;
    let unrelated = profile.join("downloaded-payload.txt");
    std::fs::write(&unrelated, b"generated unrelated data")?;
    let legacy = profile.join("library.sqlite");
    std::fs::write(&legacy, b"generated legacy placeholder")?;

    let work = fixture();
    let mut library = Library::open(&profile)?;
    assert!(library.entries(LibraryView::All)?.is_empty());
    library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
    library.save_preferences(
        &qbutt_content::WorkGroup::single(work.clone()),
        preferences(&work, 0),
    )?;
    assert!(library.entries(LibraryView::History)?.is_empty());
    library.record_opened(
        &qbutt_content::WorkGroup::single(work.clone()),
        work.variants[0].reference.clone(),
        preferences(&work, 0),
    )?;
    library.save_preferences(
        &qbutt_content::WorkGroup::single(work.clone()),
        preferences(&work, 1),
    )?;
    drop(library);

    let status = Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(&profile)
        .status()?;
    assert!(
        status.success(),
        "the second process failed to reopen the library"
    );

    let mut library = Library::open(&profile)?;
    let mut partial = work.clone();
    partial.title = "Refreshed generated title".into();
    partial.needs_inspection = true;
    partial.description = None;
    partial.cover = None;
    partial.year = None;
    partial.aliases = vec!["New generated alias".into()];
    partial.identifiers = BTreeMap::from([("new".into(), "2".into())]);
    partial.variants.truncate(1);
    library.set_favorite(&qbutt_content::WorkGroup::single(partial.clone()), false)?;
    let entry = library.get(&work.reference)?.ok_or("missing saved work")?;
    assert!(!entry.favorite);
    assert_eq!(entry.group.anchor_work().unwrap().title, partial.title);
    assert_eq!(entry.group.anchor_work().unwrap().variants, work.variants);
    assert_eq!(entry.preferences, preferences(&work, 1));
    assert_eq!(
        entry.last_opened.as_ref().unwrap().variant,
        work.variants[0].reference
    );
    assert_eq!(library.entries(LibraryView::History)?.len(), 1);
    assert!(library.entries(LibraryView::Favorites)?.is_empty());
    drop(library);
    let status = Command::new(std::env::current_exe()?)
        .arg("--reopen")
        .arg(&profile)
        .arg("partial")
        .status()?;
    assert!(
        status.success(),
        "partial known metadata did not survive process reopen"
    );

    let mut library = Library::open(&profile)?;
    let mut authoritative = partial.clone();
    authoritative.needs_inspection = false;
    authoritative.aliases.clear();
    authoritative.identifiers.clear();
    let replaced = library.set_favorite(
        &qbutt_content::WorkGroup::single(authoritative.clone()),
        false,
    )?;
    assert!(replaced.group.anchor_work().unwrap().description.is_none());
    assert!(replaced.group.anchor_work().unwrap().cover.is_none());
    assert!(replaced.group.anchor_work().unwrap().year.is_none());
    assert!(replaced.group.anchor_work().unwrap().aliases.is_empty());
    assert!(replaced.group.anchor_work().unwrap().identifiers.is_empty());

    library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), true)?;
    let entry = library
        .remove_history(&work.reference)?
        .ok_or("missing saved work")?;
    assert!(entry.favorite);
    assert!(entry.last_opened.is_none());
    assert_eq!(entry.preferences, preferences(&work, 1));
    assert_eq!(entry.group.anchor_work().unwrap().variants, work.variants);
    assert!(library.entries(LibraryView::History)?.is_empty());
    assert_eq!(library.entries(LibraryView::Favorites)?.len(), 1);

    let before = library.get(&work.reference)?.unwrap();
    let mut invalid = preferences(&work, 0);
    invalid.selected.as_mut().unwrap().publication.id = "another-publication".into();
    assert!(matches!(
        library.save_preferences(&qbutt_content::WorkGroup::single(partial.clone()), invalid),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    for id in ["not-a-publication-id", "0", "01"] {
        let mut invalid_work = work.clone();
        invalid_work.reference.id = id.into();
        for variant in &mut invalid_work.variants {
            variant.reference.publication = invalid_work.reference.clone();
        }
        assert!(matches!(
            library.set_favorite(
                &qbutt_content::WorkGroup::single(invalid_work.clone()),
                true
            ),
            Err(LibraryError::InvalidEntry(_))
        ));
        assert_eq!(library.get(&work.reference)?.unwrap(), before);
    }
    let mut zero_torrent = work.clone();
    zero_torrent.variants[1].reference.selector = VariantSelector::Torrent { id: 0 };
    assert!(matches!(
        library.set_favorite(
            &qbutt_content::WorkGroup::single(zero_torrent.clone()),
            false
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let missing = VariantRef {
        publication: work.reference.clone(),
        selector: VariantSelector::Torrent { id: u64::MAX },
    };
    assert!(matches!(
        library.record_opened(
            &qbutt_content::WorkGroup::single(partial.clone()),
            missing,
            preferences(&work, 0)
        ),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let mut private = work.clone();
    private.cover = Some(Url::parse(
        "https://anilibria.top/poster.jpg?token=generated-secret",
    )?);
    assert!(matches!(
        library.set_favorite(&qbutt_content::WorkGroup::single(private.clone()), false),
        Err(LibraryError::InvalidEntry(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);

    let database = profile.join("content-library.sqlite");
    let raw = Connection::open(&database)?;
    raw.execute_batch(
        "CREATE TRIGGER reject_library_update BEFORE UPDATE ON library
         BEGIN SELECT RAISE(ABORT, 'generated integration fault'); END;",
    )?;
    assert!(matches!(
        library.set_favorite(&qbutt_content::WorkGroup::single(partial.clone()), false),
        Err(LibraryError::Sqlite(_))
    ));
    assert_eq!(library.get(&work.reference)?.unwrap(), before);
    raw.execute_batch("DROP TRIGGER reject_library_update;")?;
    drop(library);

    let current_version: i64 = raw.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for version in (1..8).chain(std::iter::once(current_version + 1)) {
        raw.pragma_update(None, "user_version", version)?;
        assert!(matches!(
            Library::open(&profile),
            Err(LibraryError::UnsupportedSchema(actual)) if actual == version
        ));
        assert_eq!(
            raw.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
            version
        );
    }
    raw.pragma_update(None, "user_version", current_version)?;

    raw.execute("UPDATE library SET entry = ?1", ["not JSON"])?;
    let library = Library::open(&profile)?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::Metadata(_))
    ));
    assert!(matches!(
        library.entries(LibraryView::Favorites),
        Err(LibraryError::Metadata(_))
    ));
    let raw_metadata: String = raw.query_row("SELECT entry FROM library", [], |row| row.get(0))?;
    assert_eq!(raw_metadata, "not JSON");
    drop(library);

    let mut corrupt = before.clone();
    corrupt
        .preferences
        .selected
        .as_mut()
        .unwrap()
        .publication
        .id = "another-publication".into();
    let metadata = serde_json::to_string(&corrupt)?;
    raw.execute("UPDATE library SET entry = ?1", [&metadata])?;
    let mut library = Library::open(&profile)?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::CorruptEntry(_))
    ));
    assert!(matches!(
        library.set_favorite(&qbutt_content::WorkGroup::single(work.clone()), false),
        Err(LibraryError::CorruptEntry(_))
    ));
    assert_eq!(
        raw.query_row("SELECT entry FROM library", [], |row| row
            .get::<_, String>(0))?,
        metadata
    );
    drop(library);

    let mut corrupt_id = before.clone();
    corrupt_id.group.publications[0].reference.id = "not-a-publication-id".into();
    let invalid_reference = corrupt_id.group.publications[0].reference.clone();
    for variant in &mut corrupt_id.group.publications[0].variants {
        variant.reference.publication = invalid_reference.clone();
    }
    raw.execute(
        "UPDATE library SET reference = ?1, entry = ?2",
        rusqlite::params![
            serde_json::to_string(&corrupt_id.group.publications[0].reference)?,
            serde_json::to_string(&corrupt_id)?,
        ],
    )?;
    let library = Library::open(&profile)?;
    assert!(matches!(
        library.get(&corrupt_id.group.publications[0].reference),
        Err(LibraryError::CorruptEntry(_))
    ));
    assert!(matches!(
        library.entries(LibraryView::All),
        Err(LibraryError::CorruptEntry(_))
    ));
    drop(library);

    let mut corrupt_torrent = before.clone();
    corrupt_torrent.group = qbutt_content::WorkGroup::single(zero_torrent);
    raw.execute(
        "UPDATE library SET reference = ?1, entry = ?2",
        rusqlite::params![
            serde_json::to_string(&work.reference)?,
            serde_json::to_string(&corrupt_torrent)?,
        ],
    )?;
    let library = Library::open(&profile)?;
    assert!(matches!(
        library.get(&work.reference),
        Err(LibraryError::CorruptEntry(_))
    ));
    drop(library);
    drop(raw);

    verify_malformed_schemas(root, current_version)?;
    let foreign_profile = root.join("foreign-profile");
    std::fs::create_dir(&foreign_profile)?;
    let foreign = Connection::open(foreign_profile.join("content-library.sqlite"))?;
    foreign.execute_batch(
        "CREATE TABLE unrelated (value TEXT); INSERT INTO unrelated VALUES ('generated data');",
    )?;
    assert!(matches!(
        Library::open(&foreign_profile),
        Err(LibraryError::ForeignDatabase)
    ));
    assert_eq!(
        foreign.query_row("SELECT value FROM unrelated", [], |row| row
            .get::<_, String>(0))?,
        "generated data"
    );
    drop(foreign);

    assert_eq!(std::fs::read(unrelated)?, b"generated unrelated data");
    assert_eq!(std::fs::read(legacy)?, b"generated legacy placeholder");
    verify_group_split(root)?;
    Ok(())
}

fn verify_group_split(root: &Path) -> Result<()> {
    let first = fixture();
    let mut second = first.clone();
    second.reference.id = "2".into();
    for variant in &mut second.variants {
        variant.reference.publication = second.reference.clone();
    }
    let group = qbutt_content::WorkGroup {
        anchor: first.reference.clone(),
        references: vec![first.reference.clone(), second.reference.clone()],
        publications: vec![first.clone(), second.clone()],
    };
    let mut changed = first.clone();
    changed.year = Some(first.year.unwrap() + 1);
    let current = qbutt_content::WorkGroup::single(changed.clone());

    for existing_second in [false, true] {
        let profile = root.join(format!("group-split-{existing_second}"));
        let mut library = Library::open(&profile)?;
        library.set_favorite(&group, true)?;
        library.record_opened(
            &group,
            second.variants[0].reference.clone(),
            preferences(&second, 0),
        )?;
        library.save_preferences(&group, preferences(&first, 1))?;
        let previous = library.get(&first.reference)?.unwrap();

        let existing = if existing_second {
            let mut newer = second.clone();
            newer.description = Some("Newer independent saved metadata".into());
            let newer = qbutt_content::WorkGroup::single(newer);
            library.set_favorite(&newer, true)?;
            library.record_opened(
                &newer,
                second.variants[2].reference.clone(),
                preferences(&second, 2),
            )?;
            library.save_preferences(&newer, preferences(&second, 1))?;
            library.get(&second.reference)?
        } else {
            None
        };

        let before = library.entries(LibraryView::All)?;
        let mut invalid = preferences(&changed, 1);
        invalid.selected.as_mut().unwrap().publication.id = "9".into();
        assert!(matches!(
            library.save_preferences(&current, invalid),
            Err(LibraryError::InvalidEntry(_))
        ));
        assert_eq!(library.entries(LibraryView::All)?, before);

        let saved = library.save_preferences(&current, preferences(&changed, 1))?;
        assert!(saved.favorite);
        assert!(saved.last_opened.is_none());
        assert_eq!(saved.group.references, vec![first.reference.clone()]);
        assert_eq!(saved.group.anchor_work().unwrap().year, changed.year);
        let detached = library
            .get(&second.reference)?
            .ok_or("detached history is missing")?;
        if let Some(existing) = existing {
            assert_eq!(detached, existing);
        } else {
            assert!(!detached.favorite);
            assert_eq!(detached.last_opened, previous.last_opened);
            assert_eq!(
                detached.preferences.selected,
                Some(second.variants[0].reference.clone())
            );
            assert_eq!(detached.group.publications, vec![second.clone()]);
        }
        assert_eq!(library.entries(LibraryView::All)?.len(), 2);
        drop(library);
        let reopened = Library::open(&profile)?;
        assert_eq!(reopened.get(&second.reference)?, Some(detached));
    }
    println!(
        "explicit_split=atomic membership=current detached_history=exact existing_intents=retained reopen=ok"
    );
    Ok(())
}

fn verify_malformed_schemas(root: &Path, current_version: i64) -> Result<()> {
    let schemas = [
        "CREATE TABLE library (
            reference TEXT NOT NULL,
            entry TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        ) STRICT;",
        "CREATE TABLE library (
            reference TEXT PRIMARY KEY NOT NULL,
            entry TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );",
        "CREATE TABLE library (
            reference TEXT PRIMARY KEY NOT NULL,
            entry TEXT,
            updated_at INTEGER
        ) STRICT;",
        "CREATE TABLE library (
            reference TEXT PRIMARY KEY NOT NULL,
            entry BLOB NOT NULL,
            updated_at INTEGER NOT NULL
        ) STRICT;",
    ];
    for (index, schema) in schemas.into_iter().enumerate() {
        let malformed_profile = root.join(format!("malformed-schema-{index}"));
        std::fs::create_dir(&malformed_profile)?;
        let malformed = Connection::open(malformed_profile.join("content-library.sqlite"))?;
        malformed.execute_batch(schema)?;
        malformed.pragma_update(None, "application_id", 0x51424c42)?;
        malformed.pragma_update(None, "user_version", current_version)?;
        let before_schema: String = malformed.query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'library'",
            [],
            |row| row.get(0),
        )?;
        assert!(matches!(
            Library::open(&malformed_profile),
            Err(LibraryError::InvalidSchema)
        ));
        let after_schema: String = malformed.query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'library'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(after_schema, before_schema);
    }

    Ok(())
}

fn verify_reopened(profile: &Path, partial: bool) -> Result<()> {
    let mut work = fixture();
    if partial {
        work.title = "Refreshed generated title".into();
        work.needs_inspection = true;
        work.aliases.insert(0, "New generated alias".into());
        work.identifiers.insert("new".into(), "2".into());
    }
    let library = Library::open(profile)?;
    let entry = library
        .get(&work.reference)?
        .ok_or("missing reopened work")?;
    assert_eq!(entry.group.anchor_work().unwrap(), &work);
    assert_eq!(entry.preferences, preferences(&work, 1));
    assert_eq!(entry.favorite, !partial);
    assert_eq!(
        entry.last_opened.as_ref().unwrap().variant,
        work.variants[0].reference
    );
    assert!(entry.last_opened.as_ref().unwrap().at_unix_millis > 0);
    if !partial {
        assert_eq!(
            library.entries(LibraryView::Favorites)?,
            vec![entry.clone()]
        );
    } else {
        assert!(library.entries(LibraryView::Favorites)?.is_empty());
    }
    assert_eq!(library.entries(LibraryView::History)?, vec![entry]);
    println!(
        "second_process: saved Work, all variants, choices, favorite and exact history restored"
    );
    Ok(())
}

fn preferences(work: &Work, index: usize) -> VariantPreferences {
    VariantPreferences {
        selected: Some(work.variants[index].reference.clone()),
        filters: VariantFilters {
            episode: Some("1".into()),
            translation: Some("Generated voice".into()),
            source: Some(Source::AniLiberty),
            language: Some("ru".into()),
            player: Some("Generated player".into()),
            transport: Some(work.variants[index].transport),
            query: Some("Generated".into()),
        },
    }
}

fn fixture() -> Work {
    let reference = PublicationRef {
        source: Source::AniLiberty,
        id: "1".into(),
    };
    let selectors = [
        VariantSelector::Episode {
            id: "generated-episode".into(),
            quality: VideoQuality::P1080,
        },
        VariantSelector::Torrent { id: 42 },
        VariantSelector::Rutube {
            episode_id: "generated-episode".into(),
            rutube_id: "0123456789abcdef0123456789abcdef".into(),
        },
    ];
    let variants = selectors
        .into_iter()
        .map(|selector| {
            let transport = if matches!(selector, VariantSelector::Torrent { .. }) {
                Transport::Torrent
            } else {
                Transport::Video
            };
            Variant {
                reference: VariantRef {
                    publication: reference.clone(),
                    selector,
                },
                title: None,
                transport,
                episode: Some("1".into()),
                quality: Some("1080p".into()),
                translation: Some("Generated voice".into()),
                languages: vec!["ru".into(), "en".into()],
                player: Some("Generated player".into()),
                size: Some(1024),
                seeders: Some(3),
                info_hash: None,
                reported_video_ads: false,
            }
        })
        .collect();
    Work {
        reference,
        title: "Generated catalog fixture".into(),
        category: Category::Anime,
        url: Url::parse("https://anilibria.top/anime/releases/release/generated-fixture").unwrap(),
        cover: Some(Url::parse("https://anilibria.top/generated-poster.jpg").unwrap()),
        description: Some("Legal generated metadata without acquisition URLs".into()),
        year: Some(2026),
        aliases: vec!["Generated alias".into()],
        identifiers: BTreeMap::from([("generated".into(), "1".into())]),
        variants,
        needs_inspection: false,
    }
}
