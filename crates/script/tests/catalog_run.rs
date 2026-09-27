// Catalog-run checks that do not require the optional rs2b0t checkout.
// Bright-card / pin-wide regressions live in LIVE harnesses and scenario cells.

mod common;

#[test]
fn bank_picker_import_links_and_awaits_host_selection() {
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::snapshot::WorldTile;
    use script::isolate_fb::{BankSelectionInput, NativeFactsInput};
    use script::{LoadIsolate, LoadShape};
    use std::sync::Arc;
    let banks = Arc::new(NamedBankFacts::from_banks(vec![
        NamedBank::new(
            "Air near",
            WorldTile {
                x: 3224,
                z: 3218,
                level: 0,
            },
        ),
        NamedBank::new(
            "Walk near",
            WorldTile {
                x: 3200,
                z: 3210,
                level: 0,
            },
        ),
    ]));
    let iso = LoadIsolate::spawn_with_content(
        r#"
import { nearestBankReachable } from '../../api/bank/BankLocations.js';
import { Navigator } from '../../event/webwalk/Navigator.js';
export default class Probe extends LoopingBot {
    async loop() {
        globalThis.picked = await nearestBankReachable({ x: 3222, z: 3218, level: 0 }, Navigator);
    }
}
"#
        .into(),
        LoadShape::CompatClass,
        vec![],
        None,
        banks,
    )
    .expect("Alcher/LeatherCrafter bank imports must link");
    let mut snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(1);
    let _ = iso.probe("true");
    let request_id = iso
        .drain_interacts()
        .into_iter()
        .find_map(|req| match req {
            script::shim::InteractReq::SelectBank { request_id, .. } => Some(request_id),
            _ => None,
        })
        .expect("reachable picker starts its native selection");
    for (tick, id) in [(2, request_id - 1), (3, request_id)] {
        snapshot.tick = tick;
        iso.post_snapshot(script::isolate_fb::encode_snapshot_with_native(
            &snapshot,
            NativeFactsInput {
                bank_selection: BankSelectionInput {
                    request_id: id,
                    generation: 1,
                    bank_index: 1,
                    kind: 2,
                },
                ..Default::default()
            },
        ));
        iso.on_game_tick(tick);
        let actual = iso.probe("typeof picked === 'undefined' ? null : [picked.name, picked.tile.x, picked.tile.z, picked.tile.level]").unwrap();
        if tick == 2 {
            assert_eq!(
                actual,
                serde_json::Value::Null,
                "a stale completion cannot settle the current selection"
            );
        } else {
            assert_eq!(
                actual,
                serde_json::json!(["Walk near", 3200, 3210, 0]),
                "the awaited result is the host's reachable bank, not a JS air-nearest substitute"
            );
        }
    }
    iso.join();
}
