//! Frozen upstream tier-1 tests, executed as modules in the production Load isolate.
//! Only `bun:test` is remapped to the local assertion/registration shim; upstream
//! test sources remain byte-identical under `tests/upstream/2de6bb23`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use script::isolate_fb::{encode_snapshot_with_native, ItemRowInput, NativeFactsInput};
use script::{transpile_ts, LoadIsolate, LoadShape, Ready};
use serde_json::json;

mod common;

const FROZEN_TESTS: &str = "upstream/2de6bb23";
const TEST_MODULE_PREFIX: &str = "/rs2b0t/upstream/2de6bb23/test";
const TEST_HARNESS_URL: &str = "/rs2b0t/bot/__upstream/bun_test.js";
const EXPECTED_TEST_FILES: usize = 50;
const SETUP_DEADLINE: Duration = Duration::from_secs(20);
const RUN_DEADLINE: Duration = Duration::from_secs(10);

fn collect_test_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut tests = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).expect("read upstream test directory") {
            let path = entry.expect("read upstream test entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".test.ts"))
            {
                tests.push(path);
            }
        }
    }
    tests.sort();
    tests
}

fn test_url(file: &str) -> String {
    let file = file.strip_suffix(".ts").unwrap_or(file);
    format!("{TEST_MODULE_PREFIX}/{file}")
}

fn bot_source(test_url: &str) -> String {
    format!(
        r#"
import {{ runTests }} from '{TEST_HARNESS_URL}';
import '{test_url}';
export default class UpstreamTier1 extends LoopingBot {{
    async loop() {{
        if (globalThis.__upstream_started) return;
        globalThis.__upstream_started = true;
        try {{ globalThis.__upstream_results = await runTests(); }}
        catch (error) {{ globalThis.__upstream_runner_error = String(error?.stack ?? error); }}
        globalThis.__upstream_done = true;
    }}
}}
"#
    )
}
fn catalog_bank_facts() -> Arc<api::named_banks::NamedBankFacts> {
    let banks = api::named_banks::BANK_CATALOG
        .iter()
        .map(|definition| api::named_banks::NamedBank {
            name: definition.name,
            tile: definition.tile,
            definition: Some(definition),
            routable: true,
        })
        .collect();
    Arc::new(api::named_banks::NamedBankFacts::from_banks(banks))
}

fn wait_ready(isolate: &LoadIsolate) -> Result<(), String> {
    let deadline = Instant::now() + SETUP_DEADLINE;
    loop {
        match isolate.poll_ready() {
            Ready::Ready => return Ok(()),
            Ready::Failed(error) => return Err(error),
            Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ready::Pending => return Err("isolate setup timed out".into()),
        }
    }
}

fn run_upstream_file(
    root: &Path,
    path: &Path,
    harness: &str,
    game_data: Arc<api::game_data::SelectedGameData>,
    bank_facts: Arc<api::named_banks::NamedBankFacts>,
) -> serde_json::Value {
    let relative = path.strip_prefix(root).expect("test is below frozen root");
    let file = relative.to_string_lossy().replace('\\', "/");
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            return serde_json::json!({ "file": file, "stage": "read", "error": error.to_string() })
        }
    };
    run_upstream_source(&file, &source, harness, game_data, bank_facts)
}

