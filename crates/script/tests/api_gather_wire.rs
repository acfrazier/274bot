#![cfg(feature = "load")]

use api::selected::{RunKey, Truth};
use api::snapshot::WorldTile;
use script::api_gather::{GatherCounts, GatherEnd, GatherFailure, GatherPage, GatherPhase};
use script::isolate_fb::{
    decode_interact_batch, encode_interact_batch, encode_snapshot_delta_with_native,
    NativeFactsInput, ReachViewInput, Snapshot, SnapshotInput,
};
use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
use script::shim::InteractReq;
use script::{observed, CompiledId};
use std::sync::Arc;

fn empty_input(tick: u64) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 0,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

fn native_facts<'a>(
    page: Option<&'a GatherPage>,
    outcome: Option<&'a GatherEnd>,
) -> NativeFactsInput<'a> {
    NativeFactsInput {
        api_gather: page,
        api_gather_outcome: outcome,
        ..NativeFactsInput::default()
    }
}

fn script_status() -> Arc<ScriptStatus> {
    Arc::new(ScriptStatus {
        run: RunKey {
            slot: 1,
            run: 2,
            session: 3,
        },
        card: CompiledId("Gatherer"),
        phase: NativePhase::Working,
        active_settings: 1,
        pending_settings: None,
        fields: vec![
            StatusField {
                key: "skill",
                label: "Skill",
                value: StatusValue::Text(Arc::from("Mining")),
            },
            StatusField {
                key: "yielded",
                label: "Yielded",
                value: StatusValue::Integer(9),
            },
            StatusField {
                key: "tile",
                label: "Tile",
                value: StatusValue::Tile(WorldTile {
                    x: 3200,
                    z: 3210,
                    level: 1,
                }),
            },
            StatusField {
                key: "visible",
                label: "Visible",
                value: StatusValue::Truth(Truth::Unknown),
            },
        ]
        .into(),
        failure: None,
    })
}

fn blocked_end(token: u64) -> GatherEnd {
    GatherEnd::Blocked {
        token,
        failure: GatherFailure {
            code: Arc::from("route-blocked"),
            message: Arc::from("path is blocked"),
            retryable: true,
        },
        counts: GatherCounts {
            yielded: 11,
            dropped: 12,
            deposited: 13,
            trips: 14,
            xp: -15,
        },
    }
}

#[test]
fn gather_run_settings_use_typed_rows_and_preserve_empty_lists() {
    let mut settings = serde_json::Map::new();
    settings.insert("text".into(), serde_json::json!("Mining"));
    settings.insert("integer".into(), serde_json::json!(12));
    settings.insert("boolean".into(), serde_json::json!(true));
    settings.insert("falseBoolean".into(), serde_json::json!(false));
    settings.insert("emptyList".into(), serde_json::json!([]));
    settings.insert("list".into(), serde_json::json!(["copper", "tin"]));
    settings.insert(
        "tile".into(),
        serde_json::json!({"x": 3200, "z": 3210, "level": 1}),
    );
    let request = InteractReq::GatherRun {
        request_id: 42,
        settings: Arc::new(settings.clone()),
    };
    let bytes = encode_interact_batch(std::slice::from_ref(&request));
    let decoded = decode_interact_batch(&bytes).expect("typed gather request decodes");
    assert_eq!(decoded, vec![request]);
    let InteractReq::GatherRun {
        settings: decoded, ..
    } = &decoded[0]
    else {
        panic!("gather-run row retained its operation");
    };
    assert_eq!(decoded.as_ref(), &settings);
    assert_eq!(decoded.get("emptyList"), Some(&serde_json::json!([])));
}

#[test]
fn host_control_deserialization_fails_closed() {
    for forged in [
        serde_json::json!({"op": "gather-run", "request_id": 42, "settings": {}}),
        serde_json::json!({"op": "gather-stop", "request_id": 42}),
        serde_json::json!({"op": "progress-read", "request_id": 42, "name": "cook"}),
    ] {
        assert!(serde_json::from_value::<InteractReq>(forged).is_err());
    }
}

