use std::sync::Arc;

use script::{slot::RunState, surface::SurfaceStart, SlotScript};
use script_js::{native_fixture, prepare_raw_origin_for_test, surface, LoadShape};

#[test]
#[ignore = "isolated cold construction-count probe; run explicitly"]
fn u9_normal_slot_start_does_not_construct_risk_tables() {
    assert_eq!(script::combat::risk::risk_table_build_count_for_test(), 0);

    let mut slot = SlotScript::new();
    let prepared =
        prepare_raw_origin_for_test("bot.js", &native_fixture("export function tick() {}"), &[])
            .unwrap();
    assert_eq!(prepared.checked.shape(), LoadShape::NativeTick);
    slot.bind_surface_factory(Arc::new(surface::JsSurfaceFactory));
    slot.start_surface(SurfaceStart {
        card: Arc::new(surface::JsStartCard {
            card_name: "risk startup test".into(),
            source: Arc::<str>::from(prepared.js),
            checked: prepared.checked,
            siblings: Arc::default(),
        }),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
        wilderness_rules: script::surface::no_wilderness(),
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
