//! Frozen `Traversal.walkResilient` / `Traversal.walkTo` options: the
//! `maxBudget` / `maxExpansions` search bounds (`Traversal.ts:24, 106`,
//! `WalkExecutor.ts:230`) and walkTo's `WalkOptions`
//! (`WalkExecutor.ts:113–139, 165–177, 226–245`).

use script::isolate_fb::TileInput;
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

/// A frozen search bound (`maxBudget` on walkResilient, `maxExpansions` on
/// walkTo) is accepted and the walk still routes and settles: the host
/// router's own bound never searches less than a frozen caller asked.
fn walk_with_bound(call: &str) -> (String, Vec<InteractReq>, serde_json::Value) {
    let src = format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__err = '';
        globalThis.__ok = null;
        try {{
            globalThis.__ok = await {call};
        }} catch (e) {{
            globalThis.__err = String(e.message || e);
        }}
    }}
}}
"#
    );
    let iso = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    snap.here = Some(TileInput {
        x: 3222,
        z: 3222,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    let ops = iso.drain_interacts();
    // The route ends at the dest.
    snap.tick = 2;
    snap.here = Some(TileInput {
        x: 3222,
        z: 3240,
        level: 0,
    });
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    iso.probe("true").unwrap();
    let err = iso
        .probe("__err")
        .unwrap()
        .as_str()
        .unwrap_or_default()
        .to_string();
    let ok = iso.probe("__ok").unwrap();
    iso.join();
    (err, ops, ok)
}

#[test]
fn a_frozen_search_bound_is_accepted_and_the_walk_still_resolves() {
    // JiveKQ passes maxBudget 120_000 (route.ts:17); 0 and a bound past the
    // host's own are accepted the same way.
    for bound in [0u64, 120_000, 9_000_000] {
        for call in [
            format!(
                "Traversal.walkResilient({{ x: 3222, z: 3240, level: 0 }}, {{ radius: 0, maxBudget: {bound} }})"
            ),
            format!(
                "Traversal.walkTo({{ x: 3222, z: 3240, level: 0 }}, {{ radius: 0, maxExpansions: {bound} }})"
            ),
        ] {
            let (err, ops, ok) = walk_with_bound(&call);
            assert_eq!(err, "", "{call}");
            assert!(
                ops.iter().any(|op| matches!(
                    op,
                    InteractReq::Walk {
                        x: 3222,
                        z: 3240,
                        ..
                    }
                )),
                "{call}: the walk goes out: {ops:?}"
            );
            assert_eq!(ok, true, "{call}: the walk settles arrived");
        }
    }
}

/// One `Traversal.walkTo(3222,3240)` with `opts` (a JS object literal) from
/// (3222,3222), `ticks` game ticks with no walk outcome posted. Returns the
/// ops, the logs, the caught error text (`""` if none), the Sustain count
/// and the settled value (`null` while walking).
fn walk_to_with(
    opts: &str,
    ticks: u64,
) -> (
    Vec<InteractReq>,
    Vec<String>,
    String,
    i64,
    serde_json::Value,
) {
    let src = format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
import {{ Sustain }} from '../../api/sustain/Sustain.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__logs = [];
        globalThis.__err = '';
        globalThis.__sus = 0;
        globalThis.__ok = null;
        Sustain.set(() => {{ globalThis.__sus += 1; }});
        const opts = {opts};
        opts.log = (m) => globalThis.__logs.push(String(m));
        try {{
            globalThis.__ok = await Traversal.walkTo({{ x: 3222, z: 3240, level: 0 }}, opts);
        }} catch (e) {{
            globalThis.__err = String(e.message || e);
        }}
    }}
}}
"#
    );
    let iso = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    snap.here = Some(TileInput {
        x: 3222,
        z: 3222,
        level: 0,
    });
    let mut ops = Vec::new();
    for tick in 1..=ticks {
        snap.tick = tick;
        post_snapshot_input(&iso, &snap);
        iso.on_game_tick(tick);
        // The probe round-trips the isolate: the tick's batch is complete.
        iso.probe("true").unwrap();
        ops.extend(iso.drain_interacts());
    }
    let logs: Vec<String> = serde_json::from_value(iso.probe("__logs").unwrap()).unwrap();
    let err = iso
        .probe("__err")
        .unwrap()
        .as_str()
        .unwrap_or_default()
        .to_string();
    let sus = iso.probe("__sus").unwrap().as_i64().unwrap_or(0);
    let ok = iso.probe("__ok").unwrap();
    iso.join();
    (ops, logs, err, sus, ok)
}

fn world_walks(ops: &[InteractReq]) -> Vec<&InteractReq> {
    ops.iter()
        .filter(|op| matches!(op, InteractReq::Walk { .. } | InteractReq::WalkNear { .. }))
        .collect()
}

#[test]
fn walk_to_without_options_is_a_radius_two_walk() {
    // Frozen `opts?.radius ?? 2` (WalkExecutor.ts:227).
    let (ops, _, err, _, _) = walk_to_with("{}", 2);
    assert_eq!(err, "");
    match world_walks(&ops).as_slice() {
        [InteractReq::WalkNear {
            x: 3222,
            z: 3240,
            level: 0,
            radius: 2,
            allow_teleports: false,
            ..
        }] => {}
        other => panic!("one radius-2 world walk, got {other:?}"),
    }
}