#[test]
fn every_gather_terminal_kind_round_trips_with_counts_and_reason() {
    let counts = GatherCounts {
        yielded: 2,
        dropped: 3,
        deposited: 4,
        trips: 5,
        xp: -6,
    };
    let cases = [
        (
            GatherEnd::Stopped { token: 1, counts },
            1,
            None,
            None,
            false,
            counts,
        ),
        (
            GatherEnd::Blocked {
                token: 2,
                failure: GatherFailure {
                    code: Arc::from("blocked"),
                    message: Arc::from("path closed"),
                    retryable: true,
                },
                counts,
            },
            2,
            Some("blocked"),
            Some("path closed"),
            true,
            counts,
        ),
        (
            GatherEnd::Refused {
                token: 3,
                reason: Arc::from("invalid-settings"),
            },
            3,
            None,
            Some("invalid-settings"),
            false,
            GatherCounts::default(),
        ),
        (
            GatherEnd::Failed {
                token: 4,
                reason: Arc::from("preparation failed"),
                counts,
            },
            4,
            None,
            Some("preparation failed"),
            false,
            counts,
        ),
    ];

    for (end, expected_end, code, message, retryable, expected_counts) in cases {
        let (bytes, _) = encode_snapshot_delta_with_native(
            None,
            &empty_input(1),
            native_facts(None, Some(&end)),
            false,
        );
        let snapshot = Snapshot::from_bytes(&bytes).unwrap();
        let outcome = snapshot.api_gather_outcome().unwrap();
        assert_eq!(outcome.request_id(), end.token());
        assert_eq!(outcome.end(), expected_end);
        assert_eq!(outcome.code(), code);
        assert_eq!(outcome.message(), message);
        assert_eq!(outcome.retryable(), retryable);
        assert_eq!(
            (
                outcome.yielded(),
                outcome.dropped(),
                outcome.deposited(),
                outcome.trips(),
                outcome.xp(),
            ),
            (
                expected_counts.yielded,
                expected_counts.dropped,
                expected_counts.deposited,
                expected_counts.trips,
                expected_counts.xp,
            )
        );
    }
}
#[test]
fn gather_snapshot_deltas_use_arc_identity_clear_live_page_and_retain_terminal() {
    observed::on_reset();
    let (empty_keyframe, _) = encode_snapshot_delta_with_native(
        None,
        &empty_input(0),
        NativeFactsInput::default(),
        false,
    );
    let empty_keyframe = Snapshot::from_bytes(&empty_keyframe).unwrap();
    assert!(empty_keyframe.has_api_gather());
    assert_eq!(empty_keyframe.api_gather().unwrap().request_id(), 0);
    assert!(!empty_keyframe.has_api_gather_outcome());
    let status = script_status();
    let page = GatherPage {
        token: 7,
        phase: GatherPhase::Preparing,
        status: None,
    };

    let terminal = blocked_end(7);
    let (both_pages, _) = encode_snapshot_delta_with_native(
        None,
        &empty_input(0),
        native_facts(Some(&page), Some(&terminal)),
        false,
    );
    let both_pages = Snapshot::from_bytes(&both_pages).unwrap();
    assert!(both_pages.has_api_gather());
    assert!(both_pages.has_api_gather_outcome());
    let (keyframe, mut fingerprint) = encode_snapshot_delta_with_native(
        None,
        &empty_input(1),
        native_facts(Some(&page), None),
        false,
    );
    let decoded = Snapshot::from_bytes(&keyframe).expect("gather keyframe decodes");
    assert!(decoded.has_api_gather());
    let posted = decoded.api_gather().expect("keyframe carries gather page");
    assert_eq!(
        (posted.request_id(), posted.phase(), posted.has_status()),
        (7, 1, false)
    );
    observed::apply(&decoded);

    let (unchanged, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(2),
        native_facts(Some(&page), None),
        false,
    );
    assert!(!Snapshot::from_bytes(&unchanged).unwrap().has_api_gather());
    fingerprint = next;
    observed::apply(&Snapshot::from_bytes(&unchanged).unwrap());
    assert_eq!(
        observed::with(|scene| scene.since_login().api_gather().unwrap().request_id),
        7
    );

    let running = GatherPage {
        token: 7,
        phase: GatherPhase::Running,
        status: Some(Arc::clone(&status)),
    };
    let (running_delta, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(3),
        native_facts(Some(&running), None),
        false,
    );
    let view = Snapshot::from_bytes(&running_delta).unwrap();
    assert!(view.has_api_gather());
    assert_eq!(view.api_gather().unwrap().phase(), 2);
    assert!(view.api_gather().unwrap().has_status());
    observed::apply(&view);
    let observed_page = observed::with(|scene| scene.since_login().api_gather().unwrap().clone());
    assert_eq!(observed_page.request_id, 7);
    fingerprint = next;

    let same_arc = GatherPage {
        token: 7,
        phase: GatherPhase::Running,
        status: Some(Arc::clone(&status)),
    };
    let (same_status, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(4),
        native_facts(Some(&same_arc), None),
        false,
    );
    assert!(!Snapshot::from_bytes(&same_status).unwrap().has_api_gather());
    fingerprint = next;

    let equal_but_new_arc = GatherPage {
        token: 7,
        phase: GatherPhase::Running,
        status: Some(script_status()),
    };
    let (new_status, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(5),
        native_facts(Some(&equal_but_new_arc), None),
        false,
    );
    assert!(Snapshot::from_bytes(&new_status).unwrap().has_api_gather());
    fingerprint = next;

    let (clear, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(6),
        NativeFactsInput::default(),
        false,
    );
    let clear = Snapshot::from_bytes(&clear).unwrap();
    assert!(clear.has_api_gather());
    let clear_page = clear.api_gather().unwrap();
    assert_eq!(clear_page.request_id(), 0);
    assert!(!clear_page.has_status());
    observed::apply(&clear);
    assert_eq!(
        observed::with(|scene| scene.since_login().api_gather().unwrap().request_id),
        0
    );
    fingerprint = next;

    let (outcome_delta, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(7),
        native_facts(None, Some(&terminal)),
        false,
    );
    let outcome_view = Snapshot::from_bytes(&outcome_delta).unwrap();
    assert!(outcome_view.has_api_gather_outcome());
    let outcome = outcome_view.api_gather_outcome().unwrap();
    assert_eq!((outcome.request_id(), outcome.end()), (7, 2));
    assert_eq!(
        (
            outcome.yielded(),
            outcome.dropped(),
            outcome.deposited(),
            outcome.trips(),
            outcome.xp()
        ),
        (11, 12, 13, 14, -15)
    );
    assert_eq!(outcome.code(), Some("route-blocked"));
    assert_eq!(outcome.message(), Some("path is blocked"));
    assert!(outcome.retryable());
    observed::apply(&outcome_view);
    fingerprint = next;

    let (retained, next) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(8),
        NativeFactsInput::default(),
        false,
    );
    let retained = Snapshot::from_bytes(&retained).unwrap();
    assert!(!retained.has_api_gather_outcome());
    observed::apply(&retained);
    assert_eq!(
        observed::with(|scene| scene
            .since_login()
            .api_gather_outcome()
            .unwrap()
            .end
            .clone()),
        terminal
    );

    let next_terminal = GatherEnd::Stopped {
        token: 8,
        counts: GatherCounts::default(),
    };
    let (replacement, _) = encode_snapshot_delta_with_native(
        Some(&next),
        &empty_input(9),
        native_facts(None, Some(&next_terminal)),
        false,
    );
    assert!(Snapshot::from_bytes(&replacement)
        .unwrap()
        .has_api_gather_outcome());
}

