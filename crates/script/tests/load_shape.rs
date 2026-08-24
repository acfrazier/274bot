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
fn empty_and_whitespace_reject() {
    assert_eq!(detect_shape(""), LoadShape::Reject);
    assert_eq!(detect_shape("   \n\t  "), LoadShape::Reject);
}
