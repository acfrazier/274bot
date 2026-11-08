//! External consumer of the production script library, without test-hooks.
use script::quester::queue::{
    Queue, QueueConfigErrorKind, QueueSettings, ReleasePathStatus, UNQUALIFIED_PATH_REASON,
};
use script::quester::registry::PathRegistry;

fn main() {
    let registry = PathRegistry::Bundled;
    let priest = Queue::from_registry(
        registry.clone(),
        QueueSettings {
            quests: vec!["priest".into()],
            ..QueueSettings::default()
        },
    )
    .expect("the qualified Priest Path remains selectable");
    let candidate = priest.next_candidate().expect("qualified Priest candidate");
    assert_eq!(priest.id(candidate), Some("priest"));

    let mut checked = 0;
    for id in std::env::args().skip(1) {
        let path = registry
            .index()
            .paths
            .iter()
            .find(|path| path.id == id)
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
    assert!(checked > 0, "the refusal cell must name at least one draft");
    println!("release-refusal: {checked} mapped drafts refused; qualified Priest admitted");
}
