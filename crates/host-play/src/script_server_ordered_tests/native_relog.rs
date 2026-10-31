use super::*;
use std::sync::Arc;

#[cfg(feature = "test-support")]
#[test]
fn ordered_real_native_dispatch_uses_packet_clock_after_session_reset() {
    let mut rig = Rig::new();
    let _hook_reset = TickHookReset;
    let hook = crate::tick_engine_test_support::TickEngineTestHook::new("so3-wire");
    hook.bind_client(&mut rig.client, "so3-wire", Arc::clone(&rig.input));

    const BEFORE_RELOG_TICK: u64 = 50_000;
    const BEFORE_RELOG_PACKET_TICK: u32 = 50_000;
    const RESET_NATIVE_TICK: u64 = 50_001;
    const CONTENT_NATIVE_TICK: u64 = 50_002;
    const ADVANCED_NATIVE_TICK: u64 = 50_003;
    const RESET_PACKET_TICK: u32 = 1;
    const CONTENT_PACKET_TICK: u32 = 2;
    const ADVANCED_PACKET_TICK: u32 = 3;

    rig.client.gens.player_info = u64::from(BEFORE_RELOG_PACKET_TICK);
    rig.pump_clocks(BEFORE_RELOG_TICK, u64::from(BEFORE_RELOG_PACKET_TICK));
    let before_session = rig
        .input
        .with_server_ordered_input(|state| state.snapshot().session_generation());
    let before_tick = rig
        .input
        .with_server_ordered_input(|state| state.snapshot().observed_tick());
    assert_eq!(before_tick, Some(BEFORE_RELOG_PACKET_TICK));

    rig.client.gens.session += 1;
    rig.client.gens.player_info = 0;
    rig.snapshot.reset_session(rig.client.gens);
    rig.client.gens.player_info = 1;
    rig.client.gens.player += 1;
    rig.snapshot.rebuild(&rig.client);
    hook.bind_client(&mut rig.client, "so3-wire", Arc::clone(&rig.input));
    rig.pump_clocks(RESET_NATIVE_TICK, u64::from(RESET_PACKET_TICK));
    let relog_input = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_ne!(relog_input.session_generation(), before_session);
    assert_eq!(relog_input.session_generation(), rig.client.gens.session);
    assert_eq!(relog_input.observed_tick(), Some(RESET_PACKET_TICK));

    hook.arm_measurement(1511, &[]);
    rig.start([
        Emission::Content(close_request()),
        Emission::Advanced {
            operations: vec![api::prot::Send::close_modal()],
            debug: None,
        },
    ]);

    rig.pump_clocks(CONTENT_NATIVE_TICK, u64::from(CONTENT_PACKET_TICK));
    let content_trace = hook.trace();
    assert_eq!(
        content_trace.dispatches.len(),
        1,
        "real content dispatch trace: {content_trace:?}"
    );
    let content_receipt = content_trace.dispatches[0].receipt;
    assert_eq!(
        content_receipt.terminal, None,
        "real content dispatch after relog was refused: {content_receipt:?}"
    );
    let after_content = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(
        after_content.session_generation(),
        relog_input.session_generation()
    );
    assert_eq!(after_content.observed_tick(), Some(CONTENT_PACKET_TICK));
    assert_eq!(after_content.packet_count(), 1);

    rig.pump_clocks(ADVANCED_NATIVE_TICK, u64::from(ADVANCED_PACKET_TICK));
    let after_advanced = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(after_advanced.observed_tick(), Some(ADVANCED_PACKET_TICK));
    assert_eq!(after_advanced.packet_count(), 2);
    rig.pump_clocks(
        ADVANCED_NATIVE_TICK + 1,
        u64::from(ADVANCED_PACKET_TICK + 1),
    );

    let trace = hook.trace();
    assert_eq!(
        trace.dispatches.len(),
        2,
        "real host dispatch trace: {trace:?}"
    );
    for (dispatch, native_tick, packet_tick) in [
        (
            &trace.dispatches[0],
            CONTENT_NATIVE_TICK,
            CONTENT_PACKET_TICK,
        ),
        (
            &trace.dispatches[1],
            ADVANCED_NATIVE_TICK,
            ADVANCED_PACKET_TICK,
        ),
    ] {
        let receipt = dispatch.receipt;
        assert_eq!(
            receipt.terminal, None,
            "real dispatch was refused: native tick={native_tick}, packet tick={packet_tick}, receipt={receipt:?}"
        );
        assert!(receipt.dispatched_prefix > 0, "{receipt:?}");
        assert_eq!(receipt.dispatched_prefix, receipt.row_count, "{receipt:?}");
        assert_eq!(receipt.dispatch_tick, u64::from(packet_tick));
        assert_eq!(dispatch.game_tick, native_tick);
    }
    assert!(trace.dispatches[0]
        .request_debug
        .as_deref()
        .is_some_and(|request| request.contains("CloseModal")));
    assert_eq!(trace.dispatches[1].request_debug, None);
    assert_eq!(trace.wire.len(), 2, "real encrypted packets: {trace:?}");
    assert_eq!(rig.decoded().len(), 2);

    let results = rig.results.lock().unwrap();
    assert!(
        results.errors.is_empty(),
        "script errors: {:?}",
        results.errors
    );
    assert_eq!(
        results.receipts,
        trace
            .dispatches
            .iter()
            .map(|dispatch| dispatch.receipt)
            .collect::<Vec<_>>()
    );
}
