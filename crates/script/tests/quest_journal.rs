//! `api.questJournalBegin` / `Next` / `Close`: one owned token over the posted
//! quest row and the paired main-modal texts.
//!
//! Fixture-only. A posted click target is not a headed journal open: this is
//! not F-PROOF and this file runs no LIVE. The refusals are the point — no
//! click on an occupied or unusable pair, no acquire of the closed pair, one
//! `close-modal` for the acquired root only, and no verb from an abort.

use std::time::Duration;

use script::isolate_fb::{
    IsolateBuf, MainModalTextsInput, NativeFactsInput, QuestStatusInput, ReachViewInput,
    SnapshotFingerprint, SnapshotInput, WidgetTextInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value as Json};

/// Past the machine's own 3000ms acquisition window.
const PAST_WINDOW_MS: u64 = 3_100;

/// Probe runner: clears the interact queue, calls one method, and returns both
/// the public result and exactly what that call queued. `argCount === 0` is a
/// call with no arguments at all.
const SCRIPT: &str = r#"
export const apiVersion = 2;
globalThis.__questRun = function (method, argCount, input) {
  const h = globalThis.__rs2b0t_host || (globalThis.__rs2b0t_host = { interact: [], log: [] });
  h.interact = [];
  const out = argCount === 0 ? globalThis.__rs_api[method]() : globalThis.__rs_api[method](input);
  const queued = (h.interact || []).slice();
  h.interact = [];
  return { out: out === undefined ? null : out, queued: queued };
};
export function tick(api) {}
"#;

/// The same flow through a real native tick, so the host forwards the queued
/// verbs through the tick drain instead of a probe.
const TICK_SCRIPT: &str = r#"
export const apiVersion = 2;
globalThis.__questCall = null;
globalThis.__questOut = null;
export function tick(api) {
  const call = globalThis.__questCall;
  if (!call) return;
  globalThis.__questCall = null;
  const out = api[call.method](call.input);
  globalThis.__questOut = out === undefined ? null : out;
}
"#;

fn base_snapshot<'a>() -> SnapshotInput<'a> {
    SnapshotInput {
        tick: 1,
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
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        withdraw_load_result_seq: 0,
        withdraw_load_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: 2,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: Some("bot"),
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
        scene_state: 2,
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
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

fn row<'a>(name: &'a str, status: &'a str, component_id: Option<i32>) -> QuestStatusInput<'a> {
    QuestStatusInput {
        name,
        status,
        component_id,
    }
}

/// Quest rows the fixture posts. `Keep` is the production default — every
/// rebuild re-posts the tab — and `Null` is the unbound tab (`post_base`
/// encodes null).
enum Rows<'a> {
    Keep,
    Set(&'a [QuestStatusInput<'a>]),
    Null,
}

struct Post<'a> {
    tick: u64,
    rows: Rows<'a>,
    pair: Option<MainModalTextsInput<'a>>,
    widgets: &'a [WidgetTextInput<'a>],
    hold: bool,
    /// Write every field instead of a delta: the pair is written again even
    /// when its content did not change (a same-id co-post).
    full: bool,
}

impl<'a> Post<'a> {
    fn at(tick: u64) -> Self {
        Self {
            tick,
            rows: Rows::Keep,
            pair: None,
            widgets: &[],
            hold: false,
            full: false,
        }
    }

    fn rows(mut self, rows: &'a [QuestStatusInput<'a>]) -> Self {
        self.rows = Rows::Set(rows);
        self
    }

    fn null_tab(mut self) -> Self {
        self.rows = Rows::Null;
        self
    }

    fn closed_pair(mut self) -> Self {
        self.pair = Some(MainModalTextsInput {
            root: -1,
            texts: &[],
        });
        self
    }

    fn pair(mut self, root: i32, texts: &'a [String]) -> Self {
        self.pair = Some(MainModalTextsInput { root, texts });
        self
    }

    fn widgets(mut self, widgets: &'a [WidgetTextInput<'a>]) -> Self {
        self.widgets = widgets;
        self
    }

    fn hold(mut self, hold: bool) -> Self {
        self.hold = hold;
        self
    }

    fn full(mut self) -> Self {
        self.full = true;
        self
    }
}

