//! External consumer of the production script library, without test-hooks.
//!
//! Arguments are `draft:<id>` (refused through the public queue entry point),
//! `unavailable:<id>` (a row the index marks unplayable on this server, refused
//! with its own reason) and `qualified:<id>` (admitted through the same entry
//! point). The caller reads the three lists from `index.json`. This program
//! requires that every row of the bundled production index is in exactly one
//! list and that no list names a row the index lacks, so a new row that nobody
//! classified fails here.
use script::quester::queue::{
    Queue, QueueConfigErrorKind, QueueSettings, ReleasePathStatus, UNQUALIFIED_PATH_REASON,
};
use script::quester::registry::PathRegistry;
use std::collections::BTreeSet;

fn main() {
    let registry = PathRegistry::Bundled;
    let mut drafts = Vec::new();
    let mut qualified = Vec::new();
    let mut unavailable = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.split_once(':') {
            Some(("draft", id)) if !id.is_empty() => drafts.push(id.to_string()),
            Some(("qualified", id)) if !id.is_empty() => qualified.push(id.to_string()),
            Some(("unavailable", id)) if !id.is_empty() => unavailable.push(id.to_string()),
            _ => panic!(
                "expected draft:<id>, unavailable:<id> or qualified:<id>, got {argument:?}"
            ),
        }
    }

    every_index_row_is_listed_once(&registry, &drafts, &qualified, &unavailable);
    let admitted = admit_qualified(&registry, &qualified);
    let refused = refuse_drafts(&registry, &drafts);
    let blocked = refuse_unavailable(&registry, &unavailable);
    assert!(refused > 0, "the refusal cell must name at least one draft");
    println!(
        "release-refusal: {refused} mapped drafts refused; {blocked} unavailable rows refused; {admitted} qualified guides admitted; {} index rows",
        registry.index().paths.len()
    );
}

/// Refused plus admitted is the whole index: a row in none of the three lists,
/// in two of them, or a listed id the index lacks, fails the cell.
fn every_index_row_is_listed_once(
    registry: &PathRegistry,
    drafts: &[String],
    qualified: &[String],
    unavailable: &[String],
) {
    let index: BTreeSet<&str> = registry
        .index()
        .paths
        .iter()
        .map(|path| path.id.as_str())
        .collect();
    let mut listed = BTreeSet::new();
    for (kind, ids) in [
        ("draft", drafts),
        ("qualified", qualified),
        ("unavailable", unavailable),
    ] {
        for id in ids {
            assert!(
                index.contains(id.as_str()),
                "{kind}:{id} is not a row of the release index"
            );
            assert!(
                listed.insert(id.as_str()),
                "{id} is named in more than one list, or twice"
            );
        }
    }
    let unlisted: Vec<&str> = index.difference(&listed).copied().collect();
    assert!(
        unlisted.is_empty(),
        "release index rows in no list (neither refused nor admitted): {unlisted:?}"
    );
}

/// Every guide whose index status is qualified is selectable by the production
/// entry point, by explicit pick, by explicit priority and by the default queue.
fn admit_qualified(registry: &PathRegistry, expected: &[String]) -> usize {
    let index = registry.index();
    let in_index: BTreeSet<&str> = index
        .paths
        .iter()
        .filter(|path| path.status == Some(ReleasePathStatus::Qualified))
        .map(|path| path.id.as_str())
        .collect();
    let named: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
    assert_eq!(
        named.len(),
        expected.len(),
        "a qualified id is named twice: {expected:?}"
    );
    assert_eq!(
        in_index, named,
        "the production index and index.json disagree on the qualified guides"
    );
    assert!(
        named.contains("priest"),
        "the qualified Priest Path must stay selectable"
    );

    for id in expected {
        let path = index
            .paths
            .iter()
            .find(|path| &path.id == id)
            .unwrap_or_else(|| panic!("qualified {id:?} is absent from the release index"));
        assert!(
            path.unavailable.is_none(),
            "{id}: a qualified guide must be available"
        );
        for field in ["quests", "order_override"] {
            let mut settings = QueueSettings {
                include_draft_guides: true,
                ..QueueSettings::default()
            };
            if field == "quests" {
                settings.quests.push(id.clone());
            } else {
                settings.order_override.push(id.clone());
            }
            let queue = Queue::from_registry(registry.clone(), settings)
                .unwrap_or_else(|error| panic!("{id}: {field} refused a qualified guide: {error}"));
            let candidate = queue
                .next_candidate()
                .unwrap_or_else(|| panic!("{id}: {field} left no candidate"));
            assert_eq!(queue.id(candidate), Some(id.as_str()), "{field}");
            println!("release-admission id={id} field={field}");
        }
    }

    let queue = Queue::from_registry(
        registry.clone(),
        QueueSettings {
            include_draft_guides: true,
            ..QueueSettings::default()
        },
    )
    .expect("the default release queue must build");
    let picked: BTreeSet<&str> = queue
        .rows()
        .iter()
        .filter(|row| row.picked)
        .map(|row| index.paths[usize::from(row.index)].id.as_str())
        .collect();
    assert_eq!(
        picked, named,
        "the default release queue must pick exactly the qualified guides"
    );
    expected.len()
}

