//! Frozen `ChatDialog.continue` (`api/ui/dialogue/ChatDialog.ts:175-182`):
//! press the posted Continue, then true once the chat modal changes or the
//! page offers Continue again; false with no Continue or on the 3 s timeout.

mod common;

use script::interact::InteractReq;
use script::native::snapshot_source::{
    ItemRowInput, MakeButtonInput, MakeProductInput, SnapshotInput,
};
use script_js::{LoadIsolate, LoadShape};
use serde_json::Value;
use std::time::Duration;

const SRC: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__continued = null;
        globalThis.__continued = await ChatDialog.continue();
    }
}
"#;
const MAKE_ONE_SRC: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__made = null;
        globalThis.__made = await ChatDialog.makeOne('WOOL');
    }
}
"#;

fn page(snap: &mut SnapshotInput<'_>, modal: i32, can_continue: bool) {
    snap.chat_open = modal != -1;
    snap.chat_modal_id = modal;
    snap.chat_continue = can_continue;
}

fn post_tick(iso: &LoadIsolate, snap: &mut SnapshotInput<'_>, tick: u64) {
    snap.tick = tick;
    common::post_snapshot_input(iso, snap);
    iso.on_game_tick(tick);
    let _ = iso.probe("true");
}

fn start(modal: i32) -> (LoadIsolate, SnapshotInput<'static>) {
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    page(&mut snap, modal, true);
    post_tick(&iso, &mut snap, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::ContinueDialog { component_id: None }]
    );
    (iso, snap)
}

#[test]
fn a_next_page_that_still_offers_continue_is_true() {
    let (iso, mut snap) = start(4882);
    // Same chat interface, the next page, Continue still posted.
    post_tick(&iso, &mut snap, 2);
    assert_eq!(iso.probe("__continued").unwrap(), true);
    assert!(iso.drain_interacts().is_empty(), "one Continue per call");
    iso.join();
}

#[test]
fn a_changed_chat_modal_is_true() {
    let (iso, mut snap) = start(4882);
    page(&mut snap, 4887, false);
    post_tick(&iso, &mut snap, 2);
    assert_eq!(iso.probe("__continued").unwrap(), true);
    iso.join();
}

#[test]
fn an_unchanged_modal_without_continue_waits_out_the_timeout() {
    let (iso, mut snap) = start(4882);
    page(&mut snap, 4882, false);
    post_tick(&iso, &mut snap, 2);
    assert_eq!(
        iso.probe("__continued").unwrap(),
        Value::Null,
        "Continue gone on the same modal is not a moved page"
    );
    std::thread::sleep(Duration::from_millis(3_100));
    post_tick(&iso, &mut snap, 3);
    assert_eq!(iso.probe("__continued").unwrap(), false);
    iso.join();
}

#[test]
fn no_posted_continue_is_false_without_a_press() {
    let iso = LoadIsolate::spawn(SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let mut snap = common::ingame_snapshot();
    page(&mut snap, 4882, false);
    post_tick(&iso, &mut snap, 1);
    assert_eq!(iso.probe("__continued").unwrap(), false);
    assert!(iso.drain_interacts().is_empty());
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_dialogue__chatdialog_makeone__contract() {
    let iso = LoadIsolate::spawn(MAKE_ONE_SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let buttons = [
        MakeButtonInput { qty: 5, com_id: 80 },
        MakeButtonInput { qty: 1, com_id: 81 },
        MakeButtonInput {
            qty: -1,
            com_id: 82,
        },
    ];
    let other_buttons = [MakeButtonInput { qty: 1, com_id: 90 }];
    let products = [
        MakeProductInput {
            object_id: 1737,
            name: "Ball of wool",
            buttons: &buttons,
        },
        MakeProductInput {
            object_id: 1759,
            name: "Wool",
            buttons: &other_buttons,
        },
    ];
    let mut inventory = [ItemRowInput::nc(Some("Ball of wool"), 2)];
    inventory[0].id = 1737;
    let mut snap = common::ingame_snapshot();
    snap.main_modal_id = 3000;
    snap.make_products = &products;
    snap.inv = &inventory;
    post_tick(&iso, &mut snap, 1);
    assert_eq!(
        iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 81 }],
        "makeOne selects the first matching product's exact Make-1 button"
    );
    assert_eq!(iso.probe("__made").unwrap(), Value::Null);

    let mut updated_inventory = [ItemRowInput::nc(Some("Ball of wool"), 3)];
    updated_inventory[0].id = 1737;
    snap.main_modal_id = 3001;
    snap.inv = &updated_inventory;
    post_tick(&iso, &mut snap, 2);
    assert_eq!(iso.probe("__made").unwrap(), true);
    assert!(
        iso.drain_interacts().is_empty(),
        "one Make-1 click is issued"
    );
    iso.join();

    let refused = LoadIsolate::spawn(MAKE_ONE_SRC.into(), LoadShape::CompatClass, vec![]).unwrap();
    let no_make_one = [MakeButtonInput { qty: 5, com_id: 80 }];
    let products = [MakeProductInput {
        object_id: 1737,
        name: "Ball of wool",
        buttons: &no_make_one,
    }];
    let mut snap = common::ingame_snapshot();
    snap.main_modal_id = 3000;
    snap.make_products = &products;
    post_tick(&refused, &mut snap, 1);
    assert_eq!(refused.probe("__made").unwrap(), false);
    assert!(
        refused.drain_interacts().is_empty(),
        "a missing Make-1 control never dispatches"
    );
    refused.join();
}