fn post_page(
    iso: &LoadIsolate,
    buf: &mut IsolateBuf,
    last: &mut Option<SnapshotFingerprint>,
    page: Post<'_>,
    rows: Option<&[QuestStatusInput<'_>]>,
) {
    let mut snap = base_snapshot();
    snap.tick = page.tick;
    snap.hold = page.hold;
    snap.widgets = page.widgets;
    if let Some(pair) = page.pair {
        // A real rebuild writes slot 68 and the pair from the same walk. The
        // machine still reads the pair only: the pair is the occupancy fact.
        snap.main_modal_id = pair.root;
    }
    let facts = NativeFactsInput {
        quest_statuses: rows,
        main_modal_texts: page.pair,
        ..Default::default()
    };
    let (bytes, fingerprint) = buf.encode_snapshot_delta_with_native(
        if page.full { None } else { last.as_ref() },
        &snap,
        facts,
        false,
    );
    *last = Some(fingerprint);
    iso.post_snapshot(bytes);
}

struct Journal {
    iso: LoadIsolate,
    buf: IsolateBuf,
    fingerprint: Option<SnapshotFingerprint>,
    /// The tab the fixture re-posts each rebuild. `None` is the null tab.
    rows: Option<Vec<(String, String, Option<i32>)>>,
}

impl Journal {
    fn new() -> Self {
        Self::with_script(SCRIPT)
    }

    fn with_script(js: &str) -> Self {
        Self {
            iso: LoadIsolate::spawn(js.to_string(), LoadShape::NativeTick, vec![]).unwrap(),
            buf: IsolateBuf::new(),
            fingerprint: None,
            rows: None,
        }
    }

    fn post(&mut self, page: Post<'_>) {
        match page.rows {
            Rows::Set(rows) => {
                self.rows = Some(
                    rows.iter()
                        .map(|row| {
                            (
                                row.name.to_string(),
                                row.status.to_string(),
                                row.component_id,
                            )
                        })
                        .collect(),
                );
            }
            Rows::Null => self.rows = None,
            Rows::Keep => {}
        }
        let posted: Option<Vec<QuestStatusInput<'_>>> = self.rows.as_ref().map(|rows| {
            rows.iter()
                .map(|(name, status, component_id)| QuestStatusInput {
                    name,
                    status,
                    component_id: *component_id,
                })
                .collect()
        });
        post_page(
            &self.iso,
            &mut self.buf,
            &mut self.fingerprint,
            page,
            posted.as_deref(),
        );
    }

    /// A tick has to finish before the host forwards its interact batch; the
    /// probe is the barrier (the command loop is FIFO).
    fn barrier(&self) {
        let _ = self.iso.probe("true");
    }

    fn run(&self, method: &str, input: Json) -> Json {
        let expr = format!("globalThis.__questRun({:?}, 1, {input})", method);
        self.iso.probe(&expr).unwrap()
    }

    fn run_bare(&self, method: &str) -> Json {
        let expr = format!("globalThis.__questRun({:?}, 0, null)", method);
        self.iso.probe(&expr).unwrap()
    }

    fn begin(&self, name: &str) -> Json {
        self.run("questJournalBegin", json!({ "name": name }))
    }

    fn next(&self, token: u64) -> Json {
        self.run("questJournalNext", json!({ "token": token }))
    }

    fn close(&self, token: u64) -> Json {
        self.run("questJournalClose", json!({ "token": token }))
    }
}

/// A refusal is the public error and no verb.
fn assert_error(result: &Json, error: &str) {
    assert_eq!(
        result["out"],
        json!({ "ok": false, "error": error }),
        "{result}"
    );
    assert_eq!(result["queued"], json!([]), "a refusal emits no verb");
}

/// One `if-button` on the posted row id, no verb kind handed to the caller.
fn assert_if_button(result: &Json, component_id: i64) -> u64 {
    assert_eq!(result["out"]["ok"], true, "{result}");
    assert!(
        result["out"].get("kind").is_none(),
        "the caller never sees a verb kind: {result}"
    );
    assert_eq!(
        result["queued"],
        json!([{ "op": "if-button", "component_id": component_id }]),
        "{result}"
    );
    result["out"]["value"]["token"]
        .as_u64()
        .expect("begin token")
}

