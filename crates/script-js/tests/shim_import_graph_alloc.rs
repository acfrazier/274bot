#![cfg(feature = "test-hooks")]

use std::collections::BTreeSet;

use rustyscript::Module;
use script::{ScriptKind, ScriptSource};
use script_js::shim::{
    filter_gated_modules_for_card_test, filter_gated_modules_for_test, shim_modules, BOT_MODULE,
    MAIN_MODULE,
};
use script_js::{CacheMeta, JsCache, JsLibrary};
const SIBLING_MODULE: &str = "/rs2b0t/bot/scripts/bot/sibling.js";

const THIEVER_SOURCE: &str = r#"
import { foodHealAmount, shouldEatToUseFood, MIN_EAT_HP } from '../../api/combat/food.js';
import { createReturnToAnchorTask, resolveRunAnchor, tileWithinLeash } from '../../api/tasks/Anchor.js';
import { TaskBot } from '../../api/bot/Bot.js';
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
import { Reachability } from '../../event/webwalk/geometry/Reachability.js';
import Tile from '../../geometry/Tile.js';
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
import { Bank } from '../../api/bank/Bank.js';
import { openBankAccess } from '../../api/bank/Banking.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Paint } from '../../paint/Paint.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
import { SettingsStore } from '../../runtime/Settings.js';
import { Skills } from '../../api/skills/Skills.js';
import { ContinueDialog } from '../../api/tasks/ContinueDialog.js';
import { GroundItems } from '../../api/grounditems/GroundItems.js';
import { Npcs } from '../../api/npcs/Npcs.js';
import { Traversal } from '../../api/walking/Traversal.js';
import { nearestBank } from '../../api/bank/BankLocations.js';
import { walkOpening } from '../../event/webwalk/walkOpening.js';
import { PICKPOCKET_TARGET_NAMES } from '../../data/pickpocketTargets.js';
import { fmtDuration } from '../../paint/paintLogic.js';
import { chooseTarget } from '../../api/thieving/targets.js';
import {
    STUN_COMBAT_TICKS,
    autoFoodBanking,
    canStealNow,
    closeBankAndConfirmCount,
    countFood,
    foodMatches,
    shouldRestockFood,
    THIEVER_BANKING_OPTIONS,
    withdrawTo
} from '../../api/thieving/stealRules.js';
import { scriptFood } from '../../api/loadout/loadoutPlan.js';
import { LOADOUT_SETTING } from '../../api/loadout/loadoutSetting.js';

export default class ThieverImportGraphProbe extends TaskBot {
    onStart() {}
    loop() {}
}
"#;

fn thiever_modules() -> Vec<Module> {
    let mut modules = shim_modules();
    modules.push(Module::new(
        SIBLING_MODULE,
        "import '../../api/map/regions.js';",
    ));
    modules.push(Module::new(BOT_MODULE, THIEVER_SOURCE));
    modules
}

