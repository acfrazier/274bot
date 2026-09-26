//! Frozen `Traversal.walkResilient` options the host maps rather than
//! runs: `maxBudget` (`Traversal.ts:24, 106`).

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
