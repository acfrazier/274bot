use super::*;

const MANIFEST: &str = "export const manifest = { apiVersion: 2, name: 'Walker', version: '1', \
     author: 'alice', description: 'Walks', category: 'Examples', tags: ['walk'], \
     settings: { spot: { type: 'string', default: 'Lumbridge', label: 'Spot' } }, \
     capabilities: [] };\n";

fn native(body: &str) -> String {
    format!("{MANIFEST}{body}")
}

fn refusal_code(refusal: &OriginRefusal) -> &'static str {
    match refusal.declaration() {
        Some(ExtractionError::Declaration(error)) => error.code(),
        other => panic!("expected a declaration refusal, got {other:?} ({refusal})"),
    }
}

#[test]
fn manifest_form_entry_is_checked_v2_with_erased_execution() {
    let origin = native("export function tick(api) { return api.tick; }\n");
    let prepared = prepare_raw_origin_for_test("walker.js", &origin, &[]).unwrap();
    assert_eq!(prepared.checked.shape(), LoadShape::NativeTick);
    assert_eq!(prepared.checked.family(), ApiFamily::V2);
    assert!(prepared.checked.v2_native());
    assert_eq!(
        prepared.js.len(),
        origin.len(),
        "erasure keeps every byte offset"
    );
    assert!(
        prepared.js.starts_with(';'),
        "erasure keeps the statement boundary"
    );
    assert!(!prepared.js.contains("manifest"), "{}", prepared.js);
    assert!(prepared
        .js
        .ends_with("export function tick(api) { return api.tick; }\n"));
}

#[test]
fn checked_origin_keeps_the_manifest_settings_and_metadata() {
    let origin = native("export async function tick(api) {}\n");
    let checked = check_origin(Path::new("walker.ts"), &origin).unwrap();
    let manifest = checked.manifest().expect("native manifest");
    assert_eq!(manifest.name(), "Walker");
    assert_eq!(manifest.tags(), ["walk".to_owned()]);
    assert_eq!(manifest.settings_schema().len(), 1);
    assert_eq!(manifest.settings_schema()[0].id, "spot");
}

#[test]
fn native_tick_without_manifest_is_manifest_required() {
    for origin in [
        "export function tick(api) {}\n",
        "export async function tick(api) {}\n",
        "export const apiVersion = 2;\nexport function tick(api) {}\n",
        "// defineBot(\nconst marker = '__rs2b0tManifest';\nexport function tick(api) {}\n",
    ] {
        let refusal = prepare_raw_origin_for_test("bot.js", origin, &[]).unwrap_err();
        assert_eq!(refusal_code(&refusal), "manifest-required", "{origin}");
        let text = refusal.to_string();
        assert!(
            text.contains("export const manifest = { apiVersion: 2, name: 'My script'"),
            "{text}"
        );
    }
}

#[test]
fn standalone_api_version_without_tick_is_manifest_required() {
    let refusal =
        prepare_raw_origin_for_test("bot.ts", "export const apiVersion = 2;\n", &[]).unwrap_err();
    assert_eq!(refusal_code(&refusal), "manifest-required");
}

#[test]
fn recognized_compat_script_is_v1_with_unchanged_execution() {
    let origin = "import { defineBot } from '@rs2b0t/api';\n\
                  export default defineBot({ name: 'Compat', create: () => ({ loop() {} }) });\n";
    let prepared = prepare_raw_origin_for_test("compat.js", origin, &[]).unwrap();
    assert_eq!(prepared.checked.shape(), LoadShape::CompatDefineBot);
    assert_eq!(prepared.checked.family(), ApiFamily::V1);
    assert_eq!(prepared.js, origin, "compat JS executes byte-for-byte");
}

#[test]
fn compat_script_keeps_its_explicit_version_check() {
    let origin = "export const apiVersion = 2;\n\
                  export default defineBot({ name: 'Compat', create: () => ({}) });\n";
    let refusal = prepare_raw_origin_for_test("compat.js", origin, &[]).unwrap_err();
    assert_eq!(
        refusal,
        OriginRefusal::CompatVersion("api-version-conflict: v2 with compatibility shape".into())
    );
    let accepted = "export const apiVersion = 1;\n\
                    export default defineBot({ name: 'Compat', create: () => ({}) });\n";
    let prepared = prepare_raw_origin_for_test("compat.js", accepted, &[]).unwrap();
    assert_eq!(prepared.checked.family(), ApiFamily::V1);
}

