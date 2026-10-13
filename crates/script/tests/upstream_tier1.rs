//! Frozen upstream tier-1 tests, executed as modules in the production Load isolate.
//! Only `bun:test` is remapped to the local assertion/registration shim; upstream
//! test sources remain byte-identical under `tests/upstream/2de6bb23`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use script::{transpile_ts, LoadIsolate, LoadShape, Ready};

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
            } else if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.ends_with(".test.ts")) {
                tests.push(path);
            }
        }
    }
    tests.sort();
    tests
}

fn test_url(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).expect("test is below frozen root");
    let mut url = relative.to_string_lossy().replace('\\', "/");
    url.truncate(url.len() - ".ts".len());
    format!("{TEST_MODULE_PREFIX}/{url}")
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

fn run_upstream_file(root: &Path, path: &Path, harness: &str) -> serde_json::Value {
    let file = path.strip_prefix(root).expect("test is below frozen root");
    let file = file.to_string_lossy().replace('\\', "/");
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => return serde_json::json!({ "file": file, "stage": "read", "error": error.to_string() }),
    };
    let js = match transpile_ts(&source) {
        Ok(js) => js,
        Err(error) => return serde_json::json!({ "file": file, "stage": "transpile", "error": error }),
    };
    let harness_import = format!("'{TEST_HARNESS_URL}'");
    let adapted = js
        .replace("'bun:test'", &harness_import)
        .replace("\"bun:test\"", &harness_import);
    let url = test_url(root, path);
    let siblings = vec![
        (TEST_HARNESS_URL.to_string(), harness.to_string()),
        (url.clone(), adapted),
    ];
    let isolate = match LoadIsolate::spawn(bot_source(&url), LoadShape::CompatClass, siblings) {
        Ok(isolate) => isolate,
        Err(error) => return serde_json::json!({ "file": file, "stage": "spawn", "error": error.to_string() }),
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
        Ok(value) => serde_json::json!({ "file": file, "stage": "result", "error": "runner returned no case array", "value": value, "logs": logs }),
        Err(error) => serde_json::json!({ "file": file, "stage": "result", "error": error, "logs": logs }),
    }
}

#[test]
fn upstream_tier1_runs_unmodified_sources_in_load() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(FROZEN_TESTS);
    let tests = collect_test_files(&root);
    assert_eq!(tests.len(), EXPECTED_TEST_FILES, "tier-1 source inventory changed");
    let harness = include_str!("upstream/harness/bun_test.js");
    let results: Vec<_> = tests.iter().map(|path| run_upstream_file(&root, path, harness)).collect();

    if let Some(evidence_dir) = std::env::var_os("COMPAT_REFREEZE_EVIDENCE_DIR") {
        let evidence_dir = PathBuf::from(evidence_dir);
        std::fs::create_dir_all(&evidence_dir).expect("create evidence directory");
        let report = serde_json::json!({ "revision": "2de6bb2397ceb9ffd05f2354f67dde328e324531", "file_count": results.len(), "results": results });
        std::fs::write(evidence_dir.join("upstream-tier1.json"), serde_json::to_vec_pretty(&report).unwrap()).expect("write upstream results");
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
}

#[test]
fn refrozen_runtime_members_load_and_refuse_by_name() {
    let ts = r#"
import { InvItem as ModuleInvItem } from '#/bot/api/inventory/Inventory.js';
import { isEdibleFood as foodIsEdible } from '#/bot/api/combat/food.js';
import { lockedBankAt } from '#/bot/api/bank/BankLocations.js';
import { SUPER_DEFENCE, SUPER_SET } from '#/bot/api/combat/boostPotions.js';
import { ENCLAVE_TARGET_OPTIONS as options, GUTANOTH_BLUE, enclaveTargets as selectEnclave } from '#/bot/api/combat/hunting/sites.js';
import { chasesTarget, Fight } from '#/bot/api/combat/hunting/combat.js';
import { ClueDuelSearch } from '#/bot/api/duel/ClueDuel.js';
import { resetClueDuelSearch } from '#/bot/api/ai/clues/duelTravel.js';
import { toolAttackLevel as rootToolAttackLevel, toolsNeedingEquip as rootToolsNeedingEquip, InvItem as RootInvItem } from '@rs2b0t/api';
import { toolAttackLevel, toolsNeedingEquip } from '#/bot/api/acquisition/Tools.js';
import { parseBankStrategy } from '#/bot/api/bank/Banking.js';
import type { EventMap, Task, InvItem as InvItemType } from '@rs2b0t/api';
import type { BankOpts, CombatHost } from '#/bot/api/combat/hunting/supply.js';

const check = (ok, message) => { if (!ok) throw new Error(message); };
async function refuses(member, call) {
    let error;
    try { await call(); } catch (caught) { error = caught; }
    check(error && String(error.message ?? error).includes(member), `missing named refusal for ${member}`);
}
check(ModuleInvItem === RootInvItem, 'InvItem root re-export differs from module');
check(new ModuleInvItem({ id: 1, name: 'Shark', count: 1, slot: 0, noted: true }).noted === true, 'InvItem.noted missing');
check(foodIsEdible({ name: 'Shark', noted: false }, 'Shark') === true, 'isEdibleFood false for unnoted food');
check(foodIsEdible({ name: 'Shark', noted: true }, 'Shark') === false, 'isEdibleFood accepted a noted item');
const locked = lockedBankAt({ x: 3185, z: 3440, level: 0 });
check(locked === null || typeof locked === 'object', 'lockedBankAt result shape');
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
check(parseBankStrategy('loot') === 'items' && parseBankStrategy('Loot count') === 'items', 'parseBankStrategy loot alias mismatch');
const taskBot = new TaskBot();
let activeName;
taskBot.add({ label: 'sample task', validate: () => true, execute: () => { activeName = taskBot.activeTaskName; } });
await taskBot.loop();
check(activeName === 'sample task' && taskBot.activeTaskName === null, 'TaskBot.activeTaskName lifecycle mismatch');
await refuses('Fight.hasChaseTarget', () => Fight.prototype.hasChaseTarget.call({}));
await refuses('ClueDuelSearch.constructor', () => new ClueDuelSearch());
await refuses('ClueDuelSearch.tick', () => ClueDuelSearch.prototype.tick.call({}));
await refuses('resetClueDuelSearch', () => resetClueDuelSearch());
export default class RefreezeProbe extends LoopingBot { loop() {} }
"#;
    let js = transpile_ts(ts).expect("transpile refreeze probe");
    LoadIsolate::validate_source(&js, LoadShape::CompatClass, &[]).expect("new refrozen members load and behave");
}
