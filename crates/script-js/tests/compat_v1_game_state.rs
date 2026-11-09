//! COMPAT-FINISH-029 Game state family: native spell-on-NPC dispatch and the
//! removed compat-only autoRetaliate alias.

mod common;

use api::identity::ExecutionId;
use api::script_contract::ChildOrigin;
use api::selected::{ClientRevision, RunKey};
use script::interact::InteractReq;
use script::native::request::NativeKind;
use script::native::seat::{SeatEntry, SeatResult, SeatTerminal};
use script::native::seat_rows::{SeatLiveDetails, SeatPageInput, SeatResultValue, SeatStartArgs};
use script::native::snapshot_source::{
    CombatStyleInput, NativeFactsInput, SceneEntityInput, SnapshotInput,
};
use script::native::NativePhase;
use script_js::wire::IsolateBuf;
use script_js::{LoadIsolate, LoadShape};
use serde_json::Value;

const MODULES_JSON: &str = include_str!("../compat-js/modules.json");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");
const INDEX_D_TS: &str = include_str!("../compat-js/index.d.ts");

const CAST_SRC: &str = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    async loop() {
        if (globalThis.__did) return;
        globalThis.__did = true;
        globalThis.__cast = null;
        globalThis.__missing = null;
        globalThis.__cast = await Game.castOnNpc('WIND STRIKE', { index: 7 });
        globalThis.__missing = await Game.castOnNpc('not a spell', null);
    }
}
"#;

const REMOVAL_SRC: &str = r#"
import { Game } from '../../api/game/Game.js';
export default class T extends LoopingBot {
    loop() {
        const trap = (key) => {
            try { Game[key]; return 'read'; }
            catch (error) { return error.message; }
        };
        const plain = {};
        globalThis.__facts = {
            own: Object.prototype.hasOwnProperty.call(Game, 'autoRetaliate'),
            descriptor: Object.getOwnPropertyDescriptor(Game, 'autoRetaliate') === undefined,
            keys: Reflect.ownKeys(Game).includes('autoRetaliate'),
            removed: trap('autoRetaliate'),
            unknown: trap('__noSuchGameMember'),
            plain: plain.autoRetaliate === undefined,
            retaliate: Game.autoRetaliateOn(),
            setRetaliate: Game.setAutoRetaliate(false),
            spellTargets: typeof Game.castOnNpc === 'function'
                && typeof Game.castOnLoc === 'function'
                && typeof Game.castOnItem === 'function',
            style: typeof Game.setCombatStyle === 'function',
        };
    }
}
"#;

fn spawn(source: &str) -> LoadIsolate {
    LoadIsolate::spawn(source.to_owned(), LoadShape::CompatClass, vec![])
        .expect("compat isolate starts")
}
fn post_native_seat_page<'a>(
    isolate: &LoadIsolate,
    buffer: &mut IsolateBuf,
    snapshot: &SnapshotInput<'_>,
    live: &'a [SeatEntry],
    results: &'a [SeatResult],
    details: &'a [SeatLiveDetails<'a>],
    values: &'a [SeatResultValue<'a>],
) {
    let page = SeatPageInput::new(live, results, details, 0)
        .expect("valid native seat page")
        .with_result_values(values)
        .expect("valid native seat result values");
    let bytes = buffer.encode_snapshot_with_native(
        snapshot,
        NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        },
    );
    isolate.post_snapshot(bytes);
    isolate.on_game_tick(snapshot.tick);
    let _ = isolate
        .probe("true")
        .expect("posted native page tick completes");
}

fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        let ident = |character: Option<char>| {
            character.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        };
        !ident(before) && !ident(after)
    })
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_game_state__game_castonnpc__contract() {
    if !api::test_support::game_data_available() {
        return;
    }
    let execution = ExecutionId::first(RunKey {
        slot: 4,
        run: 9,
        session: 1,
    });
    let iso = LoadIsolate::spawn_with_execution(
        CAST_SRC.to_owned(),
        LoadShape::CompatClass,
        vec![],
        execution,
    )
    .expect("compat isolate with a native execution starts");
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected data");
    let component_id = selected
        .spells()
        .iter()
        .find(|spell| spell.name.eq_ignore_ascii_case("wind strike"))
        .expect("selected Wind Strike")
        .component_id;
    let spells = [CombatStyleInput {
        mode: 0,
        label: "Wind Strike",
        component_id,
    }];
    let actions: [String; 0] = [];
    let npcs = [SceneEntityInput {
        index: 7,
        id: 1,
        name: Some("Goblin"),
        x: 3222,
        z: 3222,
        level: 0,
        distance: 1,
        health: 10,
        max_health: 10,
        in_combat: false,
        animating: false,
        actions: &actions,
        reachable: true,
        reachable_adj: true,
        combat_level: 2,
        target_kind: 0,
        target_index: -1,
        size: 1,
        nx: 3222,
        nz: 3222,
        shape: 0,
        angle: 0,
    }];
    let mut snapshot = common::ingame_snapshot();
    snapshot.spell_buttons = &spells;
    snapshot.npcs = &npcs;
    let mut buffer = IsolateBuf::new();
    post_native_seat_page(&iso, &mut buffer, &snapshot, &[], &[], &[], &[]);

    assert_eq!(
        iso.probe("__cast").expect("pending cast result"),
        Value::Null,
        "starting a native cast does not fabricate success"
    );
    assert_eq!(
        iso.drain_interacts(),
        Vec::<InteractReq>::new(),
        "the shim never submits the legacy UseWidgetOn interaction"
    );
    let batches = iso.drain_seat_batches();
    let starts: Vec<_> = batches
        .iter()
        .flat_map(|batch| batch.batch.starts.iter())
        .collect();
    assert_eq!(starts.len(), 1, "one typed native cast starts");
    let start = starts[0];
    let id = start.id;
    assert_eq!(start.kind, NativeKind::CastNpc);
    assert_eq!(
        start.args,
        SeatStartArgs::CastNpc(script::combat::CastNpcRequest {
            component_id,
            index: 7,
        }),
        "the exact selected spell and NPC index cross the native seat wire"
    );

    let live = [SeatEntry {
        id,
        parent: None,
        kind: NativeKind::CastNpc,
        origin: ChildOrigin::Started,
        depth: 1,
        decisions_consumed: 0,
        prompt: None,
    }];
    let details = [SeatLiveDetails {
        id,
        phase: NativePhase::Working,
        prompt: None,
    }];
    snapshot.tick = 2;
    post_native_seat_page(&iso, &mut buffer, &snapshot, &live, &[], &details, &[]);
    let live_batches = iso.drain_seat_batches();
    assert!(
        live_batches
            .iter()
            .all(|batch| batch.batch.starts.iter().all(|start| start.id != id)),
        "a running cast is not started twice"
    );

    let terminal = [SeatResult {
        id,
        seq: 1,
        outcome: SeatTerminal::Done,
        prior_parent: None,
        session: Some(id),
        authority_revoked: false,
    }];
    let encoded = script::channel::encode(&Value::Bool(true)).expect("encoded native result");
    let values = [SeatResultValue {
        id,
        value: &encoded,
    }];
    snapshot.tick = 3;
    post_native_seat_page(&iso, &mut buffer, &snapshot, &[], &terminal, &[], &values);
    let terminal_batches = iso.drain_seat_batches();
    assert!(
        terminal_batches
            .iter()
            .any(|batch| batch.batch.consumed_terminal_seq == 1),
        "the terminal seat page is acknowledged by the next isolate batch"
    );
    assert_eq!(iso.probe("__cast").expect("cast result"), Value::Bool(true));
    assert_eq!(
        iso.probe("__missing").expect("missing-spell result"),
        Value::Bool(false)
    );
    assert!(
        iso.drain_interacts().is_empty(),
        "native seat results do not create legacy JS interactions"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_game_state__game_autoretaliate__removed() {
    let iso = spawn(REMOVAL_SRC);
    let mut snapshot = common::ingame_snapshot();
    snapshot.retaliate_enabled = true;
    common::post_snapshot_input(&iso, &snapshot);
    iso.on_game_tick(1);
    let facts = iso.probe("__facts").expect("removal facts");
    let interactions = iso.drain_interacts();
    iso.join();

    assert_eq!(facts["own"], false, "Game has no own autoRetaliate member");
    assert_eq!(
        facts["descriptor"], true,
        "Game has no autoRetaliate descriptor"
    );
    assert_eq!(facts["keys"], false, "Game has no autoRetaliate key");
    let removed = facts["removed"].as_str().expect("generic proxy error");
    let unknown = facts["unknown"].as_str().expect("generic proxy error");
    assert_eq!(
        removed.replace("Game.autoRetaliate", "Game.<key>"),
        unknown.replace("Game.__noSuchGameMember", "Game.<key>"),
        "a removed member follows the generic unknown-key proxy trap"
    );
    assert_eq!(facts["plain"], true, "plain-object reads remain undefined");
    assert_eq!(
        facts["retaliate"], true,
        "autoRetaliateOn reads posted native state"
    );
    assert_eq!(
        facts["setRetaliate"], true,
        "setAutoRetaliate remains available"
    );
    assert_eq!(
        facts["spellTargets"], true,
        "the supported spell-target methods remain"
    );
    assert_eq!(facts["style"], true, "setCombatStyle remains available");
    assert_eq!(
        interactions,
        vec![InteractReq::SetRetaliate { on: false }],
        "the supported retaliation control still queues its native interaction"
    );

    for (file, text) in [
        ("index.d.ts", INDEX_D_TS),
        ("modules.json", MODULES_JSON),
        ("abi-contract.json", ABI_CONTRACT),
    ] {
        assert!(
            !mentions(text, "autoRetaliate"),
            "{file} still promises autoRetaliate"
        );
    }
}
