//! Consumer-visible runtime probes for JS API v2 quest paths and progress.
use std::sync::Arc;

use api::quest_progress::EvidenceStamp;
use api::selected::{Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use script::api_progress::{JournalEvidence, ProgressPage, QuestProgressRow};
use script::interact::InteractReq;
use script_js::{LoadIsolate, LoadShape};
use {
    script::native::snapshot_source::{NativeFactsInput, SnapshotInput},
    script_js::wire::encode_snapshot_delta_with_native,
};

const BASE_SCRIPT: &str = r#"
export const apiVersion = 2;
export function tick(api) { globalThis.__api = api; }
"#;
const JOURNAL_SCRIPT: &str = r#"
import { Quests } from '../../api/ui/questlog/Quests.js';
export const apiVersion = 2;
export function tick(api) {
    globalThis.__api = api;
    globalThis.__Quests = Quests;
}
"#;

const DRAGON_SCRIPT: &str = r#"
import { DRAGON_STAGE, readDragonProgress } from '../../api/ai/quests/defs/dragonslayer/journal.js';
export const apiVersion = 2;
export function tick(api) {
    globalThis.__api = api;
    globalThis.__DRAGON_STAGE = DRAGON_STAGE;
    globalThis.__readDragonProgress = readDragonProgress;
}
"#;

fn post_snapshot(iso: &LoadIsolate, tick: u64, api_progress: Option<&ProgressPage>) {
    let input = SnapshotInput {
        tick,
        here: None,
        ingame: true,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 7,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        bank_op_result_seq: 0,
        bank_op_result: false,
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 0,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: script::native::snapshot_source::ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
        user_move_intent_seq: 0,
        walk_outcome_cancel_reason: Default::default(),
    };
    let native = NativeFactsInput {
        api_progress,
        ..NativeFactsInput::default()
    };
    let (bytes, _) = encode_snapshot_delta_with_native(None, &input, native, false);
    iso.post_snapshot(bytes);
}

fn tick(iso: &LoadIsolate, tick: u64, api_progress: Option<&ProgressPage>) {
    post_snapshot(iso, tick, api_progress);
    iso.on_game_tick(tick);
    let _ = iso.probe("true").unwrap();
}

fn isolate() -> LoadIsolate {
    let iso = LoadIsolate::spawn(BASE_SCRIPT.into(), LoadShape::NativeTick, vec![]).unwrap();
    tick(&iso, 1, None);
    iso
}
fn journal_isolate() -> LoadIsolate {
    let iso = LoadIsolate::spawn(JOURNAL_SCRIPT.into(), LoadShape::NativeTick, vec![]).unwrap();
    tick(&iso, 1, None);
    iso
}

fn dragon_isolate() -> LoadIsolate {
    let iso = LoadIsolate::spawn(DRAGON_SCRIPT.into(), LoadShape::NativeTick, vec![]).unwrap();
    tick(&iso, 1, None);
    iso
}

fn begin(iso: &LoadIsolate, label: &str, args: &str) {
    let expression = format!(
        r#"(() => {{
          globalThis.__{label}Result = null;
          globalThis.__{label}Settles = 0;
          const promise = __api.questProgress({args});
          globalThis.__{label}Promise = promise;
          promise.then((result) => {{
            globalThis.__{label}Result = result;
            globalThis.__{label}Settles += 1;
          }});
          return true;
        }})()"#
    );
    assert_eq!(iso.probe(&expression).unwrap(), true);
}
fn begin_journal(iso: &LoadIsolate, label: &str, args: &str) {
    let expression = format!(
        r#"(() => {{
          globalThis.__{label}Result = null;
          globalThis.__{label}Settles = 0;
          const promise = __Quests.journal({args});
          globalThis.__{label}Promise = promise;
          promise.then((result) => {{
            globalThis.__{label}Result = result;
            globalThis.__{label}Settles += 1;
          }});
          return true;
        }})()"#
    );
    assert_eq!(iso.probe(&expression).unwrap(), true);
}