#[test]
fn begin_clicks_the_posted_row_id_and_the_later_pair_is_acquired() {
    let mut j = Journal::new();
    let rows = [
        row("Quest Journal", "unknown", Some(6)),
        row("Cook's Assistant", "notStarted", Some(1234)),
    ];
    j.post(Post::at(1).rows(&rows).closed_pair());

    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);

    // The pair is still the closed start: wait, never empty lines.
    let waiting = j.next(token);
    assert_eq!(waiting["out"], json!({ "pending": true }), "{waiting}");
    assert!(
        waiting["out"].get("ok").is_none(),
        "not-done has no ok field: {waiting}"
    );
    assert_eq!(waiting["queued"], json!([]));

    // A later pair whose root is not -1 is this token's journal. The modal
    // title is not the row name and is not compared, matched or stripped.
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    let acquired = j.next(token);
    assert_eq!(acquired["out"]["ok"], true, "{acquired}");
    assert_eq!(
        acquired["out"]["value"],
        json!({ "lines": ["@dre@The Cook's Quest"], "root": 77, "as_of_sequence": 2 }),
        "{acquired}"
    );
    assert_eq!(acquired["queued"], json!([]));

    // One close-modal while the latest pair is still the acquired root.
    let closing = j.close(token);
    assert_eq!(closing["out"], json!({ "pending": true }), "{closing}");
    assert_eq!(
        closing["queued"],
        json!([{ "op": "close-modal" }]),
        "{closing}"
    );

    // Still open: a second close emits nothing.
    let again = j.close(token);
    assert_eq!(again["out"], json!({ "pending": true }));
    assert_eq!(again["queued"], json!([]), "one close-modal per token");

    // The explicit closed pair is the only success, and Close never returns
    // journal lines.
    let empty: Vec<String> = Vec::new();
    j.post(Post::at(3).pair(-1, &empty));
    let closed = j.close(token);
    assert_eq!(
        closed["out"],
        json!({ "ok": true, "value": { "closed": true, "as_of_sequence": 3 } }),
        "{closed}"
    );
    assert_eq!(closed["queued"], json!([]));

    // The token is spent.
    assert_error(&j.close(token), "stale");
    j.iso.join();
}

#[test]
fn the_tick_path_forwards_the_single_if_button_verb() {
    let mut j = Journal::with_script(TICK_SCRIPT);
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());

    let call = json!({
        "method": "questJournalBegin",
        "input": { "name": "Cook's Assistant" },
    });
    let _ = j.iso.probe(&format!("globalThis.__questCall = {call}"));
    j.iso.on_game_tick(1);
    j.barrier();

    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }],
        "one if-button on the posted row id, and nothing else"
    );
    let out = j.iso.probe("globalThis.__questOut").unwrap();
    assert_eq!(out["ok"], true, "{out}");
    assert!(
        out.get("kind").is_none(),
        "no verb kind reaches the caller: {out}"
    );
    j.iso.join();
}

#[test]
fn begin_refuses_bad_args_before_the_page() {
    let j = Journal::new();
    // No page has ever been posted, so every one of these is an arg refusal.
    assert_error(&j.run_bare("questJournalBegin"), "invalid-args");
    assert_error(&j.run("questJournalBegin", json!({})), "invalid-args");
    assert_error(
        &j.run("questJournalBegin", json!({ "name": 7 })),
        "invalid-args",
    );
    assert_error(
        &j.run("questJournalBegin", json!("Cook's Assistant")),
        "invalid-args",
    );
    assert_error(
        &j.run("questJournalBegin", json!(["Cook's Assistant"])),
        "invalid-args",
    );
    assert_error(
        &j.run("questJournalBegin", json!({ "name": "   " })),
        "invalid-args",
    );
    // A present id is not a begin key, even beside a legal name.
    assert_error(
        &j.run(
            "questJournalBegin",
            json!({ "name": "Cook's Assistant", "id": "cook" }),
        ),
        "invalid-args",
    );
    assert_error(
        &j.run("questJournalBegin", json!({ "id": "cook" })),
        "invalid-args",
    );
    assert_error(&j.run("questJournalNext", json!({})), "invalid-args");
    assert_error(
        &j.run("questJournalNext", json!({ "token": "1" })),
        "invalid-args",
    );
    assert_error(
        &j.run("questJournalClose", json!({ "token": 1.5 })),
        "invalid-args",
    );
    // Args were fine here: a missing page is the page error, not invalid-args.
    assert_error(&j.next(1), "snapshot-unavailable");
    assert_error(&j.close(1), "snapshot-unavailable");
    assert_error(&j.begin("Cook's Assistant"), "snapshot-unavailable");
    j.iso.join();
}

