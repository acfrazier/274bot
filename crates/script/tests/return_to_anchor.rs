//! Frozen `createReturnToAnchorTask` (`Anchor.ts:85–128`) through the shim.

use script::isolate_fb::TileInput;
use script::{LoadIsolate, LoadShape};

mod common;
use common::{ingame_snapshot, post_snapshot_input};

fn validate_at(anchor_z: i32) -> serde_json::Value {
    let src = format!(
        r#"
import {{ createReturnToAnchorTask }} from '../../api/tasks/Anchor.js';
import Tile from '../../geometry/Tile.js';
export default class T extends LoopingBot {{
    loop() {{
        const host = {{ getAnchor: () => new Tile(3222, {anchor_z}, 0), leashRadius: () => 20 }};
        const task = createReturnToAnchorTask(host, {{ slack: 4, arriveRadius: 2 }});
        globalThis.__valid = task.validate();
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
    let valid = iso.probe("globalThis.__valid").unwrap();
    iso.join();
    valid
}

#[test]
fn validate_keys_off_the_leash_not_the_arrive_disk() {
    assert_eq!(
        validate_at(3212),
        false,
        "10 tiles out is inside leash 20 + slack 4, though past arriveRadius 2 + slack"
    );
    assert_eq!(
        validate_at(3192),
        true,
        "30 tiles out is beyond leash 20 + slack 4"
    );
}
