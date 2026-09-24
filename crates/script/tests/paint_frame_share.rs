//! The isolate forwards one shared paint frame per changed tick.
//!
//! The frame is built on the isolate thread and read by every host paint
//! view; it crosses as the typed value and is shared by `Arc`, so a read
//! copies a handle and not the frame.

use std::sync::Arc;

use script::{LoadIsolate, LoadShape};

mod common;

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

/// 600 body rows: over the 512-line cap the wire decoder used to enforce.
const OVER_CAP_LINES: &str = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, {});
        for (let i = 0; i < 600; i++) p.text('row ' + i);
        p.end();
    }
}
"#;

/// 40 advertised buttons: over the 32-button cap.
const OVER_CAP_BUTTONS: &str = r#"
import { Paint } from '../../paint/Paint.js';
export default class T extends LoopingBot {
    onPaint() {
        const items = [];
        for (let i = 0; i < 40; i++) items.push({ id: 'b' + i, label: 'B' + i });
        const p = Paint.begin(null, {});
        p.buttons(items);
        p.end();
    }
}
"#;

/// A frame over a cap is dropped and logged, never forwarded — the check the
/// wire decoder used to make on the host side.
#[test]
fn an_over_cap_frame_is_dropped_and_logged() {
    for (src, want) in [
        (OVER_CAP_LINES, "vector length 600 exceeds cap 512"),
        (OVER_CAP_BUTTONS, "vector length 40 exceeds cap 32"),
    ] {
        let iso = spawn(src, LoadShape::CompatClass);
        let _ = iso.drain_logs();
        tick(&iso, 1);
        assert!(iso.paint().is_none(), "over-cap frame not forwarded");
        let logs = iso.drain_logs();
        assert!(
            logs.iter().any(|l| l.contains(want)),
            "expected {want:?} in {logs:?}"
        );
        tick(&iso, 2);
        let again = iso.drain_logs();
        assert!(
            !again.iter().any(|l| l.contains("exceeds cap")),
            "an unchanged over-cap frame is logged once: {again:?}"
        );
        iso.join();
    }
}

/// A reset re-stamps the held frame with the new generation, so an overlay
/// that captured the pre-reset generation fails the host click/select check
/// until the next forwarded frame.
#[test]
fn a_session_reset_restamps_the_held_frame() {
    let iso = spawn(STEADY, LoadShape::CompatClass);
    tick(&iso, 1);
    let before = iso.paint().expect("first frame");
    iso.reset_session_work();
    let held = iso.paint().expect("held frame");
    assert_eq!(
        held.generation,
        before.generation + 1,
        "the held frame carries the post-reset generation"
    );
    assert_eq!(held.lines, before.lines, "the frame itself is unchanged");
    assert!(
        !Arc::ptr_eq(&before, &held),
        "a re-stamped frame is a new shared frame"
    );
    tick(&iso, 2);
    let next = iso.paint().expect("next frame");
    assert_eq!(
        next.generation, held.generation,
        "the next forwarded frame carries the same generation"
    );
    iso.join();
}
