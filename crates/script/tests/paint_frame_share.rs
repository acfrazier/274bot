//! The isolate forwards one shared paint frame per changed tick.
//!
//! The frame is built on the isolate thread and read by every host paint
//! view; it crosses as the typed value and is shared by `Arc`, so a read
//! copies a handle and not the frame.

use std::sync::Arc;

use script::{LoadIsolate, LoadShape};

const TICKED: &str = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, {});
        p.text('tick ' + globalThis.__rs2b0t_host.tick);
        p.end();
    }
}
"#;

const STEADY: &str = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, {});
        p.text('steady');
        p.end();
    }
}
"#;

fn spawn(src: &str, shape: LoadShape) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), shape, vec![]).expect("bot loads")
}

fn tick(iso: &LoadIsolate, n: u64) {
    iso.on_game_tick(n);
    let _ = iso.probe("0");
}

#[test]
fn paint_reads_share_the_forwarded_frame() {
    let iso = spawn(TICKED, LoadShape::CompatClass);
    tick(&iso, 1);
    let first = iso.paint().expect("first frame");
    let again = iso.paint().expect("second read");
    assert!(
        Arc::ptr_eq(&first, &again),
        "two reads of one forwarded frame share it"
    );
    assert_eq!(first.lines, vec!["tick 1".to_string()]);

    tick(&iso, 2);
    let next = iso.paint().expect("second frame");
    assert!(
        !Arc::ptr_eq(&first, &next),
        "a changed frame is a new shared frame"
    );
    assert_eq!(next.lines, vec!["tick 2".to_string()]);
    assert_eq!(first.lines, vec!["tick 1".to_string()], "readers keep it");
    iso.join();
}

#[test]
fn an_unchanged_tick_does_not_forward_a_new_frame() {
    let iso = spawn(STEADY, LoadShape::CompatClass);
    tick(&iso, 1);
    let first = iso.paint().expect("first frame");
    tick(&iso, 2);
    tick(&iso, 3);
    let after = iso.paint().expect("still framed");
    assert!(
        Arc::ptr_eq(&first, &after),
        "an identical pass is not re-sent"
    );
    iso.join();
}
