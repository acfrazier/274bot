//! Frozen `ChatDialog.continue` (`api/ui/dialogue/ChatDialog.ts:175-182`):
//! press the posted Continue, then true once the chat modal changes or the
//! page offers Continue again; false with no Continue or on the 3 s timeout.

mod common;

use script::isolate_fb::SnapshotInput;
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
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
    assert_eq!(iso.drain_interacts(), vec![InteractReq::ContinueDialog]);
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