#[test]
fn canonical_sample_status_wire_delta_stays_inside_two_kib() {
    struct Capture(Option<ScriptStatus>);
    impl script::native::NativeOutput for Capture {
        fn status(&mut self, status: ScriptStatus) {
            self.0 = Some(status);
        }
        fn paint(&mut self, _: Arc<script::shim::ScriptPaint>) {
            unreachable!("status publication cannot paint");
        }
        fn log(&mut self, _: api::hostlog::Level, _: &str) {
            unreachable!("status publication cannot log");
        }
        fn settings_applied(&mut self, _: u64) {
            unreachable!("status publication cannot apply settings");
        }
    }
    let mut capture = Capture(None);
    script::gatherer::status::publish(
        &mut capture,
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        1,
        None,
        NativePhase::Working,
        None,
        &script::gatherer::status::StatusData {
            skill: "Woodcutting",
            method: Arc::from("woodcutting.normal.op1"),
            phase: "gathering",
            area: Arc::from("Start (Lumbridge)"),
            target: Arc::from("Tree"),
            tool: Arc::from("Bronze axe"),
            bait: -1,
            food: 0,
            coins: 0,
            yielded: 29,
            dropped: 28,
            deposited: 0,
            trips: 0,
            xp: 725,
            xp_per_hour: Some(4500),
            bank: Arc::from("—"),
            last_progress: 12,
            deaths: 0,
            absent: 0,
            zone_gated: 0,
            excluded_targets: Arc::from(""),
            last_event: Arc::from("dropped"),
        },
    );
    let page = GatherPage {
        token: 31,
        phase: GatherPhase::Running,
        status: Some(Arc::new(capture.0.expect("canonical status publication"))),
    };
    let input = empty_input(1);
    let (bare, _) =
        encode_snapshot_delta_with_native(None, &input, native_facts(None, None), false);
    let (populated, fingerprint) =
        encode_snapshot_delta_with_native(None, &input, native_facts(Some(&page), None), false);
    let (unchanged, _) = encode_snapshot_delta_with_native(
        Some(&fingerprint),
        &empty_input(2),
        native_facts(Some(&page), None),
        false,
    );
    let delta = populated
        .len()
        .checked_sub(bare.len())
        .expect("populated status byte delta");
    println!("API gather canonical sample wire: without={}B populated={}B status_delta={delta}B unchanged={}B (gather page omitted)", bare.len(), populated.len(), unchanged.len());
    assert!(
        delta <= 2048,
        "canonical sample status exceeds the 2-KiB wire target"
    );
    assert!(!Snapshot::from_bytes(&unchanged).unwrap().has_api_gather());
}
