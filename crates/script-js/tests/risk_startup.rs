use std::sync::Arc;

use script::{slot::RunState, surface::SurfaceStart, SlotScript};
use script_js::{resolve_api_family, surface, LoadShape};

#[test]
#[ignore = "isolated cold construction-count probe; run explicitly"]
fn u9_normal_slot_start_does_not_construct_risk_tables() {
    assert_eq!(script::combat::risk::risk_table_build_count_for_test(), 0);

    let mut slot = SlotScript::new();
    let source: Arc<str> = "export function tick() {}".into();
    let family = resolve_api_family(&source)
        .ok()
        .map(|(_, family)| family);
    slot.bind_surface_factory(Arc::new(surface::JsSurfaceFactory::default()));
    slot.start_surface(SurfaceStart {
        card: Arc::new(surface::JsStartCard {
            source,
            shape: LoadShape::NativeTick,
            siblings: Arc::default(),
            family,
        }),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while slot.state() != RunState::Running && std::time::Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(slot.state(), RunState::Running);
    assert_eq!(
        script::combat::risk::risk_table_build_count_for_test(),
        0,
        "ordinary slot startup must not initialize risk"
    );
    slot.stop();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while slot.state() != RunState::Idle && std::time::Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(slot.state(), RunState::Idle);
}
