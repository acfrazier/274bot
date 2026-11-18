//! The slot owns terminal precedence even when a surface's onStop fails.
#![cfg(feature = "test-hooks")]

use script::{
    native::ScriptFailure,
    slot::ScriptTerminalState,
    surface::{
        test_support::{StubSurfaceCard, StubSurfaceFactory},
        ScriptStopReceipt, SurfaceFactory, SurfaceReady, SurfaceStart,
    },
    SlotScript,
};
use std::sync::Arc;

fn started_slot() -> (SlotScript, Arc<StubSurfaceFactory>) {
    let mut slot = SlotScript::new();
    let factory = Arc::new(StubSurfaceFactory::new());
    let bound: Arc<dyn SurfaceFactory> = factory.clone();
    slot.bind_surface_factory(bound);
    let start = SurfaceStart {
        card: Arc::new(StubSurfaceCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::new(api::named_banks::NamedBankFacts::default()),
        wilderness_rules: script::surface::no_wilderness(),
        loadouts: Arc::from([]),
    };
    slot.start_surface(start).unwrap();
    factory
        .last_run()
        .unwrap()
        .set_readiness(SurfaceReady::Ready);
    slot.poll_start();
    (slot, factory)
}

fn hook_failure() -> ScriptFailure {
    ScriptFailure {
        code: "luau-watchdog".into(),
        message: "onStop passed 50 ms".into(),
    }
}

#[test]
fn operator_stop_hook_failure_logs_once_without_a_receipt() {
    let (mut slot, factory) = started_slot();
    factory
        .last_run()
        .unwrap()
        .set_retirement_hook_failure(hook_failure());
    slot.stop();
    assert_eq!(
        slot.take_pending_logs(),
        vec!["onStop failed (luau-watchdog): onStop passed 50 ms"]
    );
    assert_eq!(
        slot.last_error(),
        Some("onStop failed (luau-watchdog): onStop passed 50 ms")
    );
    assert_eq!(slot.lifecycle_receipt(), None);
    assert!(slot.take_pending_logs().is_empty());
}

#[test]
fn script_stop_receipt_survives_a_hook_failure() {
    let (mut slot, factory) = started_slot();
    let run = factory.last_run().unwrap();
    run.set_terminal(
        true,
        Some(ScriptStopReceipt {
            tick: 77,
            reason: "done".into(),
        }),
    );
    run.set_retirement_hook_failure(hook_failure());
    slot.drain_logs();
    let receipt = slot.lifecycle_receipt().unwrap();
    assert_eq!(receipt.state, ScriptTerminalState::Stopped);
    assert_eq!(receipt.reason, "done");
    assert_eq!(receipt.tick, 77);
    assert_eq!(slot.last_error(), Some("script requested stop on tick 77: done\nonStop failed (luau-watchdog): onStop passed 50 ms"));
    assert_eq!(
        slot.take_pending_logs()
            .iter()
            .filter(|line| line.starts_with("onStop failed"))
            .count(),
        1
    );
}