fn forward_queued_controls(iso: &LoadIsolate) {
    let current_tick = iso.probe("__api.tick").unwrap();
    let current_tick = current_tick
        .as_f64()
        .unwrap_or_else(|| panic!("non-numeric API tick: {current_tick}"))
        as u64;
    iso.on_game_tick(current_tick);
    let _ = iso.probe("true").unwrap();
}

fn begin_and_take_read(iso: &LoadIsolate, label: &str) -> (u64, String) {
    begin(iso, label, "({quest: 'cook'})");
    forward_queued_controls(iso);
    take_single_read(&iso.drain_interacts())
}

fn take_single_read(requests: &[InteractReq]) -> (u64, String) {
    let reads: Vec<_> = requests
        .iter()
        .filter_map(|request| match request {
            InteractReq::ProgressRead {
                request_id, name, ..
            } => Some((*request_id, name.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(reads.len(), 1, "expected one progress-read in {requests:?}");
    reads.into_iter().next().unwrap()
}

fn result(iso: &LoadIsolate, label: &str) -> serde_json::Value {
    iso.probe(&format!("globalThis.__{label}Result")).unwrap()
}

fn settles(iso: &LoadIsolate, label: &str) -> u64 {
    iso.probe(&format!("globalThis.__{label}Settles"))
        .unwrap()
        .as_u64()
        .unwrap()
}

fn done_page(token: u64) -> ProgressPage {
    ProgressPage::Done {
        token,
        row: Arc::new(QuestProgressRow {
            quest: Arc::from("cook"),
            display: Arc::from("Cook's Assistant"),
            colour: QuestListStatus::InProgress,
            stage: Knowledge::Known(Arc::from("cook:2")),
            rule: Knowledge::Known(Arc::from("cook:2")),
            complete: Truth::True,
            flags: Vec::new().into(),
            evidence: EvidenceStamp {
                run: RunKey {
                    slot: 9,
                    run: 4,
                    session: 2,
                },
                tick: 31,
                sequence: 6,
            },
            journal_read: false,
            binding: Arc::from("cook:assistant"),
            role: Some(Arc::from("cook:main")),
        }),
    }
}

#[test]
fn quest_paths_is_synchronous_and_lists_every_released_path() {
    let index: script::quester::queue::ReleaseIndex =
        serde_json::from_str(script::quester::compile::INDEX_JSON).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../script/paths/289");
    let expected_rows: Vec<_> = index
        .paths
        .into_iter()
        .filter(|entry| entry.unavailable.is_none())
        .map(|entry| {
            let file = entry.file.expect("released row file");
            let document: script::quester::path::PathDocument =
                serde_json::from_slice(&std::fs::read(root.join(&file)).unwrap()).unwrap();
            assert_eq!(document.id.0.as_ref(), entry.id);
            let role = &document.roles[0];
            let mut stages = Vec::new();
            for role in &document.roles {
                let mut role_stages: Vec<_> = role
                    .sequences
                    .iter()
                    .map(|sequence| sequence.stage.0.as_ref())
                    .collect();
                role_stages.sort_by_key(|stage| {
                    stage
                        .rsplit_once(':')
                        .and_then(|(_, ordinal)| ordinal.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });
                stages.extend(role_stages);
            }
            serde_json::json!({
                "id": entry.id,
                "display": document.display_name,
                "journal": !role.progress.as_ref().unwrap().rules.is_empty(),
                "draft": entry.status == Some(script::quester::queue::ReleasePathStatus::Draft),
                "stages": stages,
            })
        })
        .collect();
    let iso = isolate();
    assert_eq!(
        iso.probe("__api.questPaths() instanceof Promise").unwrap(),
        false
    );
    let paths = iso.probe("__api.questPaths()").unwrap();
    assert_eq!(
        paths,
        serde_json::json!({
            "ok": true,
            "value": {
                "rows": expected_rows
            }
        })
    );
    let draft_of = |id: &str| {
        paths["value"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} row is missing"))["draft"]
            .clone()
    };
    assert_eq!(draft_of("druid"), serde_json::json!(true));
    assert_eq!(draft_of("priest"), serde_json::json!(false));
    iso.join();
}

#[test]
fn invalid_progress_arguments_refuse_without_emitting_a_host_row() {
    let iso = isolate();
    for (index, args) in [
        "",
        "null",
        "42",
        "[]",
        "{}",
        "({quest: 1})",
        "({quest: ''})",
        "({quest: '   '})",
    ]
    .into_iter()
    .enumerate()
    {
        let label = format!("invalid_{index}");
        begin(&iso, &label, args);
        tick(&iso, index as u64 + 2, None);
        assert_eq!(
            result(&iso, &label),
            serde_json::json!({"kind": "refused", "reason": "invalid-args"}),
            "invalid input {args}"
        );
        assert_eq!(iso.drain_interacts(), Vec::<InteractReq>::new());
    }
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_questlog__quests_journal__contract() {
    let iso = journal_isolate();

    begin_journal(&iso, "journal", "'cOoK', false");
    forward_queued_controls(&iso);
    let requests = iso.drain_interacts();
    assert_eq!(requests.len(), 1);
    let InteractReq::ProgressRead {
        request_id: token,
        name,
        journal_only,
    } = &requests[0]
    else {
        panic!("quest journal did not request a native read: {requests:?}");
    };
    assert_eq!(name, "cOoK");
    assert!(*journal_only);
    let token = *token;

    tick(&iso, 2, Some(&ProgressPage::Reading { token }));
    assert_eq!(result(&iso, "journal"), serde_json::Value::Null);
    assert!(iso.drain_interacts().is_empty());

    let run = RunKey {
        slot: 17,
        run: 9,
        session: 3,
    };
    let evidence = |tick| EvidenceStamp {
        run,
        tick,
        sequence: tick,
    };
    let done = ProgressPage::JournalDone {
        token,
        result: Some(Arc::new(JournalEvidence {
            quest: Arc::from("cook"),
            root: 8134,
            lines: vec![
                Arc::<str>::from("First journal line"),
                Arc::from("Second line"),
            ]
            .into(),
            colour: Some(0xf8f800),
            acquired: evidence(10),
            closed: evidence(12),
        })),
    };
    tick(&iso, 3, Some(&done));
    assert_eq!(
        result(&iso, "journal"),
        serde_json::json!(["First journal line", "Second line"])
    );
    assert_eq!(settles(&iso, "journal"), 1);

    begin_journal(&iso, "missing", "'not a quest'");
    forward_queued_controls(&iso);
    let requests = iso.drain_interacts();
    let InteractReq::ProgressRead {
        request_id: missing_token,
        journal_only: true,
        ..
    } = &requests[0]
    else {
        panic!("missing quest did not request a journal read: {requests:?}");
    };
    tick(
        &iso,
        4,
        Some(&ProgressPage::JournalDone {
            token: *missing_token,
            result: None,
        }),
    );
    assert_eq!(result(&iso, "missing"), serde_json::json!([]));
    assert_eq!(settles(&iso, "missing"), 1);
    iso.join();
}

fn dragon_journal_page(token: u64, quest: &str, lines: &[&str]) -> ProgressPage {
    let run = RunKey {
        slot: 17,
        run: 9,
        session: 3,
    };
    let evidence = |tick| EvidenceStamp {
        run,
        tick,
        sequence: tick,
    };
    ProgressPage::JournalDone {
        token,
        result: Some(Arc::new(JournalEvidence {
            quest: Arc::from(quest),
            root: 8134,
            lines: lines.iter().map(|line| Arc::<str>::from(*line)).collect(),
            colour: Some(0xf8f800),
            acquired: evidence(10),
            closed: evidence(12),
        })),
    }
}

/// Starts `readDragonProgress` and returns the one native read it asks for.
fn begin_dragon_read(iso: &LoadIsolate, label: &str) -> u64 {
    let expression = format!(
        r#"(() => {{
          globalThis.__{label}Result = null;
          globalThis.__{label}Settles = 0;
          globalThis.__{label}Promise = __readDragonProgress().then((result) => {{
            globalThis.__{label}Result = result === undefined ? 'undefined' : result;
            globalThis.__{label}Settles += 1;
          }});
          return true;
        }})()"#
    );
    assert_eq!(iso.probe(&expression).unwrap(), true);
    forward_queued_controls(iso);
    let requests = iso.drain_interacts();
    assert_eq!(requests.len(), 1, "one native call: {requests:?}");
    let InteractReq::ProgressRead {
        request_id,
        name,
        journal_only,
    } = &requests[0]
    else {
        panic!("readDragonProgress did not request a progress read: {requests:?}");
    };
    assert_eq!(name, "Dragon Slayer");
    assert!(*journal_only);
    *request_id
}

#[test]
fn read_dragon_progress_returns_the_stage_rust_resolves() {
    let iso = dragon_isolate();
    let token = begin_dragon_read(&iso, "ship");
    tick(&iso, 2, Some(&ProgressPage::Reading { token }));
    assert_eq!(settles(&iso, "ship"), 0);

    let repaired = dragon_journal_page(
        token,
        "dragon",
        &[
            "@str@The Guildmaster of the Champions' Guild said his friend",
            "@str@I have repaired my ship using wooden planks and steel",
            "@dbl@I still need to find a @dre@captain@dbl@ for my ship.",
        ],
    );
    tick(&iso, 3, Some(&repaired));
    assert_eq!(result(&iso, "ship"), serde_json::json!({ "stage": 7 }));
    assert_eq!(settles(&iso, "ship"), 1);

    let token = begin_dragon_read(&iso, "done");
    tick(
        &iso,
        4,
        Some(&dragon_journal_page(
            token,
            "dragon",
            &["@red@QUEST COMPLETE!"],
        )),
    );
    assert_eq!(result(&iso, "done"), serde_json::json!({ "stage": 10 }));
    iso.join();
}

#[test]
fn read_dragon_progress_has_no_stage_when_the_read_gives_none() {
    let iso = dragon_isolate();

    let token = begin_dragon_read(&iso, "unmatched");
    let page = dragon_journal_page(token, "dragon", &["Something else entirely."]);
    tick(&iso, 2, Some(&page));
    assert_eq!(result(&iso, "unmatched"), serde_json::json!("undefined"));

    let token = begin_dragon_read(&iso, "missing");
    let page = ProgressPage::JournalDone {
        token,
        result: None,
    };
    tick(&iso, 3, Some(&page));
    assert_eq!(result(&iso, "missing"), serde_json::json!("undefined"));

    let token = begin_dragon_read(&iso, "refused");
    let page = ProgressPage::Refused {
        token,
        reason: Arc::from("busy"),
    };
    tick(&iso, 4, Some(&page));
    assert_eq!(result(&iso, "refused"), serde_json::json!("undefined"));
    iso.join();
}

#[test]
fn dragon_stage_constants_are_the_content_stages() {
    let iso = dragon_isolate();
    let served = iso.probe("globalThis.__DRAGON_STAGE").unwrap();
    let expected: serde_json::Map<String, serde_json::Value> =
        script::quest_journal::dragon_stage::DragonStage::ALL
            .iter()
            .map(|stage| (stage.upstream_name().to_owned(), stage.value().into()))
            .collect();
    assert_eq!(served, serde_json::Value::Object(expected));
    iso.join();
}

#[test]
fn matching_read_ack_and_terminal_pages_settle_exact_values_once() {
    let iso = isolate();
    let (token, quest) = begin_and_take_read(&iso, "progress");
    assert!((1..=(1_u64 << 53) - 1).contains(&token));
    assert_eq!(quest, "cook");

    let stale = ProgressPage::Refused {
        token: token + 1,
        reason: Arc::from("stale"),
    };
    tick(&iso, 2, Some(&stale));
    assert_eq!(result(&iso, "progress"), serde_json::Value::Null);
    assert_eq!(settles(&iso, "progress"), 0);

    let reading = ProgressPage::Reading { token };
    tick(&iso, 3, Some(&reading));
    assert_eq!(result(&iso, "progress"), serde_json::Value::Null);
    assert_eq!(settles(&iso, "progress"), 0);
    assert!(iso.drain_interacts().is_empty());

    let done = done_page(token);
    tick(&iso, 4, Some(&done));
    let expected = serde_json::json!({
        "kind": "done",
        "value": {
            "end": "done",
            "token": token,
            "row": {
                "quest": "cook",
                "display": "Cook's Assistant",
                "colour": "inProgress",
                "stage": {"state": "known", "value": "cook:2"},
                "complete": "true",
                "rule": {"state": "known", "value": "cook:2"},
                "flags": [],
                "evidence": {"run": 4, "session": 2, "tick": 31, "sequence": 6},
                "journal_read": false,
                "binding": "cook:assistant",
                "role": "cook:main"
            }
        }
    });
    assert_eq!(result(&iso, "progress"), expected);
    assert_eq!(settles(&iso, "progress"), 1);

    tick(&iso, 5, Some(&done));
    assert_eq!(result(&iso, "progress"), expected);
    assert_eq!(settles(&iso, "progress"), 1);

    begin(&iso, "unknown", "({quest: 'nope'})");
    forward_queued_controls(&iso);
    let (unknown_token, quest) = take_single_read(&iso.drain_interacts());
    assert_eq!(quest, "nope");
    let refused = ProgressPage::Refused {
        token: unknown_token,
        reason: Arc::from("unknown-path"),
    };
    tick(&iso, 6, Some(&refused));
    assert_eq!(
        result(&iso, "unknown"),
        serde_json::json!({
            "kind": "done",
            "value": {"end": "refused", "token": unknown_token, "reason": "unknown-path"}
        })
    );
    iso.join();
}

#[test]
fn newer_read_supersedes_old_and_reset_aborts_the_live_waiter() {
    let iso = isolate();
    let (first_token, _) = begin_and_take_read(&iso, "first");
    begin(&iso, "second", "({quest: 'cook'})");
    tick(&iso, 2, None);
    assert_eq!(
        result(&iso, "first"),
        serde_json::json!({"kind": "aborted", "reason": "superseded"})
    );
    assert_eq!(settles(&iso, "first"), 1);
    let (second_token, _) = take_single_read(&iso.drain_interacts());
    assert_ne!(first_token, second_token);
    let done = done_page(second_token);
    tick(&iso, 3, Some(&done));
    assert_eq!(result(&iso, "second")["kind"], "done");

    let (reset_token, _) = begin_and_take_read(&iso, "reset");
    iso.reset_session_work(None);
    tick(&iso, 4, None);
    assert_eq!(
        result(&iso, "reset"),
        serde_json::json!({"kind": "aborted", "reason": "reset"})
    );
    assert!(iso.drain_interacts().is_empty());

    let (fresh_token, quest) = begin_and_take_read(&iso, "fresh");
    assert_ne!(fresh_token, reset_token);
    assert_eq!(quest, "cook");
    tick(&iso, 5, Some(&done_page(fresh_token)));
    assert_eq!(result(&iso, "fresh")["kind"], "done");
    iso.join();
}

#[test]
fn reconnect_reemits_only_unacknowledged_read_once_per_epoch() {
    let iso = isolate();
    let (lost_token, _) = begin_and_take_read(&iso, "lost");
    iso.reconnect_session_work(None);
    assert!(iso.drain_interacts().is_empty());
    tick(&iso, 2, None);
    assert_eq!(take_single_read(&iso.drain_interacts()).0, lost_token);
    tick(&iso, 3, None);
    assert!(iso.drain_interacts().is_empty());
    tick(&iso, 4, Some(&done_page(lost_token)));
    assert_eq!(result(&iso, "lost")["kind"], "done");

    let (acked_token, _) = begin_and_take_read(&iso, "acked");
    tick(&iso, 5, Some(&ProgressPage::Reading { token: acked_token }));
    assert!(iso.drain_interacts().is_empty());
    iso.reconnect_session_work(None);
    assert!(iso.drain_interacts().is_empty());
    tick(&iso, 6, None);
    assert!(iso.drain_interacts().is_empty());
    tick(&iso, 7, Some(&done_page(acked_token)));
    assert_eq!(result(&iso, "acked")["kind"], "done");
    iso.join();
}
