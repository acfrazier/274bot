//! External consumer of the production script library, without test-hooks.
//!
//! Arguments are `draft:<id>` (refused through the public queue entry point) and
//! `qualified:<id>` (admitted through the same entry point). The caller reads the
//! qualified ids from `index.json`; this program requires that the bundled
//! production index names exactly the same set.
use script::quester::queue::{
    Queue, QueueConfigErrorKind, QueueSettings, ReleasePathStatus, UNQUALIFIED_PATH_REASON,
};
use script::quester::registry::PathRegistry;
use std::collections::BTreeSet;

fn main() {
    let registry = PathRegistry::Bundled;
    let mut drafts = Vec::new();
    let mut qualified = Vec::new();
    for argument in std::env::args().skip(1) {
        match argument.split_once(':') {
            Some(("draft", id)) if !id.is_empty() => drafts.push(id.to_string()),
            Some(("qualified", id)) if !id.is_empty() => qualified.push(id.to_string()),
            _ => panic!("expected draft:<id> or qualified:<id>, got {argument:?}"),
        }
    }

    let admitted = admit_qualified(&registry, &qualified);
    let refused = refuse_drafts(&registry, &drafts);
    assert!(refused > 0, "the refusal cell must name at least one draft");
    println!(
        "release-refusal: {refused} mapped drafts refused; {admitted} qualified guides admitted"
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