#[test]
fn walk_to_an_explicit_false_teleport_opt_wins_over_a_true_one() {
    // Frozen resolveWalkUseTeleports (WalkExecutor.ts:165-177).
    for opts in [
        "{ useTeleportCatalog: true, policy: { useTeleports: false } }",
        "{ useTeleportCatalog: false, policy: { useTeleports: true } }",
    ] {
        let (ops, _, _, _, _) = walk_to_with(opts, 2);
        assert!(
            matches!(
                world_walks(&ops).as_slice(),
                [InteractReq::WalkNear {
                    allow_teleports: false,
                    ..
                }]
            ),
            "{opts}: {ops:?}"
        );
    }
    let (ops, _, _, _, _) = walk_to_with("{ policy: { useTeleports: true } }", 2);
    assert!(
        matches!(
            world_walks(&ops).as_slice(),
            [InteractReq::WalkNear {
                allow_teleports: true,
                ..
            }]
        ),
        "{ops:?}"
    );
}

#[test]
fn walk_to_distance_before_teleport_gates_teleports_on_the_span() {
    // policy.ts:61-74: the span from here to dest is 18.
    let allowed = |span: i32| {
        let (ops, _, _, _, _) = walk_to_with(
            &format!(
                "{{ useTeleportCatalog: true, policy: {{ distanceBeforeTeleport: {span} }} }}"
            ),
            2,
        );
        match world_walks(&ops).as_slice() {
            [InteractReq::WalkNear {
                allow_teleports, ..
            }] => *allow_teleports,
            other => panic!("one world walk, got {other:?}"),
        }
    };
    assert!(allowed(18));
    assert!(!allowed(19));
}

#[test]
fn walk_to_refuses_options_the_host_cannot_honour() {
    for (opts, name) in [
        (
            "{ policy: { allowTeleportIds: ['varrock'] } }",
            "allowTeleportIds",
        ),
        (
            "{ policy: { denyTeleportIds: ['varrock'] } }",
            "denyTeleportIds",
        ),
        ("{ policy: { useShips: false } }", "useShips"),
        ("{ pathFollow: { stallTicks: 3 } }", "pathFollow"),
        ("{ forceRepath: true }", "forceRepath"),
    ] {
        let (ops, _, err, _, _) = walk_to_with(opts, 2);
        assert!(
            err.contains("not impl") && err.contains(name),
            "{opts}: refused loud, got {err:?}"
        );
        assert!(world_walks(&ops).is_empty(), "{opts}: no walk: {ops:?}");
    }
}

#[test]
fn walk_to_runs_the_callers_sustain_every_tick_it_walks() {
    // Frozen `await Sustain.run()` on every follow pass (WalkExecutor.ts:844-853).
    let (ops, _, err, sus, ok) = walk_to_with("{}", 5);
    assert_eq!(err, "");
    assert_eq!(ok, serde_json::Value::Null, "still walking");
    assert_eq!(world_walks(&ops).len(), 1, "{ops:?}");
    assert!(sus >= 4, "Sustain ran {sus} times over 5 walking ticks");
}

#[test]
fn walk_to_a_throwing_sustain_stops_the_host_walk_it_armed() {
    let src = r#"
import { Traversal } from '../../api/walking/Traversal.js';
import { Sustain } from '../../api/sustain/Sustain.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__err = '';
        Sustain.set(() => { throw new Error('sustain boom'); });
        try {
            await Traversal.walkTo({ x: 3222, z: 3240, level: 0 });
        } catch (e) {
            globalThis.__err = String(e.message || e);
        }
    }
}
"#;
    let iso = LoadIsolate::spawn(src.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = ingame_snapshot();
    let mut ops = Vec::new();
    for tick in 1..=3 {
        snap.tick = tick;
        post_snapshot_input(&iso, &snap);
        iso.on_game_tick(tick);
        iso.probe("true").unwrap();
        ops.extend(iso.drain_interacts());
    }
    let err = iso.probe("__err").unwrap();
    iso.join();
    assert_eq!(err, "sustain boom", "the rejection reaches the caller");
    let token = match world_walks(&ops).as_slice() {
        [InteractReq::WalkNear { request_id, .. }] => *request_id,
        other => panic!("one world walk, got {other:?}"),
    };
    assert!(
        ops.contains(&InteractReq::AbortWalk { request_id: token }),
        "the failed walk stops its host follow: {ops:?}"
    );
}

#[test]
fn walk_to_avoid_zone_rectangles_ride_the_walk_request() {
    // Frozen Death Plateau walkSecretPath (deathplateau/nav.ts:153-163).
    let (ops, _, err, _, _) = walk_to_with(
        "{ radius: 0, timeoutMs: 180000, useTeleportCatalog: false, \
         avoidZones: [{ minX: 2848, maxX: 2880, minZ: 3590, maxZ: 3608, level: 0 }] }",
        2,
    );
    assert_eq!(err, "", "a rectangle routes; it is not refused");
    match world_walks(&ops).as_slice() {
        [InteractReq::Walk { avoid, .. }] => assert_eq!(
            avoid,
            &vec![script::shim::InspectAvoidWire::Rect {
                min_x: 2848,
                max_x: 2880,
                min_z: 3590,
                max_z: 3608,
                level: Some(0),
            }],
            "the rectangle crosses the FlatBuffer wire on the walk"
        ),
        other => panic!("one exact walk, got {other:?}"),
    }
    let (ops, _, err, _, _) = walk_to_with(
        "{ avoidZones: [{ minX: 5, maxX: 1, minZ: 0, maxZ: 0 }] }",
        2,
    );
    assert!(
        err.contains("avoidZones"),
        "an inverted rectangle is refused: {err}"
    );
    assert!(world_walks(&ops).is_empty());
}
