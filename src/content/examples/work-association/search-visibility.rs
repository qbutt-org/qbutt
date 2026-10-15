//! Confirmed empty search groups and exact retained snapshots; no network.

use qbutt_content::{Category, Source, WorkAssociation};

pub(super) fn run() {
    let ready = super::torrent(
        Source::RuTor,
        700,
        "Generated work",
        Category::Games,
        &"a".repeat(40),
    );
    let mut pending = ready.clone();
    pending.reference.id = "701".into();
    pending.variants.clear();
    pending.needs_inspection = true;

    let mut association = WorkAssociation::default();
    association.update_search(pending.clone());
    assert_eq!(association.search_groups("Generated work").len(), 1);
    association.confirm_empty_inspection(&pending.reference);
    assert!(association.search_groups("Generated work").is_empty());
    assert_eq!(
        association.groups().len(),
        1,
        "raw exact snapshot remains known"
    );
    association.update_search(pending.clone());
    assert!(
        association.search_groups("Generated work").is_empty(),
        "late pending row cannot resurrect confirmed empty"
    );

    association.update_search(ready.clone());
    let groups = association.search_groups("Generated work");
    assert_eq!(groups.len(), 1, "another source retains the group");
    assert_eq!(groups[0].variants(None).len(), 1);
    assert_eq!(
        groups[0].publications.len(),
        2,
        "empty publication is retained in the group"
    );
    association.confirm_empty_inspection(&ready.reference);
    assert!(association.search_groups("Generated work").is_empty());
    association.update(ready);
    assert_eq!(
        association.search_groups("Generated work").len(),
        1,
        "fresh successful inspection restores availability"
    );

    association.clear();
    association.update_search(pending.clone());
    assert_eq!(
        association.search_groups("Generated work").len(),
        1,
        "new search resets the confirmation"
    );
    pending.needs_inspection = false;
    association.update(pending);
    assert!(
        association.search_groups("Generated work").is_empty(),
        "successful empty inspection is also hidden"
    );
    println!(
        "search_visibility=pending_visible confirmed_empty_hidden raw_snapshot_retained late_pending_ignored mixed_group_retained fresh_success_and_generation_restored"
    );
}
