// Task 9: Load shape detection. No V8 yet — a cheap marker scan decides
// which loader a JS source belongs to. `CompatDefineBot` (old rs2b0t
// `defineBot(...)` / manifest marker), `NativeTick` (`export function
// tick`), else `Reject`. The three string-literal fixtures at the top are
// the brief's fixtures; the extra cases pin precedence and marker
// variants.

use script::load::{detect_shape, LoadShape};

// Fixture 1: modern native-shape bot, exported tick function.
const NATIVE_FIXTURE: &str = r#"
import { Game } from '@274bot/api';
import { Traversal } from '@274bot/nav';

export function tick(ctx) {
    if (!Game.ingame()) return;
    Traversal.walkTo(ctx.tile);
}
"#;

// Fixture 2: old rs2b0t defineBot shape.
const COMPAT_FIXTURE: &str = r#"
defineBot({
    name: 'HerbCleaner',
    onStart() { this.wait(0); },
    loop() { this.cleanHerbs(); },
});
"#;

// Fixture 3: something that is neither shape — reject.
const REJECT_FIXTURE: &str = r#"
const x = 1 + 1;
console.log('not a bot shape at all', x);
"#;

// Fixture 4: BoneBurier-shaped TS — default-export LoopingBot subclass
// with typed private fields (the catalog shape).
const CLASS_TS_FIXTURE: &str = r#"
export default class BoneBurier extends LoopingBot {
    private n: number = 0;
}
"#;

#[test]
fn native_fixture_is_native_tick() {
    assert_eq!(detect_shape(NATIVE_FIXTURE), LoadShape::NativeTick);
}

#[test]
fn compat_fixture_is_compat_define_bot() {
    assert_eq!(detect_shape(COMPAT_FIXTURE), LoadShape::CompatDefineBot);
}

#[test]
fn reject_fixture_is_reject() {
    assert_eq!(detect_shape(REJECT_FIXTURE), LoadShape::Reject);
}

#[test]
fn native_marker_variants() {
    // Plain `export function tick` and the async variant both count.
    assert_eq!(
        detect_shape("export function tick() {}"),
        LoadShape::NativeTick
    );
    assert_eq!(
        detect_shape("export async function tick() { await ctx.delay(1); }"),
        LoadShape::NativeTick
    );
    // A bare `function tick` (no export) is not the load shape we gate on.
    assert_eq!(detect_shape("function tick() {}"), LoadShape::Reject);
}

#[test]
fn compat_marker_variants() {
    assert_eq!(
        detect_shape("defineBot({ name: 'A' });"),
        LoadShape::CompatDefineBot
    );
    assert_eq!(
        detect_shape("export const __rs2b0tManifest = { api: '1.0' };"),
        LoadShape::CompatDefineBot
    );
    // A bare mention of the old API name is not the marker.
    assert_eq!(
        detect_shape("// the old bot loader call was removed in v2\nconst x = 1;"),
        LoadShape::Reject
    );
}

#[test]
fn compat_wins_over_native_when_both_present() {
    // A source carrying both markers is an old compat bot that happens to
    // also export a tick: route it to the compat loader, never native.
    assert_eq!(
        detect_shape("defineBot({ loop() {} });\nexport function tick() {}"),
        LoadShape::CompatDefineBot
    );
}

#[test]
fn class_fixture_is_compat_class() {
    assert_eq!(detect_shape(CLASS_TS_FIXTURE), LoadShape::CompatClass);
}

#[test]
fn compat_class_marker_variants() {
    // All three catalog base classes route to the compat class loader.
    for base in ["LoopingBot", "TaskBot", "TreeBot"] {
        assert_eq!(
            detect_shape(&format!("export default class X extends {base} {{}}")),
            LoadShape::CompatClass
        );
    }
    // A default-export class extending something else is not the shape.
    assert_eq!(
        detect_shape("export default class X extends Object {}"),
        LoadShape::Reject
    );
    // A bare `extends LoopingBot` class that is not default-exported is
    // not a loadable card.
    assert_eq!(
        detect_shape("class X extends LoopingBot {}"),
        LoadShape::Reject
    );
}

#[test]
fn compat_class_wins_over_native_tick() {
    // Class markers win over the native `tick` export, exactly like the
    // defineBot markers.
    assert_eq!(
        detect_shape("export default class X extends LoopingBot {}\nexport function tick() {}"),
        LoadShape::CompatClass
    );
}

#[test]
fn compat_define_bot_still_wins_over_class() {
    // defineBot markers keep the highest precedence.
    assert_eq!(
        detect_shape("defineBot({ name: 'A' });\nexport default class X extends LoopingBot {}"),
        LoadShape::CompatDefineBot
    );
}

#[test]
fn empty_and_whitespace_reject() {
    assert_eq!(detect_shape(""), LoadShape::Reject);
    assert_eq!(detect_shape("   \n\t  "), LoadShape::Reject);
}
