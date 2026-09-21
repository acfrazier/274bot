//! Load the checked-in Brimhaven v2 example through real NativeTick v2.
//! Proves the fixture begins/settles on published NativeSnapshot fields,
//! not a retyped facsimile (LIVE failed because `scene_state` is raw-only).

use script::isolate_fb::{
    InspectHopInput, NativeFactsInput, ReachViewInput, RouteInspectFactsInput,
    RouteInspectTerminalInput, SnapshotInput, TileInput,
};
use script::load::{live_example_path, ApiFamily, JsLibrary, LoadIsolate, LoadShape};
use script::shim::InteractReq;

const PIER: TileInput = TileInput {
    x: 2683,
    z: 3272,
    level: 0,
};
const BANK: TileInput = TileInput {
    x: 2655,
    z: 3283,
    level: 0,
};
const STOP_OK: &str = "route inspect brimhaven qualification complete";

fn scratch() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "274bot-route-inspect-brimhaven-v2-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn live_like_snapshot(tick: u64, here: TileInput) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: Some(here),
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
        hold: false,
        ours: false,
        npcs: &[],
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
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
        my_name: Some("Liveh97qur 0"),
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
        // Present on the raw ScriptSnapshot / host Core observation.
        // NATIVE_V2_MAIN SNAPSHOT_KEYS does not expose this on NativeSnapshot.
        scene_state: 2,
        weight: 0,
        combat_level: 3,
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
        widgets: &[],
    }
}

fn post_tick(iso: &LoadIsolate, tick: u64, here: TileInput, facts: NativeFactsInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
        &live_like_snapshot(tick, here),
        facts,
    ));
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
}

fn load_actual_example() -> LoadIsolate {
    let path = live_example_path("route_inspect_brimhaven_v2.ts")
        .expect("checked-in route_inspect_brimhaven_v2.ts");
    let dir = scratch();
    let mut library = JsLibrary::with_cache(dir.join("cards.json"), dir.join("cache"));
    let card = library.load(&path).expect("transpile/load actual example");
    assert_eq!(card.shape, LoadShape::NativeTick);
    assert_eq!(card.api_family, ApiFamily::V2);
    assert!(card.unloadable.is_none(), "{:?}", card.unloadable);
    LoadIsolate::spawn(card.js, card.shape, vec![]).expect("spawn actual example")
}

fn inspect_routes(iso: &LoadIsolate) -> Vec<InteractReq> {
    iso.drain_interacts()
        .into_iter()
        .filter(|req| matches!(req, InteractReq::InspectRoute { .. }))
        .collect()
}

#[test]
fn actual_example_begins_and_settles_on_published_native_snapshot() {
    let iso = load_actual_example();
    post_tick(&iso, 1, PIER, NativeFactsInput::default());

    let hidden = iso
        .probe("globalThis.__rs_api.snapshot.scene_state")
        .expect("probe native snapshot");
    assert!(
        hidden.is_null(),
        "scene_state is raw-only; native projection must not expose it: {hidden}"
    );
    assert_eq!(
        iso.probe("globalThis.__rs_api.snapshot.ingame").unwrap(),
        true
    );
    assert_eq!(
        iso.probe("globalThis.__rs_api.snapshot.here.x").unwrap(),
        2683
    );

    let begun = inspect_routes(&iso);
    assert_eq!(
        begun.len(),
        1,
        "LIVE-like ingame+here+raw scene_state=2 must begin inspect, not wait for filtered scene_state: {begun:?}"
    );
    let token = match &begun[0] {
        InteractReq::InspectRoute {
            from_x,
            from_z,
            from_level,
            x,
            z,
            level,
            request_id,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            ..
        } => {
            assert_eq!((*from_x, *from_z, *from_level), (2683, 3272, 0));
            assert_eq!((*x, *z, *level), (2698, 3206, 0));
            assert!(!*allow_teleports);
            assert!(*allow_wilderness);
            assert!(!*allow_bank_fetch);
            assert_ne!(*request_id, 0);
            *request_id
        }
        other => panic!("expected inspect-route, got {other:?}"),
    };
    assert!(
        !iso.stopped(),
        "ready must not self-stop: {:?}",
        iso.script_stop_receipt()
    );

    let hops = [InspectHopInput {
        kind: "boat",
        loc_id: 1,
        loc_name: "Captain Barnaby",
        action: "Pay-fare",
        option: 1,
        from_x: 2683,
        from_z: 3272,
        from_level: 0,
        to_x: 2698,
        to_z: 3206,
        to_level: 0,
        ticks: 8,
    }];
    let settled_facts = NativeFactsInput {
        route_inspect: RouteInspectFactsInput {
            latest: RouteInspectTerminalInput {
                seq: 2,
                generation: 1,
                request_id: token,
                ok: true,
                reason: Some(""),
                bank_planned: false,
                ticks: 8.0,
                hops: &hops,
            },
            accepted_id: token,
            ..Default::default()
        },
        ..Default::default()
    };
    post_tick(&iso, 2, PIER, settled_facts);
    assert!(
        !iso.stopped(),
        "settled token must not stop: {:?}",
        iso.script_stop_receipt()
    );

    post_tick(&iso, 3, PIER, settled_facts);
    let snap0 = inspect_routes(&iso);
    assert!(
        snap0.iter().any(|req| matches!(
            req,
            InteractReq::InspectRoute { request_id: 0, .. }
        )),
        "after isolate settle, fixture must send snapshot-only id0: {snap0:?}"
    );
    assert_eq!(
        iso.probe("globalThis.__rs_api.inspectSettled(0)").unwrap(),
        false
    );
    iso.join();
}