fn run_upstream_source(
    file: &str,
    source: &str,
    harness: &str,
    game_data: Arc<api::game_data::SelectedGameData>,
    bank_facts: Arc<api::named_banks::NamedBankFacts>,
) -> serde_json::Value {
    let js = match transpile_ts(source) {
        Ok(js) => js,
        Err(error) => {
            return serde_json::json!({ "file": file, "stage": "transpile", "error": error })
        }
    };
    let harness_import = format!("'{TEST_HARNESS_URL}'");
    let adapted = js
        .replace("'bun:test'", &harness_import)
        .replace("\"bun:test\"", &harness_import);
    let url = test_url(file);
    let siblings = vec![
        (TEST_HARNESS_URL.to_string(), harness.to_string()),
        (url.clone(), adapted),
    ];
    let isolate = match LoadIsolate::spawn_with_content(
        bot_source(&url),
        LoadShape::CompatClass,
        siblings,
        Some(game_data),
        bank_facts,
    ) {
        Ok(isolate) => isolate,
        Err(error) => {
            return serde_json::json!({ "file": file, "stage": "spawn", "error": error.to_string() })
        }
    };
    if let Err(error) = wait_ready(&isolate) {
        let logs = isolate.join();
        return serde_json::json!({ "file": file, "stage": "load", "error": error, "logs": logs });
    }

    common::post_snapshot_input(&isolate, &common::ingame_snapshot());
    isolate.on_game_tick(1);
    let deadline = Instant::now() + RUN_DEADLINE;
    loop {
        match isolate.probe("globalThis.__upstream_done === true") {
            Ok(serde_json::Value::Bool(true)) => break,
            Ok(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            Ok(_) => {
                let logs = isolate.join();
                return serde_json::json!({ "file": file, "stage": "timeout", "error": "test run exceeded fixed deadline", "logs": logs });
            }
            Err(error) => {
                let logs = isolate.join();
                return serde_json::json!({ "file": file, "stage": "run", "error": error, "logs": logs });
            }
        }
    }

    let result = isolate.probe("globalThis.__upstream_results");
    let runner_error = isolate.probe("globalThis.__upstream_runner_error").ok();
    let logs = isolate.join();
    match result {
        Ok(serde_json::Value::Array(cases)) => {
            let pass = cases.iter().filter(|case| case["status"] == "pass").count();
            let fail = cases.iter().filter(|case| case["status"] == "fail").count();
            serde_json::json!({
                "file": file,
                "stage": if cases.is_empty() { "empty" } else { "ran" },
                "pass": pass,
                "fail": fail,
                "cases": cases,
                "runner_error": runner_error.unwrap_or(serde_json::Value::Null),
                "logs": logs,
            })
        }
        Ok(value) => {
            serde_json::json!({ "file": file, "stage": "result", "error": "runner returned no case array", "value": value, "logs": logs })
        }
        Err(error) => {
            serde_json::json!({ "file": file, "stage": "result", "error": error, "logs": logs })
        }
    }
}

fn outcome_manifest_entry(result: &serde_json::Value) -> serde_json::Value {
    let file = result["file"].as_str().expect("result file name");
    let stage = result["stage"].as_str().expect("result stage");
    if stage == "ran" {
        let cases = result["cases"]
            .as_array()
            .expect("ran result case array")
            .iter()
            .map(|case| {
                let name = case["name"].as_str().expect("case name");
                let status = case["status"].as_str().expect("case status");
                if status == "pass" {
                    serde_json::json!({"name": name, "status": status})
                } else {
                    let reason = case["error"]
                        .as_str()
                        .expect("failing case reason")
                        .lines()
                        .next()
                        .unwrap_or_default();
                    serde_json::json!({"name": name, "status": status, "reason": reason})
                }
            })
            .collect::<Vec<_>>();
        serde_json::json!({"file": file, "stage": stage, "cases": cases})
    } else {
        let reason = if stage == "excluded" {
            result["reason"].as_str().expect("excluded file reason")
        } else {
            result["error"]
                .as_str()
                .map(|error| error.lines().next().unwrap_or_default())
                .unwrap_or("no test cases registered")
        };
        serde_json::json!({"file": file, "stage": stage, "reason": reason})
    }
}

#[test]
fn upstream_harness_fails_cases_when_nested_before_each_throws() {
    let harness = include_str!("upstream/harness/bun_test.js");
    let game_data = api::game_data::for_revision(client::io::ClientRevision::R289)
        .expect("R289 selected game data");
    let source = r#"
import { beforeEach, describe, test } from 'bun:test';
describe('outer', () => {
    beforeEach(() => { throw new Error('nested setup refused'); });
    test('inner case', () => {});
});
"#;
    let result = run_upstream_source(
        "harness/hook-probe.test.ts",
        source,
        harness,
        game_data,
        catalog_bank_facts(),
    );
    assert_eq!(result["stage"], "ran", "{result}");
    let cases = result["cases"].as_array().expect("registered test cases");
    assert_eq!(cases.len(), 1, "{result}");
    assert_eq!(cases[0]["status"], "fail", "{result}");
    assert!(
        cases[0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("nested setup refused")),
        "nested hook failure was not reported: {result}"
    );
}

#[test]
fn upstream_tier1_runs_unmodified_sources_in_load() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(FROZEN_TESTS);
    let tests = collect_test_files(&root);
    assert_eq!(
        tests.len(),
        EXPECTED_TEST_FILES,
        "tier-1 source inventory changed"
    );
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("upstream/expected-2de6bb23.json"))
            .expect("parse pinned tier-1 results");
    assert_eq!(
        expected["revision"], "2de6bb2397ceb9ffd05f2354f67dde328e324531",
        "tier-1 manifest revision changed"
    );
    let expected_files = expected["files"]
        .as_array()
        .expect("tier-1 manifest file list");
    assert_eq!(
        expected_files.len(),
        EXPECTED_TEST_FILES,
        "tier-1 manifest inventory changed"
    );
    let source_files: Vec<String> = tests
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .expect("test is below frozen root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let manifest_files: Vec<String> = expected_files
        .iter()
        .map(|entry| {
            entry["file"]
                .as_str()
                .expect("manifest file name")
                .to_owned()
        })
        .collect();
    assert_eq!(
        source_files, manifest_files,
        "tier-1 source inventory differs from its expected-results manifest"
    );
    for entry in expected_files {
        if entry["stage"] == "excluded" {
            assert!(
                entry["reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.trim().is_empty()),
                "excluded test needs a documented reason: {entry}"
            );
        }
    }
    let harness = include_str!("upstream/harness/bun_test.js");
    let game_data = api::game_data::for_revision(client::io::ClientRevision::R289)
        .expect("R289 selected game data");
    let bank_facts = catalog_bank_facts();
    let results: Vec<_> = tests
        .iter()
        .zip(expected_files)
        .map(|(path, expected_file)| {
            if expected_file["stage"] == "excluded" {
                serde_json::json!({
                    "file": expected_file["file"],
                    "stage": "excluded",
                    "reason": expected_file["reason"],
                })
            } else {
                run_upstream_file(
                    &root,
                    path,
                    harness,
                    Arc::clone(&game_data),
                    Arc::clone(&bank_facts),
                )
            }
        })
        .collect();

    if let Some(evidence_dir) = std::env::var_os("COMPAT_REFREEZE_EVIDENCE_DIR") {
        let evidence_dir = PathBuf::from(evidence_dir);
        std::fs::create_dir_all(&evidence_dir).expect("create evidence directory");
        let report = serde_json::json!({ "revision": "2de6bb2397ceb9ffd05f2354f67dde328e324531", "file_count": results.len(), "results": results });
        std::fs::write(
            evidence_dir.join("upstream-tier1.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .expect("write upstream results");
    }

    for result in &results {
        eprintln!(
            "{} stage={} pass={} fail={}",
            result["file"],
            result["stage"],
            result["pass"].as_u64().unwrap_or(0),
            result["fail"].as_u64().unwrap_or(0),
        );
    }
    assert_eq!(results.len(), EXPECTED_TEST_FILES);
    let actual: Vec<_> = results.iter().map(outcome_manifest_entry).collect();
    assert_eq!(
        actual.as_slice(),
        expected_files.as_slice(),
        "upstream tier-1 results changed; review and update the pinned manifest"
    );
}

#[test]
fn refrozen_runtime_members_load_and_refuse_by_name() {
    let ts = r#"
import { InvItem as ModuleInvItem } from '#/bot/api/inventory/Inventory.js';
import { isEdibleFood as foodIsEdible } from '#/bot/api/combat/food.js';
import { lockedBankAt } from '#/bot/api/bank/BankLocations.js';
import { SUPER_DEFENCE, SUPER_SET, RANGING_POTION, rangingPlan } from '#/bot/api/combat/boostPotions.js';
import { ENCLAVE_TARGET_OPTIONS as options, GUTANOTH_BLUE, enclaveTargets as selectEnclave } from '#/bot/api/combat/hunting/sites.js';
import { chasesTarget, Fight } from '#/bot/api/combat/hunting/combat.js';
import { ClueDuelSearch } from '#/bot/api/duel/ClueDuel.js';
import { resetClueDuelSearch } from '#/bot/api/ai/clues/duelTravel.js';
import {
    toolAttackLevel as rootToolAttackLevel, toolsNeedingEquip as rootToolsNeedingEquip,
    shouldBankNow as rootShouldBankNow, bankDistance as rootBankDistance,
    bestHeldToolNames as rootBestHeldToolNames, surplusHeldToolNames as rootSurplusHeldToolNames,
    resolveBankOpenRoute as rootResolveBankOpenRoute, NEARBY_BANK_RADIUS as rootNearbyBankRadius,
    NAV_PURE_WALK as rootNavPureWalk, Inventory as RootInventory, InvItem as RootInvItem,
    Equipment as RootEquipment,
} from '@rs2b0t/api';
import * as rootApi from '@rs2b0t/api';
import { toolAttackLevel, toolsNeedingEquip } from '#/bot/api/acquisition/Tools.js';
import { shouldBankNow, parseBankStrategy, matchesCommonBankLoot, RANDOM_EVENT_CASKET_ID } from '#/bot/api/bank/Banking.js';
import type { EventMap, Task, InvItem as InvItemType } from '@rs2b0t/api';
import type { BankOpts, CombatHost } from '#/bot/api/combat/hunting/supply.js';
import { RETALIATE_VARP, PLAYER_FACE_BASE, retaliateOnFromVarp, facingPlayer } from '#/bot/api/game/Game.js';
import { reader } from '#/bot/adapter/ClientAdapter.js';
import { backpackSnapshots, backpackCapacity } from '#/bot/api/inventory/Inventory.js';
import { Inventory as ModuleInventory } from '#/bot/api/inventory/Inventory.js';
import { Equipment as ModuleEquipment } from '#/bot/api/equipment/Equipment.js';
import { bestHeldToolNames, surplusHeldToolNames } from '#/bot/api/acquisition/Tools.js';
import { resolveBankOpenRoute, NEARBY_BANK_RADIUS } from '#/bot/api/bank/Banking.js';
import { RANDOM_EVENT_CASKET_ID as bankRulesCasket } from '#/bot/api/bank/bankRules.js';
import { parseTradePartnerHeader } from '#/bot/api/trade/Trade.js';
import { tradeScreenState, stableClosedPoll } from '#/bot/api/trade/drivePartnerTrade.js';
import { shopOpBatch, SELL_STACK_STEP } from '#/bot/api/shop/Shop.js';
import { resolveCombatStyle, parseInterfaceCombatStyle, CombatStyleController } from '#/bot/api/combat/CombatStyle.js';
import { QUEST_ROCK_TYPES, GAS_ROCK_TICKS, BROKEN_PICKAXE } from '#/bot/data/miningRocks.js';
import { AUTO_CHOOSE_COM, AUTO_TOGGLE_COM } from '#/bot/api/combat/CombatStyleLogic.js';
import { wrapText } from '#/bot/paint/paintLogic.js';
import { parseBurnMode, logsForTree, firemakingLevelForLogs, resolveFireSpot, nearestFireSpot, expandLocalFirePlot, runWest, shouldBurnFullLoad, BURN_MODE_OPTIONS } from '#/bot/api/firemaking/Firemaking.js';
import { nearestWalkableBank, nearestWalkableBankAsync, bankCostForFinder, bankCostForNavigator, bankDistance } from '#/bot/api/bank/BankLocations.js';
import { NAV_PURE_WALK, NAV_WITH_TELES } from '#/bot/api/walking/Traversal.js';
import { worthStocking, buildCatalog, searchCatalog, resolveByName, resetLiveCatalog } from '#/bot/api/market/catalog.js';
import { COIN_FLOAT, PROVISION_BANK } from '#/bot/api/ai/quests/engine/QuestEngine.js';
import { suspectOrder, ARHEIN, GUARD_START, GUARD_HANDIN, SALESMAN, SUSPECTS, MURDER_EVIDENCE, MURDER_OBJ } from '#/bot/api/ai/quests/defs/murder/areas.js';
import { tilesTo, trailWalkOpts, TRACE_STORAGE_KEY } from '#/bot/api/ai/clues/ClueExecutor.js';
import { trailKit, nextCoordTool, extraItemShop, gateItemShop, SPADE_SPAWNS, PROFESSOR, MURPHY, KOJO, KOJO_EXIT } from '#/bot/api/ai/clues/data/toolAcquire.js';
import { pickByLine, isUnderground, needsHop } from '#/bot/api/ai/quests/exec/primitives.js';
import { resolveLoopCadence, BranchTask, LeafTask } from '#/bot/api/bot/Bot.js';
import * as clientAdapterExports from '#/bot/adapter/ClientAdapter.js';
import * as woodcuttingExports from '#/bot/data/woodcuttingLocations.js';
import * as settingsExports from '#/bot/runtime/Settings.js';
import * as runManagerExports from '#/bot/runtime/RunManager.js';
import * as scriptRunnerExports from '#/bot/runtime/ScriptRunner.js';
import * as botHostExports from '#/bot/runtime/BotHost.js';
import * as paintExports from '#/bot/paint/Paint.js';
import * as paintLogicExports from '#/bot/paint/paintLogic.js';
import * as gameMessagesExports from '#/bot/api/chatbox/gameMessages.js';


const check = (ok, message) => { if (!ok) throw new Error(message); };
async function refuses(member, call) {
    let error;
    try { await call(); } catch (caught) { error = caught; }
    const message = String(error?.message ?? error);
    check(error && message.includes('not impl:') && message.includes(member), `missing named refusal for ${member}: ${message}`);
}
check(
    GAS_ROCK_TICKS === 60 &&
        BROKEN_PICKAXE === 'Broken pickaxe' &&
        QUEST_ROCK_TYPES.Blurite?.join(',') === '2110' &&
        QUEST_ROCK_TYPES.Limestone?.join(',') === '4027,4028,4029',
    'mining rock constants mismatch'
);
check(ModuleInvItem === RootInvItem, 'InvItem root re-export differs from module');
check(new ModuleInvItem({ id: 1, name: 'Shark', count: 1, slot: 0, noted: true }).noted === true, 'InvItem.noted missing');
check(foodIsEdible({ name: 'Shark', noted: false }, 'Shark') === true, 'isEdibleFood false for unnoted food');
globalThis.__inventoryBackpackProbe = () => {
    const moduleRows = backpackSnapshots();
    return {
        rootHelpersAbsent: !('backpackSnapshots' in rootApi) && !('backpackCapacity' in rootApi),
        rootValues: RootInventory === ModuleInventory && RootInvItem === ModuleInvItem &&
            RootEquipment === ModuleEquipment,
        moduleName: moduleRows[0]?.name,
        moduleCapacity: backpackCapacity(),
    };
};
check(foodIsEdible({ name: 'Shark', noted: true }, 'Shark') === false, 'isEdibleFood accepted a noted item');
const locked = lockedBankAt({ x: 3185, z: 3440, level: 0 });
check(locked === null || typeof locked === 'object', 'lockedBankAt result shape');
check(bankDistance({ x: 0, z: 0, level: 0 }, { x: 3, z: 4, level: 1 }) === 5 && rootBankDistance({ x: 0, z: 0, level: 0 }, { x: 3, z: 4, level: 1 }) === 5, 'bankDistance root/module export is not Euclidean in x/z');
check(wrapText('alpha beta', 5, 1).join('|') === 'alpha| beta', 'wrapText wrapping or continuation indent mismatch');
const resolvedStyle = resolveCombatStyle('attack', [{ mode: 0, label: '(Accurate)' }]);
check(resolvedStyle?.requested === 'attack' && resolvedStyle.effective === 'attack' && resolvedStyle.mode === 0, 'resolveCombatStyle result mismatch');
const styleOptions = ['Cur.', 'Ar.', 'Bidith.', 'Tanath.', 'Gor.'];
check(pickByLine(['@dbl@Cur|tanath...'], styleOptions, [
    { whenLine: 'ar cur', choose: 'Gor.' },
    { whenLine: 'cur tanath', choose: 'Bidith.' },
]) === 'Bidith.', 'pickByLine normalization or longest-match behavior mismatch');
check(isUnderground({ z: 5000 }) === true && needsHop({ z: 3218 }, { z: 9572 }) === true, 'underground helper mismatch');
check(trailKit(null).length === 0 && trailKit(2811).join(',') === 'Spade,Sextant,Watch,Chart,Rope' && trailKit(2853).join(',') === 'Spade,Sextant,Watch,Chart', 'trailKit kit contents mismatch');
check(trailKit.length === 1, 'trailKit parameter count mismatch');
check(nextCoordTool({ sextant: false, watch: false, chart: false }) === 'sextant', 'nextCoordTool order mismatch');
check(resolveCombatStyle.length === 2 && pickByLine.length === 3 && isUnderground.length === 1 && needsHop.length === 2 && nextCoordTool.length === 1, 'pure helper parameter counts mismatch');
check(SUPER_DEFENCE === SUPER_SET[2], 'super potion aliases lost descriptor identity');
check(options.join(',') === 'both,dragons,demons', 'enclave options mismatch');
const source = { key: 'gutanoth-blue', target: 'old', bones: 'old', alsoHunt: ['old'], extra: 7 };
const demons = selectEnclave(source, 'demons');
check(demons !== source && source.target === 'old', 'enclaveTargets mutated or reused its source');
check(demons.target === 'Greater demon' && demons.bones === 'Ashes' && demons.alsoHunt === undefined && demons.extra === 7, 'demons target mapping mismatch');
const both = selectEnclave(source);
check(both.target === 'Blue dragon' && both.bones === 'Dragon bones' && both.alsoHunt[0] === 'Greater demon', 'default enclave target mapping mismatch');
const other = { key: 'taverley-black', target: 'Black dragon' };
check(selectEnclave(other) === other, 'non-enclave site identity changed');
check(chasesTarget(GUTANOTH_BLUE, 'melee') === true && chasesTarget(GUTANOTH_BLUE, 'range') === false, 'chasesTarget mapping mismatch');
check(toolAttackLevel('Rune pickaxe') === 40 && rootToolAttackLevel('Rune pickaxe') === 40, 'toolAttackLevel root/module export mismatch');
check(Array.isArray(toolsNeedingEquip([], () => 1, () => 0, () => false)) && Array.isArray(rootToolsNeedingEquip([], () => 1, () => 0, () => false)), 'toolsNeedingEquip root/module export mismatch');
const bankState = { lootCount: 15, minutesSinceLastBank: 0, itemsThreshold: 15, minutesThreshold: 10 };
check(shouldBankNow('items', bankState) === true && rootShouldBankNow('items', bankState) === true, 'shouldBankNow root/module export mismatch');
check(parseBankStrategy('loot') === 'items' && parseBankStrategy('Loot count') === 'items', 'parseBankStrategy loot alias mismatch');

check(retaliateOnFromVarp(0) === true && retaliateOnFromVarp(1) === false, 'retaliate varp interpretation mismatch');
check(facingPlayer(PLAYER_FACE_BASE) === true && facingPlayer(PLAYER_FACE_BASE - 1) === false, 'player face boundary mismatch');
check(RETALIATE_VARP > 0, 'RETALIATE_VARP export missing');
check(parseInterfaceCombatStyle('(Aggressive)') === 'strength' && parseInterfaceCombatStyle('Defensive') === 'defence' && parseInterfaceCombatStyle('Rapid') === null, 'combat interface style mapping mismatch');
const contentCasket = globalThis.__rs2b0t_selected_facts('random-event-casket');
check(
    RANDOM_EVENT_CASKET_ID === contentCasket &&
        bankRulesCasket === contentCasket &&
        matchesCommonBankLoot('', contentCasket) === true,
    'random-event casket id is not selected from game content'
);
const objCatalog = reader.objCatalog();
const selectedCatalog = globalThis.__rs2b0t_selected_facts('obj-catalog');
const selectedIds = [...selectedCatalog.byId.keys()].sort((a, b) => a - b);
check(
    Array.isArray(objCatalog) &&
        objCatalog.length === selectedIds.length &&
        objCatalog.every((item, index) =>
            item.id === selectedIds[index] &&
            typeof item.name === 'string' &&
            typeof item.cost === 'number' &&
            typeof item.stackable === 'boolean' &&
            typeof item.members === 'boolean' &&
            typeof item.equippable === 'boolean' &&
            typeof item.certlink === 'number' &&
            typeof item.certtemplate === 'number' &&
            typeof item.stackVariant === 'boolean'),
    'reader.objCatalog did not return every named selected object in ID order'
);
check(objCatalog.find((item) => item.id === 537)?.stackable === true, 'noted dragon bones are missing or not stackable');
check(objCatalog.some((item) => item.id === 1752), 'noted blue dragonhide is missing');
check(
    [883, 885, 887, 889, 891, 893].every((id) => objCatalog.some((item) => item.id === id)),
    'poisoned arrow records are missing'
);
check(objCatalog.some((item) => item.id === __UNTRADEABLE_OBJ_ID__), 'named untradeable object is missing');
check(
    [883, 884, 885, 886, 887, 888, 889, 890, 891, 892, 893].every(
        (id) => objCatalog.find((item) => item.id === id)?.equippable === false,
    ),
    'objCatalog equippable must match the missing worn models for arrows 883–893',
);
const wornModelAbsentIds = [
    78, 598, 773, 877, 878, 879, 880, 881, 882, 883, 884, 885, 886, 887,
    888, 889, 890, 891, 892, 893, 942, 1635, 1637, 1639, 1641, 1643, 1645,
    2532, 2533, 2534, 2535, 2536, 2537, 2538, 2539, 2540, 2541, 2550, 2552,
    2554, 2556, 2558, 2560, 2562, 2564, 2566, 2568, 2570, 2572, 2866,
    4023, 4024, 4025, 4026, 4027, 4028, 4029, 4030, 4031, 4032,
];
check(
    wornModelAbsentIds.every((id) => objCatalog.find((item) => item.id === id)?.equippable === false),
    'objCatalog marks an object without a worn model as equippable',
);
const wornModelPresentIds = [
    229, 301, 303, 305, 307, 309, 311, 583, 585, 590, 717, 718, 719, 727,
    731, 946, 952, 970, 972, 973, 1511, 1601, 1603, 1605, 1607, 1609, 1611,
    1613, 1615, 1733, 1735, 1741, 1785, 1925, 1963, 2347, 2520, 2522, 2524,
    2526, 2946, 2949, 2968, 2969, 3159, 3164, 3695, 3711, 3801, 3803,
];
check(
    wornModelPresentIds.every((id) => objCatalog.find((item) => item.id === id)?.equippable === true),
    'objCatalog omits a worn model from equippable',
);
check(objCatalog.find((item) => item.id === 1321)?.equippable === true, 'bronze scimitar worn model is missing');
check(reader.objCatalog() === objCatalog, 'reader.objCatalog rebuilt its selected records');
await refuses('GameMessages.record', () => gameMessagesExports.GameMessages.record('line'));
await refuses('GameMessages.firstSince', () => gameMessagesExports.GameMessages.firstSince(0, /line/));
await refuses('GameMessages.recent', () => gameMessagesExports.GameMessages.recent());
await refuses('GameMessages.reset', () => gameMessagesExports.GameMessages.reset());
const rangingPlanValue = rangingPlan([{ item: 'Ranging potion(2)', qty: 3 }]);
check(rangingPlanValue.potion === RANGING_POTION && rangingPlanValue.flask === 'Ranging potion(2)' && rangingPlanValue.want === 3, 'rangingPlan does not use the exported ranging descriptor');
check(typeof AUTO_CHOOSE_COM === 'number' && typeof AUTO_TOGGLE_COM === 'number' && typeof SELL_STACK_STEP === 'number' && typeof NEARBY_BANK_RADIUS === 'number', 'combat, shop, or bank constant export missing');
check(
    AUTO_CHOOSE_COM === globalThis.__rs2b0t_selected_facts('autocast-component', 'combat_staff_2:auto_choose') &&
        AUTO_TOGGLE_COM === globalThis.__rs2b0t_selected_facts('autocast-component', 'combat_staff_2:auto_toggle'),
    'combat style component ids must come from selected interface aliases'
);
check(
    BURN_MODE_OPTIONS.join(',') === 'Off,Chop then burn' &&
        typeof NAV_PURE_WALK === 'object' &&
        typeof NAV_WITH_TELES === 'object' &&
        rootNavPureWalk === NAV_PURE_WALK,
    'firemaking or traversal constant export mismatch'
);
await refuses('COIN_FLOAT', () => Number(COIN_FLOAT));
await refuses('PROVISION_BANK', () => PROVISION_BANK.x);
await refuses('ARHEIN', () => ARHEIN.npc);
await refuses('GUARD_START', () => GUARD_START.npc);
await refuses('GUARD_HANDIN', () => GUARD_HANDIN.npc);
await refuses('SALESMAN', () => SALESMAN.npc);
await refuses('SUSPECTS', () => SUSPECTS.length);
await refuses('suspectOrder', () => suspectOrder(MURDER_OBJ.THREAD_GREEN));
await refuses('MURDER_EVIDENCE', () => MURDER_EVIDENCE.length);
await refuses('SPADE_SPAWNS', () => SPADE_SPAWNS.length);
await refuses('PROFESSOR', () => PROFESSOR.npc);
await refuses('MURPHY', () => MURPHY.npc);
await refuses('KOJO', () => KOJO.npc);
await refuses('KOJO_EXIT', () => KOJO_EXIT.x);
await refuses('invalidateLocSnapshots', () => clientAdapterExports.invalidateLocSnapshots());
await refuses('detach', () => clientAdapterExports.detach());
await refuses('activeStatsReady', () => clientAdapterExports.activeStatsReady());
await refuses('currentLoginStatsReady', () => clientAdapterExports.currentLoginStatsReady());
await refuses('locHullHeight', () => clientAdapterExports.locHullHeight());
await refuses('resolveLocModelExtents', () => clientAdapterExports.resolveLocModelExtents());
await refuses('attach', () => clientAdapterExports.attach());
await refuses('setPacketListener', () => clientAdapterExports.setPacketListener());
await refuses('resetObjCatalog', () => clientAdapterExports.resetObjCatalog());
await refuses('readRetaliateControls', () => clientAdapterExports.readRetaliateControls());
await refuses('readModalButtons', () => clientAdapterExports.readModalButtons());
await refuses('readSelectButtonLabelsByVarp', () => clientAdapterExports.readSelectButtonLabelsByVarp());
await refuses('combatShowingThreshold', () => clientAdapterExports.combatShowingThreshold());
await refuses('WELCOME_SCREEN', () => Number(clientAdapterExports.WELCOME_SCREEN));
await refuses('resolveWoodcuttingLocation', () => woodcuttingExports.resolveWoodcuttingLocation());
await refuses('WOODCUTTING_LOCATIONS', () => Number(woodcuttingExports.WOODCUTTING_LOCATIONS));
await refuses('WOODCUTTING_LOCATION_OPTIONS', () => Number(woodcuttingExports.WOODCUTTING_LOCATION_OPTIONS));
await refuses('settingOptionLabel', () => settingsExports.settingOptionLabel());
await refuses('GLOBAL_SETTINGS_CORE', () => Number(settingsExports.GLOBAL_SETTINGS_CORE));
await refuses('NAV_SETTINGS', () => Number(settingsExports.NAV_SETTINGS));
await refuses('GLOBAL_SETTINGS', () => Number(settingsExports.GLOBAL_SETTINGS));
await refuses('MAP_PICKER_SETTINGS_NS', () => Number(settingsExports.MAP_PICKER_SETTINGS_NS));
await refuses('MAP_PICKER_SETTINGS', () => Number(settingsExports.MAP_PICKER_SETTINGS));
await refuses('LOADOUT_SETTINGS_NS', () => Number(settingsExports.LOADOUT_SETTINGS_NS));
await refuses('LOADOUT_SETTINGS', () => Number(settingsExports.LOADOUT_SETTINGS));
await refuses('PRICE_BOOK_SETTINGS_NS', () => Number(settingsExports.PRICE_BOOK_SETTINGS_NS));
await refuses('PRICE_BOOK_SETTINGS', () => Number(settingsExports.PRICE_BOOK_SETTINGS));
await refuses('resolveRunPolicy', () => runManagerExports.resolveRunPolicy());
await refuses('shouldEnableRun', () => runManagerExports.shouldEnableRun());
await refuses('loopReadyOrDetached', () => scriptRunnerExports.loopReadyOrDetached());
await refuses('stopReasonOf', () => scriptRunnerExports.stopReasonOf());
await refuses('BotHostImpl', () => new botHostExports.BotHostImpl());
await refuses('PaintFrame', () => new paintExports.PaintFrame());
await refuses('resolveDock', () => paintLogicExports.resolveDock());
await refuses('toCanvasPoint', () => paintLogicExports.toCanvasPoint());
await refuses('cycleOption', () => paintLogicExports.cycleOption());
await refuses('hitRegion', () => paintLogicExports.hitRegion());
await refuses('clipText', () => paintLogicExports.clipText());
await refuses('paintCols', () => paintLogicExports.paintCols(100, 8, 7));
await refuses('cellWidths', () => paintLogicExports.cellWidths(100, [1, 2]));
await refuses('gridRows', () => paintLogicExports.gridRows(5, 2));
await refuses('listScroll', () => paintLogicExports.listScroll(10, 4, { offset: 0, manual: false, focus: 0 }, 1, 0));
await refuses('paintClip', () => paintLogicExports.paintClip('some text'));
await refuses('gatherPaintAccent', () => paintLogicExports.gatherPaintAccent('fish'));
await refuses('stripSegments', () => paintLogicExports.stripSegments(100, [], 20, 8));
await refuses('railRows', () => paintLogicExports.railRows(100, 3));
await refuses('statColumns', () => paintLogicExports.statColumns(100, 10, 8, 2));
await refuses('PaintState.reset', () => paintLogicExports.PaintState.prototype.reset.call({}));
await refuses('PaintState', () => new paintLogicExports.PaintState());
await refuses('paintState', () => Number(paintLogicExports.paintState));
await refuses('WHEEL_ROWS', () => Number(paintLogicExports.WHEEL_ROWS));
check(typeof TRACE_STORAGE_KEY === 'string', 'clue trace key export missing');
check(typeof RANGING_POTION === 'object', 'ranging potion descriptor is not loadable');
check(typeof RETALIATE_VARP === 'number', 'RETALIATE_VARP export missing');
check(typeof resolveLoopCadence === 'function', 'resolveLoopCadence export missing');
check(typeof parseBurnMode === 'function' && typeof logsForTree === 'function' && typeof firemakingLevelForLogs === 'function' && typeof resolveFireSpot === 'function' && typeof nearestFireSpot === 'function' && typeof expandLocalFirePlot === 'function' && typeof runWest === 'function' && typeof shouldBurnFullLoad === 'function', 'firemaking helper exports missing');
check(
    parseBurnMode('  CHOP THEN BURN ') === 'chop-then-burn' && parseBurnMode('Off') === 'off',
    'firemaking burn mode mismatch'
);
check(
    logsForTree('  Dead tree ') === 'Logs' &&
        logsForTree('Oak') === 'Oak logs' &&
        logsForTree('Willow') === 'Willow logs' &&
        logsForTree('Maple') === 'Maple logs' &&
        logsForTree('Yew') === 'Yew logs' &&
        logsForTree('Magic tree') === 'Magic logs',
    'firemaking tree log mapping mismatch'
);
check(
    firemakingLevelForLogs('Logs') === 1 &&
        firemakingLevelForLogs('Yew logs') === 60 &&
        firemakingLevelForLogs('Unlisted logs') === undefined,
    'firemaking log levels mismatch'
);
const fireSpotResolved = resolveFireSpot(' varrock east ');
check(
    fireSpotResolved?.name === 'Varrock East' && fireSpotResolved.plot.bank.x === 3253,
    'fire spot resolution mismatch'
);
check(
    nearestFireSpot({ x: 3090, z: 3245, level: 0 })?.name === 'Draynor',
    'nearest fire spot mismatch'
);
const firePlotBase = { bank: { x: 50, z: 50, level: 0 }, x0: 46, x1: 54, z0: 46, z1: 54 };
const firePlotExpanded = expandLocalFirePlot(firePlotBase, 4, 24);
check(
    firePlotExpanded?.x0 === 42 &&
        firePlotExpanded.x1 === 58 &&
        firePlotExpanded.z0 === 42 &&
        firePlotExpanded.z1 === 58,
    'local fire plot expansion mismatch'
);
check(
    expandLocalFirePlot(
        { bank: { x: 50, z: 50, level: 0 }, x0: 42, x1: 58, z0: 42, z1: 58 },
        4,
        8
    ) === null,
    'local fire plot maximum mismatch'
);
const westFirePlot = { bank: { x: 0, z: 0, level: 0 }, x0: 3256, x1: 3260, z0: 3429, z1: 3429 };
check(
    runWest({ x: 3260, z: 3429, level: 0 }, westFirePlot, new Set(), () => true, () => true, 5) === 5 &&
        runWest({ x: 3260, z: 3429, level: 0 }, westFirePlot, new Set(['3258,3429']), () => true, () => true, 10) === 2,
    'west fire lane mismatch'
);
check(
    shouldBurnFullLoad('chop-then-burn', true, 28, true) &&
        !shouldBurnFullLoad('off', true, 28, true) &&
        !shouldBurnFullLoad('chop-then-burn', false, 28, true) &&
        !shouldBurnFullLoad('chop-then-burn', true, 0, true) &&
        !shouldBurnFullLoad('chop-then-burn', true, 28, false),
    'full-load burn gate mismatch'
);
check(typeof nearestWalkableBank === 'function' && typeof nearestWalkableBankAsync === 'function' && typeof bankCostForFinder === 'function' && typeof bankCostForNavigator === 'function' && typeof bankDistance === 'function', 'bank location helper exports missing');
check(typeof tilesTo === 'function' && typeof trailWalkOpts === 'function' && typeof trailKit === 'function' && typeof nextCoordTool === 'function' && typeof extraItemShop === 'function' && typeof gateItemShop === 'function', 'clue helper exports missing');
check(typeof worthStocking === 'function' && typeof buildCatalog === 'function' && typeof searchCatalog === 'function' && typeof resolveByName === 'function' && typeof resetLiveCatalog === 'function', 'catalog helper exports missing');
check(
    parseTradePartnerHeader(' Trading With: RunnerBot ') === 'RunnerBot' &&
        parseTradePartnerHeader('Trading with RunnerBot') === 'RunnerBot' &&
        parseTradePartnerHeader(' RunnerBot ') === 'RunnerBot' &&
        parseTradePartnerHeader('Trading With: ') === null,
    'trade partner header parse mismatch'
);
const heldToolReqs = [
    { kind: 'tiered', skill: 'woodcutting', tiers: [
        { name: 'Rune axe', level: 41 },
        { name: 'Steel axe', level: 6 },
        { name: 'Bronze axe', level: 1 },
    ] },
    { kind: 'exact', name: 'Tinderbox' },
];
const heldToolCount = name => ['Steel axe', 'Bronze axe', 'Tinderbox'].includes(name) ? 1 : 0;
const heldToolLevel = skill => skill === 'woodcutting' ? 41 : 1;
check(
    JSON.stringify(bestHeldToolNames(heldToolReqs, heldToolLevel, heldToolCount)) === '["Steel axe","Tinderbox"]' &&
        JSON.stringify(surplusHeldToolNames(heldToolReqs, heldToolLevel, heldToolCount)) === '["Bronze axe"]',
    'held tool selector mismatch'
);
check(
    rootBestHeldToolNames === bestHeldToolNames &&
        rootSurplusHeldToolNames === surplusHeldToolNames,
    'root tool helper exports mismatch'
);
const routeHere = { x: 100, z: 100, level: 0 };
check(
    resolveBankOpenRoute({
        bankOpen: true, here: null, stand: null, nearbyBoothDist: 0, nearest: null,
    }) === 'already-open' &&
        resolveBankOpenRoute({
            bankOpen: false, here: routeHere, stand: { x: 200, z: 200, level: 0 },
            nearbyBoothDist: NEARBY_BANK_RADIUS, nearest: null,
        }) === 'scene-booth' &&
        resolveBankOpenRoute({
            bankOpen: false, here: routeHere, stand: null, nearbyBoothDist: null,
            nearest: { tile: { x: 104, z: 100, level: 0 } },
        }) === 'local-bank' &&
        resolveBankOpenRoute({
            bankOpen: false, here: routeHere, stand: { x: 104, z: 100, level: 0 },
            nearbyBoothDist: null, nearest: { tile: { x: 104, z: 100, level: 0 } },
            preferNearby: false,
        }) === 'preset-stand' &&
        resolveBankOpenRoute({
            bankOpen: false, here: routeHere, stand: null, nearbyBoothDist: null, nearest: null,
        }) === 'nearest-fallback',
    'bank open route mismatch'
);
check(
    rootResolveBankOpenRoute === resolveBankOpenRoute &&
        rootNearbyBankRadius === NEARBY_BANK_RADIUS &&
        rootResolveBankOpenRoute({
            bankOpen: false, here: routeHere, stand: null, nearbyBoothDist: null, nearest: null,
        }) === 'nearest-fallback',
    'root bank helper exports mismatch'
);
const tradeState = globalThis.__rs2b0t_partner_trade;
check(
    tradeState('tradeScreenState', { trade_offer_open: false, trade_confirm_open: false }) === 'closed' &&
        tradeState('tradeScreenState', { trade_offer_open: false, trade_confirm_open: true }) === 'confirm' &&
        tradeState('tradeScreenState', { trade_offer_open: true, trade_confirm_open: false }) === 'offer' &&
        tradeState('tradeScreenState', { trade_offer_open: true, trade_confirm_open: true }) === 'offer',
    'trade screen state precedence mismatch'
);
const buyOps = [null, 'Value', 'Buy 1', 'Buy 5', 'Buy 10'];
check(
    SELL_STACK_STEP === 10 &&
        JSON.stringify(shopOpBatch(buyOps, 'buy', 25)) === '[4,4,3]' &&
        JSON.stringify(shopOpBatch(buyOps, 'buy', 100)) === '[4,4,4,4,4]' &&
        JSON.stringify(shopOpBatch(buyOps, 'buy', 3)) === '[2,2,2]',
    'shop op batch mismatch'
);
check(
    !worthStocking('Bronze arrow(p)') &&
        worthStocking('Dragon dagger(p)') &&
        worthStocking('Coins'),
    'worthStocking mismatch'
);
let worldTileReads = 0;
const tilesToNative = globalThis.__rs2b0t_clue_tiles_to;
check(
    tilesTo(null, () => { worldTileReads++; return { x: 0, z: 0, level: 0 }; }) === null &&
        worldTileReads === 0 &&
        tilesToNative({ x: 1, z: 2, level: 3 }, () => ({ x: 4, z: 7, level: 0 })) === 5 &&
        tilesTo(null) === null,
    'tilesTo mismatch'
);

const taskBot = new TaskBot();
let activeName;
taskBot.add({ label: 'sample task', validate: () => true, execute: () => { activeName = taskBot.activeTaskName; } });
await taskBot.loop();
check(activeName === 'sample task' && taskBot.activeTaskName === null, 'TaskBot.activeTaskName lifecycle mismatch');
let currentCombatMode = 0;
const combatStyleController = new CombatStyleController({
    offeredModes: () => [
        { mode: 0, label: 'Accurate' },
        { mode: 1, label: 'Aggressive' },
        { mode: 2, label: 'Controlled' },
        { mode: 3, label: 'Defensive' }
    ],
    currentMode: () => currentCombatMode,
    selectMode: mode => { currentCombatMode = mode; return true; }
});
const strengthResolution = combatStyleController.resolution('strength');
check(
    strengthResolution?.requested === 'strength' &&
        strengthResolution.effective === 'strength' &&
        strengthResolution.mode === 1,
    'CombatStyleController.resolution mismatch'
);
check(combatStyleController.mode('defence') === 3, 'CombatStyleController.mode mismatch');
check(combatStyleController.has('attack'), 'CombatStyleController.has mismatch');
check(!combatStyleController.has('strength'), 'CombatStyleController.has false mismatch');
check(
    combatStyleController.set('strength') &&
        currentCombatMode === 1 &&
        combatStyleController.has('strength'),
    'CombatStyleController.set mismatch'
);
await refuses('Fight.hasChaseTarget', () => Fight.prototype.hasChaseTarget.call({}));
await refuses('ClueDuelSearch.constructor', () => new ClueDuelSearch());
await refuses('ClueDuelSearch.tick', () => ClueDuelSearch.prototype.tick.call({}));
await refuses('resetClueDuelSearch', () => resetClueDuelSearch());
await refuses('stableClosedPoll', () => stableClosedPoll());
await refuses('nearestWalkableBank', () => nearestWalkableBank());
await refuses('nearestWalkableBankAsync', () => nearestWalkableBankAsync());
await refuses('bankCostForFinder', () => bankCostForFinder());
await refuses('bankCostForNavigator', () => bankCostForNavigator());
await refuses('buildCatalog', () => buildCatalog());
await refuses('searchCatalog', () => searchCatalog());
await refuses('resolveByName', () => resolveByName());
await refuses('resetLiveCatalog', () => resetLiveCatalog());
await refuses('trailWalkOpts', () => trailWalkOpts());
await refuses('extraItemShop', () => extraItemShop('Rope'));
await refuses('gateItemShop', () => gateItemShop('Shantay pass'));
await refuses('resolveLoopCadence', () => resolveLoopCadence());
await refuses('BranchTask.constructor', () => new BranchTask());
await refuses('BranchTask.validate', () => BranchTask.prototype.validate.call({}));
await refuses('BranchTask.success', () => BranchTask.prototype.success.call({}));
await refuses('BranchTask.failure', () => BranchTask.prototype.failure.call({}));
await refuses('LeafTask.constructor', () => new LeafTask());
await refuses('LeafTask.execute', () => LeafTask.prototype.execute.call({}));
export default class RefreezeProbe extends LoopingBot { loop() {} }
"#;
    let game_data = api::game_data::for_revision(client::io::ClientRevision::R289)
        .expect("R289 selected game data");
    let untradeable_obj_id = game_data
        .items()
        .iter()
        .find(|item| item.name.is_some() && !item.tradeable)
        .expect("R289 named untradeable object")
        .id;
    let ts = ts.replace("__UNTRADEABLE_OBJ_ID__", &untradeable_obj_id.to_string());
    let js = transpile_ts(&ts).expect("transpile refreeze probe");
    let isolate = LoadIsolate::spawn_with_game_data(js, LoadShape::CompatClass, vec![], game_data)
        .expect("spawn refreeze probe with selected game data");
    let ready = wait_ready(&isolate);
    let inventory_rows = [ItemRowInput {
        name: Some("Shark"),
        id: 385,
        count: 2,
        slot: 0,
        component_id: 503,
        noted: false,
        cert: -1,
        ops: &[],
    }];
    let side_rows = [ItemRowInput {
        name: Some("Deposit note"),
        id: 99,
        count: 1,
        slot: 2,
        component_id: 701,
        noted: false,
        cert: -1,
        ops: &[],
    }];
    let mut snapshot = common::ingame_snapshot();
    snapshot.tick = 2;
    snapshot.inv = &inventory_rows;
    snapshot.inv_size = 4;
    snapshot.bank_side = &side_rows;
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(snapshot.tick);
    let closed = isolate
        .probe("__inventoryBackpackProbe()")
        .expect("closed-backpack compatibility probe");
    assert_eq!(closed["rootHelpersAbsent"], json!(true));
    assert_eq!(closed["rootValues"], json!(true));
    assert_eq!(closed["moduleName"], json!("Shark"));
    assert_eq!(closed["moduleCapacity"], json!(4));

    snapshot.tick = 3;
    snapshot.bank_open = true;
    let native = NativeFactsInput {
        side_modal_id: Some(700),
        ..NativeFactsInput::default()
    };
    isolate.post_snapshot(encode_snapshot_with_native(&snapshot, native));
    isolate.on_game_tick(snapshot.tick);
    let open = isolate
        .probe("__inventoryBackpackProbe()")
        .expect("bank-open backpack compatibility probe");
    assert_eq!(open["rootHelpersAbsent"], json!(true));
    assert_eq!(open["rootValues"], json!(true));
    assert_eq!(open["moduleName"], json!("Deposit note"));
    assert_eq!(open["moduleCapacity"], json!(28));
    let logs = isolate.join();
    assert!(
        ready.is_ok(),
        "new refrozen members load and behave: {ready:?}; logs={logs:?}"
    );
}