/// The three frozen rs2b0t shapes at `90a69240a` that the loader does not
/// recognize keep `NotBotShape`, never a migration diagnostic.
#[test]
fn unrecognized_rs2b0t_defaults_keep_not_bot_shape() {
    let chicken = (
        "/rs2b0t/bot/scripts/bot/ChickenKiller.js".to_owned(),
        "export default class ChickenKiller {}\nexport const SETTINGS = {};\n".to_owned(),
    );
    let gathering = (
        "/rs2b0t/bot/scripts/bot/GatheringBot.js".to_owned(),
        "export default class GatheringBot {}\nexport const GATHERING_SETTINGS = {};\n".to_owned(),
    );
    type Siblings = Vec<(String, String)>;
    let cases: [(&str, &str, Siblings); 3] = [
        (
            "CowKiller.ts",
            "import ChickenKiller, { SETTINGS as CHICKEN_SETTINGS } from './ChickenKiller.js';\n\
             export const SETTINGS = { ...CHICKEN_SETTINGS };\n\
             export default class CowKiller extends ChickenKiller {}\n",
            vec![chicken],
        ),
        (
            "Woodcutter.ts",
            "import GatheringBot, { GATHERING_SETTINGS } from './GatheringBot.js';\n\
             export const WOODCUTTER_SETTINGS = { ...GATHERING_SETTINGS };\n\
             export default class Woodcutter extends GatheringBot {}\n",
            vec![gathering],
        ),
        (
            "EdgevilleMonkeyBars.ts",
            "import { TaskBot } from '../../api/bot/Bot.js';\n\
             class EdgevilleMonkeyBars extends TaskBot {}\n\
             export default EdgevilleMonkeyBars;\n",
            Vec::new(),
        ),
    ];
    for (file, origin, siblings) in cases {
        let refusal = prepare_raw_origin_for_test(file, origin, &siblings).unwrap_err();
        assert_eq!(refusal, OriginRefusal::NotBotShape, "{file}");
    }
}

#[test]
fn compat_markers_without_default_are_not_bot_shapes() {
    for origin in [
        "defineBot({ name: 'A' });\n",
        "export const __rs2b0tManifest = { api: '1.0' };\n",
    ] {
        let refusal = prepare_raw_origin_for_test("bot.js", origin, &[]).unwrap_err();
        assert_eq!(refusal, OriginRefusal::NotBotShape, "{origin}");
    }
}

/// A catalog compat card that imports a module the loader cannot provide is
/// still a compat card: Load lists it as unloadable instead of dropping it.
#[test]
fn compat_entry_with_an_unresolved_import_keeps_its_shape() {
    let origin = "import { acquireTool } from '../../api/acquisition/ToolAcquire.js';\n\
                  export default class EssMiner extends LoopingBot { loop() { acquireTool(); } }\n";
    let prepared = prepare_raw_origin_for_test("EssMiner.ts", origin, &[]).unwrap();
    assert_eq!(prepared.checked.shape(), LoadShape::CompatClass);
    assert_eq!(prepared.checked.family(), ApiFamily::V1);
}

#[test]
fn memory_siblings_resolve_and_a_missing_sibling_refuses() {
    let origin =
        native("import { step } from './helper.js';\nexport function tick(api) { step(); }\n");
    let siblings = vec![(
        "/rs2b0t/bot/scripts/bot/helper.js".to_owned(),
        "export function step() {}\n".to_owned(),
    )];
    let prepared = prepare_raw_origin_for_test("bot.js", &origin, &siblings).unwrap();
    assert!(prepared.checked.v2_native());
    let refusal = prepare_raw_origin_for_test("bot.js", &origin, &[]).unwrap_err();
    assert!(
        matches!(
            refusal.declaration(),
            Some(ExtractionError::UnresolvedModule(_))
        ),
        "{refusal}"
    );
}

#[test]
fn native_fixture_builder_supplies_a_valid_manifest() {
    let source = NativeFixture::new("probe")
        .settings("{ count: { type: 'number', default: 3 } }")
        .source("export function tick(api) {}\n");
    let checked = check_origin(Path::new("probe.js"), &source).unwrap();
    let manifest = checked.manifest().unwrap();
    assert_eq!(manifest.name(), "probe");
    assert_eq!(manifest.settings_schema()[0].id, "count");
    assert!(native_fixture("export function tick(api) {}\n").starts_with("export const manifest"));
}
