//! COMPAT-FINISH-029 bank family: published contracts that run in the
//! isolate (`findQuestJunk`, `Bank.normalBackpackSnapshot`) and the absence
//! checks for the removed private exports and compat-only Bank members
//! (plan-r3 section 3, "Removal tests and proxy traps").

use script::native::snapshot_source::{ItemRowInput, NativeFactsInput, SnapshotInput};
use script_js::{LoadIsolate, LoadShape, Ready};
use std::time::{Duration, Instant};

mod common;

const INDEX_D_TS: &str = include_str!("../compat-js/index.d.ts");
const MODULES_JSON: &str = include_str!("../compat-js/modules.json");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn bot(imports: &str, body: &str) -> String {
    format!(
        r#"
{imports}
export default class T extends LoopingBot {{
    loop() {{
        if (globalThis.__done) return;
        globalThis.__done = true;
        {body}
    }}
}}
"#
    )
}

fn run(src: &str, expr: &str) -> serde_json::Value {
    let iso = spawn(src);
    iso.on_game_tick(1);
    let value = iso.probe(expr).unwrap();
    iso.join();
    value
}

/// Word-bounded presence of `name` in a generated file.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !ident(before) && !ident(after)
    })
}

/// A removed named export fails module linking (the isolate's setup result),
/// and no generated output promises it.
fn assert_export_removed(module: &str, name: &str) {
    let src = bot(&format!("import {{ {name} }} from '{module}';"), "");
    let iso = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        match iso.poll_ready() {
            Ready::Failed(error) => break error,
            Ready::Ready => {
                iso.join();
                panic!("importing removed {name} linked");
            }
            Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ready::Pending => panic!("isolate setup timed out importing {name}"),
        }
    };
    iso.join();
    assert!(
        error.contains(name),
        "the link error names the removed export {name}: {error}"
    );
    for (file, text) in [
        ("index.d.ts", INDEX_D_TS),
        ("modules.json", MODULES_JSON),
        ("abi-contract.json", ABI_CONTRACT),
    ] {
        assert!(!mentions(text, name), "{file} still promises {name}");
    }
}

