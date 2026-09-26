//! Compat cards: posted settings bag values reach `onPaint` after the loaded gate.

mod common;

use script::{LoadIsolate, LoadShape};

#[test]
fn compat_onpaint_reads_posted_settings_bag() {
    let src = r#"
import { Paint } from '../../paint/Paint.js';
export default class CraftStub extends LoopingBot {
    onPaint() {
        const p = Paint.begin(null, { accent: '#aabbcc' });
        p.title('CraftStub');
        p.row('Product: ' + this.settings.str('product', '?'));
        p.end();
    }
    loop() {}
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap();
    let mut bag = serde_json::Map::new();
    bag.insert("product".into(), serde_json::json!("Sapphire ring"));
    iso.post_settings_bag(&bag);
    common::post_snapshot_input(&iso, &common::ingame_snapshot());
    iso.on_game_tick(1);
    let _ = iso.probe("0");
    let paint = iso.paint().expect("onPaint forwarded");
    assert_eq!(paint.title.as_deref(), Some("CraftStub"));
    assert!(
        paint
            .lines
            .iter()
            .any(|l| l.contains("Product: Sapphire ring")),
        "settings.str must read posted bag in onPaint: {:?}",
        paint.lines
    );
    iso.join();
}