fn barnaby_hops() -> [InspectHopInput<'static>; 1] {
    [InspectHopInput {
        kind: "boat",
        loc_id: 1,
        loc_name: "Captain Barnaby",
        action: "Pay-fare",
        option: 1,
        from_x: 2683,
        from_z: 3272,
        from_level: 0,
        to_x: 2698,
        to_z: 3206,
        to_level: 0,
        ticks: 8,
    }]
}

fn inspect_facts<'a>(
    seq: u64,
    request_id: u64,
    hops: &'a [InspectHopInput<'a>],
) -> NativeFactsInput<'a> {
    NativeFactsInput {
        route_inspect: RouteInspectFactsInput {
            latest: RouteInspectTerminalInput {
                seq,
                generation: 1,
                request_id,
                ok: true,
                reason: Some(""),
                bank_planned: false,
                ticks: 8.0,
                hops,
            },
            accepted_id: request_id,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// LIVE r3: both inspect forms consumed, ordinary walk reached exact BANK,
/// `walk_outcome_seq` stayed 0 (success never publishes for request_id 0).
/// The fixture must clean-stop on real arrival, not wait for a seq bump.
#[test]
fn actual_example_stops_on_bank_arrival_without_walk_outcome_seq() {
    let iso = load_actual_example();
    let hops = barnaby_hops();
    post_tick(&iso, 1, PIER, NativeFactsInput::default());
    let begun = inspect_routes(&iso);
    let token = match begun.as_slice() {
        [InteractReq::InspectRoute { request_id, .. }] if *request_id != 0 => *request_id,
        other => panic!("expected token inspect-route, got {other:?}"),
    };

    post_tick(&iso, 2, PIER, inspect_facts(1, token, &hops));
    post_tick(&iso, 3, PIER, inspect_facts(1, token, &hops));
    assert!(
        inspect_routes(&iso)
            .iter()
            .any(|req| matches!(req, InteractReq::InspectRoute { request_id: 0, .. })),
        "must consume snapshot id0 after the token"
    );

    let id0 = inspect_facts(2, 0, &hops);
    post_tick(&iso, 4, PIER, id0);
    // snapshot0-wait consumes id0 then returns; walk-send is the next tick.
    post_tick(&iso, 5, PIER, id0);
    let walks: Vec<_> = iso
        .drain_interacts()
        .into_iter()
        .filter(|req| matches!(req, InteractReq::WalkNear { .. }))
        .collect();
    assert_eq!(
        walks.len(),
        1,
        "after both inspect forms, fixture must dispatch walk-near: {walks:?}"
    );
    match &walks[0] {
        InteractReq::WalkNear {
            x,
            z,
            level,
            radius,
            request_id,
            allow_teleports,
            allow_bank_fetch,
            ..
        } => {
            assert_eq!((*x, *z, *level), (BANK.x, BANK.z, BANK.level));
            assert_eq!(*radius, 4);
            assert_eq!(*request_id, 0);
            assert!(!*allow_teleports);
            assert!(!*allow_bank_fetch);
        }
        other => panic!("expected walk-near, got {other:?}"),
    }
    assert!(
        !iso.stopped(),
        "walk-send must not stop before arrival: {:?}",
        iso.script_stop_receipt()
    );
    assert_eq!(
        iso.probe("globalThis.__rs_api.snapshot.walk_outcome_seq")
            .unwrap(),
        0
    );

    // Still on the pier; seq unchanged — must keep waiting, not accept near-bank.
    post_tick(&iso, 6, PIER, id0);
    assert!(
        !iso.stopped(),
        "must not stop at the pier when walk_outcome_seq stays 0: {:?}",
        iso.script_stop_receipt()
    );

    // Exact LIVE terminal tile. Host does not bump walk_outcome_seq on success.
    post_tick(&iso, 7, BANK, id0);
    let receipt = iso.script_stop_receipt();
    assert!(
        iso.stopped(),
        "exact BANK arrival with unchanged walk_outcome_seq must clean-stop: {receipt:?}"
    );
    assert_eq!(
        receipt.as_ref().map(|r| r.reason.as_str()),
        Some(STOP_OK),
        "{receipt:?}"
    );
    iso.join();
}