/// A removed `Bank` member: no own key or descriptor, the proxy's generic
/// unknown-member trap (the same error shape as any unknown key), no
/// generated promise, and the supported siblings still answer.
fn assert_bank_member_removed(member: &str) {
    let src = bot(
        "import { Bank } from '../../api/bank/Bank.js';",
        &format!(
            r#"
const trap = (key) => {{ try {{ Bank[key]; return 'read'; }} catch (e) {{ return e.message; }} }};
globalThis.__facts = {{
    own: Object.prototype.hasOwnProperty.call(Bank, '{member}'),
    descriptor: Object.getOwnPropertyDescriptor(Bank, '{member}') === undefined,
    keys: Object.keys(Bank).includes('{member}'),
    removed: trap('{member}'),
    unknown: trap('__noSuchBankMember'),
    siblings: typeof Bank.depositAllMatching === 'function'
        && typeof Bank.openNearest === 'function'
        && typeof Bank.backpackReady === 'function'
        && typeof Bank.normalBackpackSnapshot === 'function',
}};
"#
        ),
    );
    let facts = run(&src, "__facts");
    assert_eq!(facts["own"], false, "{member} is not an own key");
    assert_eq!(facts["descriptor"], true, "{member} has no own descriptor");
    assert_eq!(facts["keys"], false, "{member} is not enumerated");
    assert_eq!(
        facts["removed"]
            .as_str()
            .map(|message| message.replace(member, "<key>")),
        facts["unknown"]
            .as_str()
            .map(|message| message.replace("__noSuchBankMember", "<key>")),
        "a removed member uses the generic unknown-member trap"
    );
    assert_eq!(facts["siblings"], true, "supported Bank siblings remain");
    // Only the `Bank.js` module block: `bankRules.depositAllExcept` (the
    // `Banking.depositAllExcept` re-export) stays.
    let start = INDEX_D_TS
        .find("declare module '*api/bank/Bank.js' {")
        .expect("index.d.ts declares the Bank module");
    let block = &INDEX_D_TS[start..];
    let block = &block[..block.find("\n}\n").expect("the Bank module block closes")];
    assert!(
        !block.contains(&format!(" {member}(")),
        "index.d.ts still declares Bank.{member}"
    );
    assert!(
        !mentions(ABI_CONTRACT, &format!("Bank.{member}")),
        "abi-contract.json still promises Bank.{member}"
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__bankpaceticks__removed() {
    assert_export_removed("../../api/bank/Banking.js", "bankPaceTicks");
    // The supported sibling stays importable.
    let value = run(
        &bot(
            "import { bankPace, purgePackAtBank, withdrawCoins } from '../../api/bank/Banking.js';",
            "globalThis.__ok = [bankPace, purgePackAtBank, withdrawCoins].every((f) => typeof f === 'function');",
        ),
        "__ok",
    );
    assert_eq!(value, serde_json::json!(true));
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__nearestwalkablebank__removed() {
    assert_export_removed("../../api/bank/BankLocations.js", "nearestWalkableBank");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__nearestwalkablebankasync__removed() {
    assert_export_removed(
        "../../api/bank/BankLocations.js",
        "nearestWalkableBankAsync",
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__bankcostforfinder__removed() {
    assert_export_removed("../../api/bank/BankLocations.js", "bankCostForFinder");
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__bankcostfornavigator__removed() {
    assert_export_removed("../../api/bank/BankLocations.js", "bankCostForNavigator");
    let value = run(
        &bot(
            "import { nearestBank, nearestBankReachable, bankDistance } from '../../api/bank/BankLocations.js';",
            "globalThis.__ok = [nearestBank, nearestBankReachable, bankDistance].every((f) => typeof f === 'function');",
        ),
        "__ok",
    );
    assert_eq!(value, serde_json::json!(true));
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__bank_depositallexcept__removed() {
    assert_bank_member_removed("depositAllExcept");
    // `Banking.depositAllExcept` (the supported predicate builder) stays.
    let value = run(
        &bot(
            "import { depositAllExcept } from '../../api/bank/Banking.js';",
            "const keep = depositAllExcept(['Knife']); globalThis.__ok = [keep('knife'), keep('Logs'), keep('')];",
        ),
        "__ok",
    );
    assert_eq!(value, serde_json::json!([false, true, false]));
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bank__bank_opennearestworld__removed() {
    assert_bank_member_removed("openNearestWorld");
    // The clue adapter that used it keeps its export through the private
    // typed request.
    let value = run(
        &bot(
            "import { openClueBank } from '../../api/ai/clues/bankAccess.js';",
            "globalThis.__ok = typeof openClueBank === 'function';",
        ),
        "__ok",
    );
    assert_eq!(value, serde_json::json!(true));
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__findquestjunk__contract() {
    if !api::test_support::game_data_available() {
        return;
    }
    let src = bot(
        "import { QUEST_JUNK, findQuestJunk } from '../../api/bank/bankQuestJunk.js';",
        r#"
const asked = [];
const statuses = { 'Demon Slayer': 'complete', 'Dragon Slayer': 'inProgress' };
const banked = [{ id: 2400 }, { id: 2399 }, { id: 2400 }, { id: 1538 }, { id: 995 }, { id: '2401' }];
const found = findQuestJunk(banked, (quest) => { asked.push(quest); return statuses[quest] ?? 'unknown'; });
let oversized = null;
try {
    findQuestJunk(Array.from({ length: 289 }, () => ({ id: 2399 })), () => { asked.push('oversized'); return 'complete'; });
} catch (e) { oversized = e.constructor.name; }
let notArray = null;
try { findQuestJunk(null, () => 'complete'); } catch (e) { notArray = e.constructor.name; }
globalThis.__junk = {
    table: QUEST_JUNK.length > 0 && QUEST_JUNK.every((r) => typeof r.id === 'number' && typeof r.name === 'string' && typeof r.quest === 'string'),
    tableOrderIds: QUEST_JUNK.filter((r) => [2399, 2400, 1538].includes(r.id)).map((r) => r.id),
    found,
    asked,
    oversized,
    notArray,
};
"#,
    );
    let value = run(&src, "__junk");
    assert_eq!(value["table"], true, "QUEST_JUNK is the native table");
    // Findings follow table order, one callback per matched entry, duplicate
    // input ids do not duplicate entries, a string id never matches, and an
    // unlisted id produces nothing.
    let order: Vec<i64> = value["tableOrderIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_i64().unwrap())
        .collect();
    let found = value["found"].as_array().unwrap();
    let ids: Vec<i64> = found
        .iter()
        .map(|row| row["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, order);
    assert_eq!(value["asked"].as_array().unwrap().len(), found.len());
    for row in found {
        let status = row["status"].as_str().unwrap();
        assert_eq!(row["droppable"], status == "complete");
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys.len(),
            5,
            "id, name, quest, status, droppable: {keys:?}"
        );
    }
    let demon = found.iter().find(|row| row["id"] == 2399).unwrap();
    assert_eq!(demon["quest"], "Demon Slayer");
    assert_eq!(demon["droppable"], true);
    let dragon = found.iter().find(|row| row["id"] == 1538).unwrap();
    assert_eq!(dragon["status"], "inProgress");
    assert_eq!(dragon["droppable"], false);
    // An oversized list is rejected before any callback runs.
    assert_eq!(value["oversized"], "RangeError");
    assert!(!value["asked"]
        .as_array()
        .unwrap()
        .iter()
        .any(|quest| quest == "oversized"));
    assert_eq!(value["notArray"], "TypeError");
}

fn pack_row(name: &'static str, id: i32, count: i32, slot: i32) -> ItemRowInput<'static> {
    ItemRowInput {
        name: Some(name),
        count,
        id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: 3214,
        slot,
    }
}

fn post(iso: &LoadIsolate, input: &SnapshotInput<'_>, inv_ready: bool) {
    iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
        input,
        NativeFactsInput {
            inv_ready,
            ..Default::default()
        },
    ));
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bank__bank_normalbackpacksnapshot__contract() {
    let src = bot(
        "import { Bank } from '../../api/bank/Bank.js';",
        "globalThis.__done = false; globalThis.__snap = Bank.normalBackpackSnapshot();",
    );
    let iso = spawn(&src);
    let rows = [pack_row("Logs", 1511, 1, 0), pack_row("Coins", 995, 25, 3)];
    let mut input = common::ingame_snapshot();
    input.inv = &rows;
    input.inv_size = 28;

    // Ready, closed, 28 slots: slot/id/name/count rows only.
    post(&iso, &input, true);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__snap").unwrap(),
        serde_json::json!([
            { "slot": 0, "id": 1511, "name": "Logs", "count": 1 },
            { "slot": 3, "id": 995, "name": "Coins", "count": 25 },
        ])
    );

    // An empty but ready 28-slot pack is `[]`, not null.
    input.inv = &[];
    post(&iso, &input, true);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__snap").unwrap(), serde_json::json!([]));

    // An untransmitted pack, an open bank, or a locked inv tab is null.
    input.inv = &rows;
    post(&iso, &input, false);
    iso.on_game_tick(3);
    assert_eq!(iso.probe("__snap").unwrap(), serde_json::Value::Null);
    input.bank_open = true;
    post(&iso, &input, true);
    iso.on_game_tick(4);
    assert_eq!(iso.probe("__snap").unwrap(), serde_json::Value::Null);
    input.bank_open = false;
    input.inv_size = 0;
    post(&iso, &input, true);
    iso.on_game_tick(5);
    assert_eq!(iso.probe("__snap").unwrap(), serde_json::Value::Null);
    iso.join();
}

/// `reader.bankItems` (BankSorter's report input) lists the open bank's posted
/// rows and is empty while the bank is closed. The client-adapter census row
/// lives here because the bank unit needs it first.
#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_client_adapter__reader_bankitems__contract() {
    let src = bot(
        "import { reader } from '../../adapter/ClientAdapter.js';",
        "globalThis.__done = false; globalThis.__snap = reader.bankItems().map(row => [row.slot, row.id, row.name, row.count]);",
    );
    let iso = spawn(&src);
    let rows = [pack_row("Key", 2399, 1, 0), pack_row("Coins", 995, 40, 1)];
    let mut input = common::ingame_snapshot();
    input.bank = &rows;
    input.bank_open = true;
    input.bank_loaded = true;
    post(&iso, &input, true);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__snap").unwrap(),
        serde_json::json!([[0, 2399, "Key", 1], [1, 995, "Coins", 40]])
    );
    input.bank_open = false;
    post(&iso, &input, true);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__snap").unwrap(), serde_json::json!([]));
    iso.join();
}