/// Every mapped draft is refused with the typed reason, even when saved
/// settings request draft inclusion.
fn refuse_drafts(registry: &PathRegistry, drafts: &[String]) -> usize {
    let mut checked = 0;
    for id in drafts {
        let path = registry
            .index()
            .paths
            .iter()
            .find(|path| &path.id == id)
            .unwrap_or_else(|| panic!("mapped draft {id:?} is absent from the release index"));
        assert_eq!(
            path.status,
            Some(ReleasePathStatus::Draft),
            "{id}: a qualified Path must leave refusal coverage and gain its own real cell"
        );
        assert!(
            path.unavailable.is_none(),
            "{id}: expected a playable draft"
        );

        for field in ["quests", "order_override"] {
            let mut settings = QueueSettings {
                include_draft_guides: true,
                ..QueueSettings::default()
            };
            if field == "quests" {
                settings.quests.push(id.clone());
            } else {
                settings.order_override.push(id.clone());
            }
            let error = Queue::from_registry(registry.clone(), settings)
                .expect_err("the production entry point must refuse a draft");
            assert_eq!(error.kind, QueueConfigErrorKind::UnqualifiedPath, "{id}");
            assert_eq!(error.field, field, "{id}");
            assert_eq!(error.message.as_ref(), UNQUALIFIED_PATH_REASON, "{id}");
            println!(
                "release-refusal id={id} field={field} kind={:?}: {error}",
                error.kind
            );
        }
        checked += 1;
    }
    checked
}

/// A row the release index marks unplayable has no status and no guide file.
/// An explicit pick of it is refused with the index's own reason, and the
/// default release queue never picks it, even when drafts are requested.
fn refuse_unavailable(registry: &PathRegistry, listed: &[String]) -> usize {
    let index = registry.index();
    for id in listed {
        let position = index
            .paths
            .iter()
            .position(|path| &path.id == id)
            .unwrap_or_else(|| panic!("unavailable {id:?} is absent from the release index"));
        let path = &index.paths[position];
        let reason = path
            .unavailable
            .as_deref()
            .unwrap_or_else(|| panic!("{id}: named unavailable but the index row is playable"));
        assert_eq!(path.status, None, "{id}: an unavailable row has no status");

        let mut settings = QueueSettings {
            include_draft_guides: true,
            ..QueueSettings::default()
        };
        settings.quests.push(id.clone());
        let error = Queue::from_registry(registry.clone(), settings)
            .expect_err("an explicit pick of an unavailable row must be refused");
        assert_eq!(error.kind, QueueConfigErrorKind::InvalidSetting, "{id}");
        assert_eq!(error.field, "quests", "{id}");
        let expected = format!("{}: {reason}", path.name.as_deref().unwrap_or(id));
        assert_eq!(error.message.as_ref(), expected.as_str(), "{id}");
        println!("release-unavailable id={id} kind={:?}: {error}", error.kind);

        let queue = Queue::from_registry(
            registry.clone(),
            QueueSettings {
                include_draft_guides: true,
                ..QueueSettings::default()
            },
        )
        .expect("the default release queue must build");
        assert!(
            queue
                .rows()
                .iter()
                .filter(|row| usize::from(row.index) == position)
                .all(|row| !row.picked),
            "{id}: the default queue must not pick an unavailable row"
        );
    }
    listed.len()
}