#[test]
fn begin_refuses_an_unbound_tab_and_a_name_the_tab_did_not_post() {
    let mut j = Journal::new();
    // A keyframe with no rows is the null tab: unbound, and it needs no
    // sequence.
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(1).null_tab().pair(77, &texts));
    assert_error(&j.begin("Cook's Assistant"), "quest-tab-unbound");

    let rows = [
        row("Waterfall Quest", "notStarted", Some(42)),
        row("Quest Journal", "unknown", None),
    ];
    j.post(Post::at(2).rows(&rows).closed_pair());
    // The six seeds are not an allowlist: Death Plateau is in the identity
    // table and was not posted by this tab.
    assert_error(&j.begin("Death Plateau"), "unknown-quest");
    assert_error(&j.begin("Missing Quest"), "unknown-quest");

    // A posted row that is not one of the six seeds is this click.
    assert_if_button(&j.begin("waterfall quest"), 42);
    j.iso.join();
}

#[test]
fn an_omitted_pair_is_not_a_free_modal() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows));

    // The first post that omits slot 248 writes no property: slot 68 defaults
    // to -1 and that default is not the pair.
    let page = j
        .iso
        .probe(
            "({ pair: Object.prototype.hasOwnProperty.call(globalThis.__rs2b0t_host.snapshot, \
             'main_modal_texts'), main: globalThis.__rs2b0t_host.snapshot.main_modal_id })",
        )
        .unwrap();
    assert_eq!(page, json!({ "pair": false, "main": -1 }), "{page}");
    assert_error(&j.begin("Cook's Assistant"), "snapshot-unavailable");

    // Only the explicit closed pair frees the start.
    j.post(Post::at(2).rows(&rows).closed_pair());
    assert_if_button(&j.begin("Cook's Assistant"), 1234);
    j.iso.join();
}

#[test]
fn a_positive_pair_is_main_modal_occupied_and_a_closed_root_with_texts_is_unusable() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    // The title of the modal is the row name here, on purpose: a line that is
    // the row name is still not this token's journal.
    let texts = vec!["Cook's Assistant".to_string()];
    j.post(Post::at(1).rows(&rows).pair(77, &texts));
    assert_error(&j.begin("Cook's Assistant"), "main-modal-occupied");

    // Root -1 with non-empty texts is neither free nor occupied.
    j.post(Post::at(2).rows(&rows).pair(-1, &texts));
    assert_error(&j.begin("Cook's Assistant"), "snapshot-unavailable");
    j.iso.join();
}

#[test]
fn an_omitted_component_id_enqueues_nothing_and_never_writes_zero() {
    let mut j = Journal::new();
    // The first legal match omits its id. A later row for the same name did
    // post `0`, and the scan must not continue to it.
    let rows = [
        row("Cook's Assistant", "notStarted", None),
        row("Cook's Assistant", "complete", Some(0)),
    ];
    j.post(Post::at(1).rows(&rows).closed_pair());
    assert_error(&j.begin("Cook's Assistant"), "snapshot-unavailable");

    // A widget whose text is the quest name is not the click target either.
    let widgets = [WidgetTextInput {
        component_id: 999,
        text: "Cook's Assistant",
    }];
    j.post(Post::at(2).rows(&rows).closed_pair().widgets(&widgets));
    assert_error(&j.begin("Cook's Assistant"), "snapshot-unavailable");

    // A posted 0 on the selected row is a real id and is clicked.
    let rows = [row("Cook's Assistant", "notStarted", Some(0))];
    j.post(Post::at(3).rows(&rows).closed_pair().widgets(&widgets));
    assert_if_button(&j.begin("Cook's Assistant"), 0);
    j.iso.join();
}

#[test]
fn close_before_a_positive_root_is_acquired_emits_nothing() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);

    assert_error(&j.close(token), "stale");
    // The token is dead: it is not success and it is not a close of slot 68.
    assert_error(&j.next(token), "stale");
    j.iso.join();
}

