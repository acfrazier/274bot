//! Frozen `Traversal.walkResilient` options the host maps rather than
//! runs: `maxBudget` (`Traversal.ts:24, 106`); and frozen `Traversal.walkTo`
//! `WalkOptions` (`WalkExecutor.ts:113–139, 165–177, 226–245`).

use script::isolate_fb::TileInput;
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

fn walk_with_budget(max_budget: u64) -> (Vec<String>, Vec<InteractReq>) {
    let src = format!(
        r#"
import {{ Traversal }} from '../../api/walking/Traversal.js';
export default class T extends LoopingBot {{
    loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__logs = [];
        Traversal.walkResilient({{ x: 3222, z: 3240, level: 0 }}, {{
            radius: 0,
            maxBudget: {max_budget},
            log: (m) => globalThis.__logs.push(String(m)),
        }});
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
    snap.tick = 2;
    post_snapshot_input(&iso, &snap);
    iso.on_game_tick(2);
    let logs: Vec<String> = serde_json::from_value(iso.probe("__logs").unwrap()).unwrap();
    let ops = iso.drain_interacts();
    iso.join();
    (logs, ops)
}

fn walked(ops: &[InteractReq]) -> bool {
    ops.iter().any(|op| {
        matches!(
            op,
            InteractReq::Walk {
                x: 3222,
                z: 3240,
                ..
            }
        )
    })
}

#[test]
fn a_max_budget_within_the_host_bound_walks_without_a_note() {
    // Frozen JiveKQ passes maxBudget 120_000 (route.ts:17): the host
    // router searches its fixed bound, at least that far.
    let (logs, ops) = walk_with_budget(120_000);
    assert!(walked(&ops), "the baked walk goes out: {ops:?}");
    assert!(
        !logs.iter().any(|line| line.contains("maxBudget")),
        "{logs:?}"
    );
}

#[test]
fn a_max_budget_above_the_host_bound_is_logged_not_refused() {
    let (logs, ops) = walk_with_budget(9_000_000);
    assert!(walked(&ops), "the walk still runs: {ops:?}");
    assert!(
        logs.iter().any(|line| line
            == "walkResilient: maxBudget 9000000 is above the host search bound 4000000; routes search 4000000"),
        "{logs:?}"
    );
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
        ("{ avoidZones: ['white-wolf-mountain'] }", "avoidZones"),
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
fn walk_to_max_expansions_above_the_host_bound_is_logged_not_refused() {
    let (ops, logs, err, _, _) = walk_to_with("{ maxExpansions: 9000000 }", 2);
    assert_eq!(err, "");
    assert_eq!(world_walks(&ops).len(), 1, "{ops:?}");
    assert!(
        logs.iter().any(|line| line
            == "walkTo: maxExpansions 9000000 is above the host search bound 4000000; routes search 4000000"),
        "{logs:?}"
    );
    let (_, logs, _, _, _) = walk_to_with("{ maxExpansions: 500000 }", 2);
    assert!(
        !logs.iter().any(|line| line.contains("maxExpansions")),
        "{logs:?}"
    );
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
