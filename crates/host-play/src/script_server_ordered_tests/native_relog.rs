use super::*;
use std::sync::Arc;

#[cfg(feature = "test-support")]
#[test]
fn ordered_real_native_dispatch_uses_packet_clock_after_session_reset() {
    let Some(mut rig) = Rig::with_game_data() else {
        return;
    };
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
    const RAW_GENERATION_BEFORE_RELOG: u64 = 700_000;

    rig.client.gens.player_info = RAW_GENERATION_BEFORE_RELOG;
    rig.pump_three_clocks(
        BEFORE_RELOG_TICK,
        BEFORE_RELOG_PACKET_TICK,
        RAW_GENERATION_BEFORE_RELOG,
    );
    let before_session = rig
        .input
        .with_server_ordered_input(|state| state.snapshot().session_generation());
    let before_tick = rig
        .input
        .with_server_ordered_input(|state| state.snapshot().observed_tick());
    assert_eq!(before_tick, Some(RAW_GENERATION_BEFORE_RELOG as u32));

    rig.client.gens.session += 1;
    rig.client.gens.player_info = RAW_GENERATION_BEFORE_RELOG;
    rig.snapshot.reset_session(rig.client.gens);
    rig.client.gens.player_info += 1;
    rig.client.gens.player += 1;
    rig.snapshot.rebuild(&rig.client);
    hook.bind_client(&mut rig.client, "so3-wire", Arc::clone(&rig.input));
    rig.pump_three_clocks(
        RESET_NATIVE_TICK,
        RESET_PACKET_TICK,
        RAW_GENERATION_BEFORE_RELOG + u64::from(RESET_PACKET_TICK),
    );
    let relog_input = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_ne!(relog_input.session_generation(), before_session);
    assert_eq!(relog_input.session_generation(), rig.client.gens.session);
    assert_eq!(
        relog_input.observed_tick(),
        Some(RAW_GENERATION_BEFORE_RELOG as u32 + RESET_PACKET_TICK)
    );

    hook.arm_measurement(1511, &[]);
    rig.start([
        Emission::Content(close_request()),
        Emission::Advanced {
            operations: vec![api::prot::Send::close_modal()],
            debug: None,
        },
    ]);

    rig.pump_three_clocks(
        CONTENT_NATIVE_TICK,
        CONTENT_PACKET_TICK,
        RAW_GENERATION_BEFORE_RELOG + u64::from(CONTENT_PACKET_TICK),
    );
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
    assert_eq!(
        after_content.observed_tick(),
        Some(RAW_GENERATION_BEFORE_RELOG as u32 + CONTENT_PACKET_TICK)
    );
    assert_eq!(after_content.packet_count(), 1);

    rig.pump_three_clocks(
        ADVANCED_NATIVE_TICK,
        ADVANCED_PACKET_TICK,
        RAW_GENERATION_BEFORE_RELOG + u64::from(ADVANCED_PACKET_TICK),
    );
    let after_advanced = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(
        after_advanced.observed_tick(),
        Some(RAW_GENERATION_BEFORE_RELOG as u32 + ADVANCED_PACKET_TICK)
    );
    assert_eq!(after_advanced.packet_count(), 2);
    rig.pump_three_clocks(
        ADVANCED_NATIVE_TICK + 1,
        ADVANCED_PACKET_TICK + 1,
        RAW_GENERATION_BEFORE_RELOG + u64::from(ADVANCED_PACKET_TICK + 1),
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
        assert_eq!(
            receipt.dispatch_tick,
            RAW_GENERATION_BEFORE_RELOG + u64::from(packet_tick)
        );
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

#[test]
fn ordered_packet_clock_older_raw_generation_is_stale_before_and_after_relog() {
    for after_relog in [false, true] {
        let mut rig = Rig::new();
        if after_relog {
            rig.client.gens.session += 1;
            rig.snapshot.reset_session(rig.client.gens);
        }
        rig.snapshot.seed_tick(if after_relog { 2 } else { 15 });
        rig.client.gens.player_info = 41;
        rig.snapshot.seed_player_info_generation(41);
        rig.input
            .observe_input_tick(rig.client.revision(), 40, rig.client.gens.session);
        let observed = rig
            .input
            .with_server_ordered_input(|state| state.snapshot());
        // The old input agrees with the old stamp: only the snapshot-generation
        // guard can reject this dispatch before or after the session boundary.
        let mut authority = || Ok(());
        let receipt = ServerOrderedInputs::dispatch_advanced(
            &rig.snapshot,
            &mut rig.client,
            &[Send::close_modal()],
            ServerOrderedDispatchContext {
                request_id: 1,
                dispatch_tick: 40,
                input: observed,
                debug: None,
                authority: &mut authority,
            },
        );
        assert_eq!(
            receipt.terminal,
            Some(ServerOrderedError::StaleOwner),
            "after_relog={after_relog}: {receipt:?}"
        );
        assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
        assert_no_cipher_movement(&rig);
    }
}

#[test]
fn ordered_packet_clock_old_session_is_refused_at_the_same_relative_tick() {
    let Some(mut rig) = Rig::with_game_data() else {
        return;
    };
    rig.client.gens.player_info = 99;
    rig.snapshot.seed_tick(7);
    rig.snapshot.seed_player_info_generation(99);
    rig.input
        .observe_input_tick(rig.client.revision(), 99, rig.client.gens.session);
    rig.start([Emission::Content(close_request())]);
    let slot = script_slot_or_insert(&rig.scripts, "so3-wire");
    let action = {
        let mut slot = slot.lock().unwrap();
        slot.on_game_tick(&mut script::ScriptCtx {
            driver: &mut rig.client,
            tick: 50_000,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(&rig.snapshot),
            obj_names: None,
            compiled: script::CompiledTick::default(),
        });
        slot.take_native_action().expect("pending ordered request")
    };
    let authority = action.authority();
    assert!(
        authority.live(),
        "the pre-relog request must still be pending"
    );
    let script::native::HostEffect::ServerOrdered(envelope) = action.effect else {
        panic!("expected the real pending ordered envelope")
    };
    {
        let mut slot = slot.lock().unwrap();
        slot.reconnect_session_work();
        slot.on_is_up(true);
    }
    rig.client.gens.session += 1;
    rig.snapshot.reset_session(rig.client.gens);
    rig.client.gens.player_info = 100;
    rig.snapshot.rebuild(&rig.client);
    rig.snapshot.seed_tick(7);
    rig.snapshot.seed_player_info_generation(100);
    rig.input
        .observe_input_tick(rig.client.revision(), 100, rig.client.gens.session);
    assert_eq!(rig.snapshot.tick(), 7);
    assert!(
        !authority.live(),
        "the real session boundary revokes the owner"
    );
    let receipt = super::super::dispatch_native_server_ordered(
        &mut rig.client,
        super::super::OrderedDispatchContext {
            snapshot: &rig.snapshot,
            selected: Some(rig.selected.as_ref()),
            input: Some(&rig.input),
            builtin_gatherer_knife: false,
        },
        &authority,
        &envelope,
    );
    assert_eq!(
        receipt.terminal,
        Some(ServerOrderedError::Cancelled),
        "equal relative ticks must not revive an earlier session: {receipt:?}"
    );
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_no_cipher_movement(&rig);
}

#[test]
fn ordered_packet_clock_unrepresentable_raw_generation_is_stale() {
    let mut rig = Rig::new();
    let generation = u64::from(u32::MAX) + 1;
    rig.client.gens.player_info = generation;
    rig.snapshot.seed_tick(1);
    rig.snapshot.seed_player_info_generation(generation);
    rig.input
        .observe_input_tick(rig.client.revision(), generation, rig.client.gens.session);
    let observed = rig
        .input
        .with_server_ordered_input(|state| state.snapshot());
    assert_eq!(observed.observed_tick(), None);
    let mut authority_calls = 0;
    let mut authority = || {
        authority_calls += 1;
        Ok(())
    };
    let receipt = ServerOrderedInputs::dispatch_advanced(
        &rig.snapshot,
        &mut rig.client,
        &[Send::close_modal()],
        ServerOrderedDispatchContext {
            request_id: 1,
            dispatch_tick: generation,
            input: observed,
            debug: None,
            authority: &mut authority,
        },
    );
    assert_eq!(receipt.terminal, Some(ServerOrderedError::StaleOwner));
    assert_eq!((receipt.dispatched_prefix, receipt.wire_events), (0, 0));
    assert_eq!(authority_calls, 0);
    assert_no_cipher_movement(&rig);
}