#[test]
fn small_import_source_closure_allocation_is_bounded() {
    let mut warm_modules = thiever_modules();
    filter_gated_modules_for_test(&mut warm_modules, [SIBLING_MODULE, BOT_MODULE]);
    let mut modules = thiever_modules();
    let allocated = allocation_counter::measure(|| {
        filter_gated_modules_for_test(&mut modules, [SIBLING_MODULE, BOT_MODULE]);
    });
    const LIMIT: u64 = 128 * 1024;
    eprintln!(
        "per-start import closure allocations={} bytes; limit={LIMIT}",
        allocated.bytes_total
    );
    assert!(
        allocated.bytes_total <= LIMIT,
        "per-start import closure allocated {} bytes, above {LIMIT}",
        allocated.bytes_total
    );
}
#[test]
fn frozen_bright_catalog_cards_gate_with_bounded_per_start_allocations() {
    const FROZEN_REVISION: &str = "ae56ccc95ba5681b12f1f66ec2adea14427fe497";
    const THIEVER_C_PRIME_SHIMS: &[&str] = &[
        "/rs2b0t/bot/adapter/ClientAdapter.js",
        "/rs2b0t/bot/api/bank/Bank.js",
        "/rs2b0t/bot/api/bank/BankLocations.js",
        "/rs2b0t/bot/api/bank/Banking.js",
        "/rs2b0t/bot/api/bot/Bot.js",
        "/rs2b0t/bot/api/combat/food.js",
        "/rs2b0t/bot/api/execution/Execution.js",
        "/rs2b0t/bot/api/game/Game.js",
        "/rs2b0t/bot/api/grounditems/GroundItems.js",
        "/rs2b0t/bot/api/inventory/Inventory.js",
        "/rs2b0t/bot/api/loadout/loadoutPlan.js",
        "/rs2b0t/bot/api/loadout/loadoutSetting.js",
        "/rs2b0t/bot/api/npcs/Npcs.js",
        "/rs2b0t/bot/api/query/Query.js",
        "/rs2b0t/bot/api/skills/Skills.js",
        "/rs2b0t/bot/api/sustain/Sustain.js",
        "/rs2b0t/bot/api/tasks/Anchor.js",
        "/rs2b0t/bot/api/tasks/ContinueDialog.js",
        "/rs2b0t/bot/api/thieving/stealRules.js",
        "/rs2b0t/bot/api/thieving/targets.js",
        "/rs2b0t/bot/api/ui/dialogue/ChatDialog.js",
        "/rs2b0t/bot/api/walking/Traversal.js",
        "/rs2b0t/bot/data/pickpocketTargets.js",
        "/rs2b0t/bot/event/webwalk/geometry/Reachability.js",
        "/rs2b0t/bot/event/webwalk/walkOpening.js",
        "/rs2b0t/bot/geometry/Tile.js",
        "/rs2b0t/bot/paint/Paint.js",
        "/rs2b0t/bot/paint/paintLogic.js",
        "/rs2b0t/bot/runtime/ScriptRunner.js",
        "/rs2b0t/bot/runtime/Settings.js",
        "/rs2b0t/bot/shim/_kernel.js",
    ];
    let Some(root) = api::test_support::local_asset_path("RS2B0T", "", "frozen ae56ccc95 catalog")
    else {
        return;
    };
    let revision = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("read frozen catalog revision");
    assert!(
        revision.status.success(),
        "RS2B0T must be a git checkout: {}",
        String::from_utf8_lossy(&revision.stderr)
    );
    assert_eq!(
        String::from_utf8(revision.stdout).unwrap().trim(),
        FROZEN_REVISION,
        "RS2B0T must be the pinned frozen catalog"
    );
    let scratch =
        std::env::temp_dir().join(format!("274bot-shim-import-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("create catalog probe directory");
    let mut library =
        JsLibrary::with_cache(scratch.join("js-scripts.json"), scratch.join("js-cache"));
    library
        .register_rs2b0t(&root, &scratch.join("rs2b0t-path"))
        .expect("register frozen catalog");
    let cache = JsCache::new(scratch.join("siblings"));

    let mut bright_names = library
        .cards()
        .iter()
        .filter(|card| {
            card.source == ScriptSource::Catalog
                && card.unloadable.is_none()
                && !script_js::is_catalog_dim(&card.name)
        })
        .map(|card| card.name.clone())
        .collect::<Vec<_>>();
    bright_names.sort_unstable();
    assert_eq!(bright_names.len(), 59, "frozen bright catalog card count");

    let formerly_ungated = [
        "Duel Arena Combat Trainer",
        "ChaosDruidKiller",
        "MossGiant",
        "BrimhavenMossGiants",
        "GnomeMagicChopper",
        "Alcher",
        "TannerBot",
        "LeatherCrafter",
    ];
    for name in formerly_ungated {
        assert!(
            bright_names.iter().any(|bright| bright == name),
            "reviewed bright card {name} is missing from the pinned catalog"
        );
    }

    let shim_paths = shim_modules()
        .iter()
        .map(|module| module.filename().to_path_buf())
        .collect::<BTreeSet<_>>();
    let shim_count = shim_paths.len();
    let mut warm = shim_modules();
    warm.push(Module::new(BOT_MODULE, "export default class T {}"));
    warm.push(script_js::main_module_for_import_gate_test(
        script_js::LoadShape::CompatClass,
        script_js::ApiFamily::Unversioned,
    ));
    filter_gated_modules_for_card_test(
        &mut warm,
        [BOT_MODULE, MAIN_MODULE],
        "catalog allocation warm-up",
    );

    let mut failures = Vec::new();
    let mut ungated = Vec::new();
    let mut max_bytes = 0u64;
    let mut max_name = String::new();
    let mut thiever_modules = None;
    let mut jive_dragons_measured = false;
    for name in bright_names {
        if let Err(error) = library.ensure_js(ScriptSource::Catalog, &name) {
            failures.push(format!("{name}: transpile {error}"));
            continue;
        }
        let Some(card) = library.get(ScriptSource::Catalog, &name).cloned() else {
            failures.push(format!("{name}: missing after transpile"));
            continue;
        };
        let siblings = match script_js::resolve_sibling_modules(
            &card.path,
            &card.origin,
            &cache,
            CacheMeta {
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
                shape: Some(format!("{:?}", card.shape)),
                api_family: None,
            },
        ) {
            Ok(siblings) => siblings,
            Err(error) => {
                failures.push(format!("{name}: siblings {error}"));
                continue;
            }
        };
        let siblings = siblings
            .into_iter()
            .map(|(url, source)| (url, script_js::shim::remap_catalog_imports(&source)))
            .collect::<Vec<_>>();
        let source = script_js::shim::remap_catalog_imports(&card.js);
        let source_bytes = source.len()
            + siblings
                .iter()
                .map(|(_, source)| source.len())
                .sum::<usize>();

        let mut modules = shim_modules();
        for (url, source) in &siblings {
            modules.push(Module::new(url, source));
        }
        modules.push(Module::new(BOT_MODULE, source.as_str()));
        modules.push(script_js::main_module_for_import_gate_test(
            card.shape,
            card.api_family,
        ));
        let before = modules.len();
        let roots = siblings
            .iter()
            .map(|(url, _)| url.as_str())
            .chain([BOT_MODULE, MAIN_MODULE])
            .collect::<Vec<_>>();
        let allocation = allocation_counter::measure(|| {
            filter_gated_modules_for_card_test(&mut modules, roots, &name);
        });
        let gated = modules.len() < before;
        modules
            .pop()
            .expect("temporary main module root is retained");
        let retained_shims = modules
            .iter()
            .filter(|module| shim_paths.contains(module.filename()))
            .count();
        if name == "JiveDragons" {
            jive_dragons_measured = true;
        }
        if !gated {
            ungated.push(name.clone());
        }
        eprintln!(
            "FROZEN_CATALOG {name}: siblings={} source_bytes={source_bytes} per_start_alloc={} retained_shims={retained_shims}/{shim_count} gated={gated}",
            siblings.len(),
            allocation.bytes_total
        );
        if name == "Thiever" {
            let mut paths = modules
                .iter()
                .filter(|module| shim_paths.contains(module.filename()))
                .map(|module| module.filename().display().to_string())
                .collect::<Vec<_>>();
            paths.sort_unstable();
            let expected: BTreeSet<_> = THIEVER_C_PRIME_SHIMS.iter().copied().collect();
            let actual: BTreeSet<_> = paths.iter().map(String::as_str).collect();
            let added = actual.difference(&expected).copied().collect::<Vec<_>>();
            let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
            eprintln!(
                "C-prime to C-double-prime Thiever shim diff: added={added:?}, missing={missing:?}"
            );
            assert!(
                added.is_empty() && missing.is_empty(),
                "Thiever shim set differs from C-prime: added={added:?}, missing={missing:?}"
            );
            eprintln!("FROZEN_CATALOG Thiever shim modules: {paths:#?}");
            thiever_modules = Some(paths);
        }
        if allocation.bytes_total > max_bytes {
            max_bytes = allocation.bytes_total;
            max_name = name.clone();
        }
        const PER_START_LIMIT: u64 = 2 * 1024 * 1024;
        if allocation.bytes_total > PER_START_LIMIT {
            failures.push(format!(
                "{name}: per-start allocation {} exceeds {PER_START_LIMIT}",
                allocation.bytes_total
            ));
        }
        if !gated {
            failures.push(format!("{name}: no shim was gated"));
        }
    }

    assert!(
        thiever_modules.is_some(),
        "pinned Thiever card was not measured"
    );
    assert!(
        jive_dragons_measured,
        "pinned JiveDragons card was not measured"
    );
    eprintln!("FROZEN_CATALOG maximum per_start_alloc={max_bytes} card={max_name} limit=2MiB");
    eprintln!("FROZEN_CATALOG ungated cards: {ungated:?}");
    assert!(
        failures.is_empty(),
        "frozen catalog import-closure failures:\n{}",
        failures.join("\n")
    );
}

fn assert_module_loads(source: &str, label: &str) {
    let isolate = script_js::LoadIsolate::spawn(
        source.to_owned(),
        script_js::LoadShape::CompatClass,
        Vec::new(),
    )
    .unwrap_or_else(|error| panic!("{label}: spawn failed: {error}"));
    let probe = isolate.probe("true");
    let ready = isolate.poll_ready();
    isolate.join();
    assert!(
        probe.is_ok(),
        "{label}: setup probe failed: {probe:?}; loader state {ready:?}"
    );
    assert_eq!(ready, script_js::Ready::Ready, "{label}: loader state");
}

#[test]
fn review_regex_literal_mutants_keep_their_imports_loadable() {
    let cases = [
        (
            "regex backtick static",
            "const clean = (s) => s.replace(/`/g, '');\nimport * as regions from '../../api/map/regions.js';\n// one backtick closes the lexer template here: `\nglobalThis.__r = Object.keys(regions).length;\nexport default class P extends LoopingBot { loop() {} }",
        ),
        (
            "regex backtick dynamic",
            "const clean = (s) => s.replace(/`/g, '');\nconst mod = await import('../../api/map/regions.js');\n// one backtick closes the lexer template here: `\nexport default class P extends LoopingBot { loop() {} }",
        ),
        (
            "regex comment dynamic",
            "const split = (s) => s.split(/[/*]/);\nconst mod = await import('../../api/map/regions.js');\n/** a doc comment end closes the lexer comment */\nexport default class P extends LoopingBot { loop() {} }",
        ),
        (
            "regex backslash control",
            "const clean = (s) => s.replace(/\\s+/g, ' ');\nconst mod = await import('../../api/map/regions.js');\nexport default class P extends LoopingBot { loop() {} }",
        ),
    ];
    for (label, source) in cases {
        assert_module_loads(source, label);
    }
}

#[test]
fn missing_shim_import_error_names_module_path() {
    const MISSING_PATH: &str = "/rs2b0t/bot/api/map/__cf029_missing_module.js";
    let source = r#"
import '../../api/map/__cf029_missing_module.js';
export default class MissingShimModule extends LoopingBot {}
"#;
    let isolate = script_js::LoadIsolate::spawn(
        source.to_owned(),
        script_js::LoadShape::CompatClass,
        Vec::new(),
    )
    .expect("spawn missing-module fixture");

    let _ = isolate.probe("true");
    let error = match isolate.poll_ready() {
        script_js::Ready::Failed(error) => error,
        other => panic!("missing shim import unexpectedly loaded: {other:?}"),
    };
    assert!(
        error.contains("__cf029_missing_module.js"),
        "missing shim import error must name {MISSING_PATH}: {error}"
    );
    isolate.join();
}