#[test]
fn a_different_positive_root_is_stale_and_never_closed() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    assert_eq!(j.next(token)["out"]["ok"], true);

    let closing = j.close(token);
    assert_eq!(
        closing["queued"],
        json!([{ "op": "close-modal" }]),
        "{closing}"
    );

    // A replacement root is not success and is not closed.
    let other = vec!["@dre@Some Other Quest".to_string()];
    j.post(Post::at(3).pair(3824, &other));
    assert_error(&j.close(token), "stale");
    assert_error(&j.next(token), "stale");
    j.iso.join();
}

#[test]
fn a_same_id_co_post_is_not_a_close_and_changed_texts_are_stale() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    assert_eq!(j.next(token)["out"]["ok"], true);
    assert_eq!(
        j.close(token)["queued"],
        json!([{ "op": "close-modal" }]),
        "one close-modal for the acquired root"
    );

    // The same id is re-posted with the same texts: still open, not success,
    // and no second close. The pair is written again on purpose (a full post),
    // so this is a co-post and not an omitted slot.
    j.post(Post::at(3).full().pair(77, &texts));
    let still_open = j.close(token);
    assert_eq!(
        still_open["out"],
        json!({ "pending": true }),
        "{still_open}"
    );
    assert_eq!(still_open["queued"], json!([]));

    // The same root with different texts is a same-interface replacement.
    let moved = vec![
        "@dre@The Cook's Quest".to_string(),
        "@dre@line two".to_string(),
    ];
    j.post(Post::at(4).full().pair(77, &moved));
    assert_error(&j.close(token), "stale");
    j.iso.join();
}

#[test]
fn a_generation_bump_and_on_reset_abort_the_token_and_emit_nothing() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    assert_eq!(j.next(token)["out"]["ok"], true);
    assert_eq!(j.close(token)["queued"], json!([{ "op": "close-modal" }]));

    // Accepted v2 difference (F13 r2 R2-2): ResetSession clears the Rust
    // scene, so Next/Close here are snapshot-unavailable rather than stale.
    // The token is still dead and nothing is queued. Recorded in the changelog.
    j.iso.reset_session_work();
    assert_error(&j.close(token), "snapshot-unavailable");
    assert_error(&j.next(token), "snapshot-unavailable");
    j.iso.join();
}

#[test]
fn a_still_closed_pair_times_out_as_modal_timeout_without_a_verb() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);

    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    assert_error(&j.next(token), "modal-timeout");
    // The token is spent; the timeout is not the modals `timeout` reason and
    // it is not empty lines.
    assert_error(&j.next(token), "stale");
    j.iso.join();
}

#[test]
fn frozen_pause_and_hold_emit_no_verb_and_do_not_burn_the_window() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);

    j.iso.pause();
    let paused = j.next(token);
    assert_eq!(paused["out"], json!({ "pending": true }), "{paused}");
    assert_eq!(paused["queued"], json!([]));
    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    let paused_later = j.next(token);
    assert_eq!(
        paused_later["out"],
        json!({ "pending": true }),
        "a frozen clock does not advance the window: {paused_later}"
    );
    j.iso.resume();

    j.post(Post::at(2).rows(&rows).closed_pair().hold(true));
    let held = j.next(token);
    assert_eq!(held["out"], json!({ "pending": true }), "{held}");
    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    let held_later = j.next(token);
    assert_eq!(
        held_later["out"],
        json!({ "pending": true }),
        "a held clock does not advance the window: {held_later}"
    );
    j.post(Post::at(3).rows(&rows).closed_pair().hold(false));

    // Thawed: the same call is now the journal work, not a freeze.
    let thawed = j.next(token);
    assert_eq!(thawed["out"], json!({ "pending": true }), "{thawed}");
    j.iso.join();
}

#[test]
fn a_frozen_begin_does_not_click_and_starts_no_token() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.iso.pause();

    // No click, no armed window, no token.
    assert_error(&j.begin("Cook's Assistant"), "stale");
    j.iso.resume();

    // Thawed, the same call clicks once.
    assert_if_button(&j.begin("Cook's Assistant"), 1234);
    j.iso.join();
}

#[test]
fn a_same_name_begin_is_busy_and_another_name_cancels_without_a_close() {
    let mut j = Journal::new();
    let rows = [
        row("Cook's Assistant", "notStarted", Some(1234)),
        row("Waterfall Quest", "notStarted", Some(42)),
    ];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);

    // The same name (A-Z folded) is busy: no abort and no second click.
    assert_error(&j.begin("cook's assistant"), "busy");
    assert_eq!(j.next(token)["out"], json!({ "pending": true }));

    // Another name cancels the live token and emits no close for it: the queue
    // holds only the new click.
    let other = j.begin("Waterfall Quest");
    let other_token = assert_if_button(&other, 42);
    assert_ne!(other_token, token);
    assert_error(&j.next(token), "stale");
    j.iso.join();
}

#[test]
fn another_name_begin_of_an_occupied_pair_is_refused_and_cancels_the_old_token() {
    let mut j = Journal::new();
    let rows = [
        row("Cook's Assistant", "notStarted", Some(1234)),
        row("Waterfall Quest", "notStarted", Some(42)),
    ];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    // The click took effect: the pair is now this token's journal.
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    assert_eq!(j.next(token)["out"]["ok"], true);

    // Begin of another name cancels first, then refuses the occupied pair.
    // The refusal emits no close-modal for the cancelled token.
    assert_error(&j.begin("Waterfall Quest"), "main-modal-occupied");
    assert_error(&j.next(token), "stale");
    j.iso.join();
}

#[test]
fn the_owned_flow_never_calls_the_modals_machine() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let spy = j
        .iso
        .probe(
            "(function () { \
             const fns = globalThis.rustyscript && globalThis.rustyscript.functions; \
             if (!fns || typeof fns.__rs2b0t_modals !== 'function') return 'missing'; \
             const original = fns.__rs2b0t_modals; \
             globalThis.__modalsCalls = 0; \
             fns.__rs2b0t_modals = function (payload) { \
               globalThis.__modalsCalls += 1; \
               return original(payload); \
             }; \
             return fns.__rs2b0t_modals === original ? 'unwritable' : 'spied'; \
             })()",
        )
        .unwrap();
    assert_eq!(spy, json!("spied"), "the spy has to install: {spy}");

    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    assert_eq!(j.next(token)["out"]["ok"], true);
    assert_eq!(j.close(token)["queued"], json!([{ "op": "close-modal" }]));
    let empty: Vec<String> = Vec::new();
    j.post(Post::at(3).pair(-1, &empty));
    assert_eq!(j.close(token)["out"]["ok"], true);

    let calls = j.iso.probe("globalThis.__modalsCalls").unwrap();
    assert_eq!(
        calls,
        json!(0),
        "the journal machine does not route through the modals machine"
    );
    j.iso.join();
}

#[test]
fn the_page_fields_stay_off_api_snapshot() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let hidden = j
        .iso
        .probe(
            "({ quests: typeof globalThis.__rs_api.snapshot.quest_statuses, \
             pair: typeof globalThis.__rs_api.snapshot.main_modal_texts })",
        )
        .unwrap();
    assert_eq!(
        hidden,
        json!({ "quests": "undefined", "pair": "undefined" }),
        "the copy the machine reads stays off api.snapshot: {hidden}"
    );
    let raw = j
        .iso
        .probe(
            "({ quests: Array.isArray(globalThis.__rs2b0t_host.snapshot.quest_statuses), \
             pair: !!globalThis.__rs2b0t_host.snapshot.main_modal_texts })",
        )
        .unwrap();
    assert_eq!(raw, json!({ "quests": true, "pair": true }), "{raw}");
    j.iso.join();
}

#[test]
fn a_refused_begin_leaves_the_live_token_closable() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    assert_eq!(j.next(token)["out"]["ok"], true);

    assert_error(&j.begin("Missing Quest"), "unknown-quest");
    let closing = j.close(token);
    assert_eq!(closing["out"], json!({ "pending": true }), "{closing}");
    assert_eq!(
        closing["queued"],
        json!([{ "op": "close-modal" }]),
        "{closing}"
    );
    j.iso.join();
}

#[test]
fn a_wrong_token_on_an_unusable_pair_is_stale_and_kills_the_live_token() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    let token = assert_if_button(&j.begin("Cook's Assistant"), 1234);
    let junk = vec!["junk".to_string()];
    j.post(Post::at(2).pair(-1, &junk));
    assert_error(&j.next(token + 5), "stale");
    assert_error(&j.close(token + 5), "stale");
    assert_error(&j.next(token), "stale");
    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(3).pair(77, &texts));
    assert_error(&j.next(token), "stale");
    j.iso.join();
}
