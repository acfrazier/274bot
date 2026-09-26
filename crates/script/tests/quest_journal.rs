//! `api.questJournalBegin` plus one awaited `api.questJournalRun`: one owned
//! token over the posted quest row and paired main-modal texts.
//!
//! Fixture-only. A posted click target is not a headed journal open: this is
//! not F-PROOF and this file runs no LIVE. The refusals are the point — no
//! click on an occupied or unusable pair, and exactly one Rust-owned
//! `if-button` / `close-modal` for a successful run.
use std::time::Duration;

use script::isolate_fb::{
    IsolateBuf, MainModalTextsInput, NativeFactsInput, QuestStatusInput, ReachViewInput,
    SnapshotFingerprint, SnapshotInput,
};
use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};
use serde_json::{json, Value as Json};

/// Past the machine's own 3000ms acquisition window.
const PAST_WINDOW_MS: u64 = 3_100;

const SCRIPT: &str = r#"
export const apiVersion = 2;
globalThis.__questRequest = null;
globalThis.__questStarted = false;
globalThis.__questBegin = null;
globalThis.__questOut = null;
export async function tick(api) {
  if (globalThis.__questStarted || !globalThis.__questRequest) return;
  globalThis.__questStarted = true;
  const request = globalThis.__questRequest;
  const begin = Object.prototype.hasOwnProperty.call(request, 'name')
    ? api.questJournalBegin({ name: request.name })
    : { ok: true, value: { token: request.token } };
  globalThis.__questBegin = begin;
  if (!begin.ok) {
    globalThis.__questOut = begin;
    return;
  }
  globalThis.__questOut = await api.questJournalRun(begin.value);
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
    /// Write every field instead of a delta: the pair is written again even
    /// when its content did not change (a same-id co-post).
    full: bool,
    hold: bool,
}

impl<'a> Post<'a> {
    fn at(tick: u64) -> Self {
        Self {
            tick,
            rows: Rows::Keep,
            pair: None,
            full: false,
            hold: false,
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

    fn full(mut self) -> Self {
        self.full = true;
        self
    }

    fn hold(mut self, hold: bool) -> Self {
        self.hold = hold;
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

    fn probe(&self, expression: &str) -> Json {
        self.iso.probe(expression).unwrap()
    }

    fn begin(&self, input: Json) -> Json {
        self.probe(&format!("globalThis.__rs_api.questJournalBegin({input})"))
    }

    fn begin_bare(&self) -> Json {
        self.probe("globalThis.__rs_api.questJournalBegin()")
    }

    fn request(&self, request: Json) {
        let _ = self.probe(&format!("globalThis.__questRequest = {request}; true"));
    }

    fn tick(&self, tick: u64) {
        self.iso.on_game_tick(tick);
        self.barrier();
    }

    fn begin_result(&self) -> Json {
        self.probe("globalThis.__questBegin")
    }

    fn outcome(&self) -> Json {
        self.probe("globalThis.__questOut")
    }
}

fn assert_helper_error(result: &Json, error: &str) {
    assert_eq!(result, &json!({ "ok": false, "error": error }), "{result}");
}

#[test]
fn one_await_owns_the_click_acquire_close_and_result() {
    let mut j = Journal::new();
    let rows = [
        row("Quest Journal", "unknown", Some(6)),
        row("Cook's Assistant", "notStarted", Some(1234)),
    ];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);

    let begin = j.begin_result();
    assert_eq!(begin["ok"], true, "{begin}");
    let token = begin["value"]["token"].as_u64().expect("begin token");
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }],
        "Rust emits the admitted row click in the starting tick"
    );
    assert!(j.outcome().is_null(), "the run still owns acquisition");

    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    j.tick(2);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::CloseModal],
        "acquiring the exact pair emits its one close"
    );
    assert!(j.outcome().is_null(), "the run waits for an observed close");

    // The same pair remains open: no second close.
    j.post(Post::at(3).pair(77, &texts).full());
    j.tick(3);
    assert!(
        j.iso.drain_interacts().is_empty(),
        "one close-modal per token"
    );
    assert!(j.outcome().is_null());

    let empty: Vec<String> = Vec::new();
    j.post(Post::at(4).pair(-1, &empty));
    j.tick(4);
    assert_eq!(
        j.outcome(),
        json!({
            "kind": "done",
            "value": {
                "kind": "done",
                "token": token,
                "lines": ["@dre@The Cook's Quest"],
                "root": 77,
                "as_of_sequence": 2,
                "closed_as_of_sequence": 4,
            }
        })
    );
    assert!(j.iso.drain_interacts().is_empty());
    j.iso.join();
}

#[test]
fn begin_is_sync_run_is_async_and_removed_pump_methods_stay_absent() {
    let mut j = Journal::new();
    let surface = j.probe(
        "({ \
          begin: typeof globalThis.__rs_api.questJournalBegin, \
          run: typeof globalThis.__rs_api.questJournalRun, \
          next: typeof globalThis.__rs_api.questJournalNext, \
          close: typeof globalThis.__rs_api.questJournalClose \
        })",
    );
    assert_eq!(
        surface,
        json!({
            "begin": "function",
            "run": "function",
            "next": "undefined",
            "close": "undefined",
        })
    );

    assert_helper_error(&j.begin_bare(), "invalid-args");
    for input in [
        json!({}),
        json!({ "name": 7 }),
        json!("Cook's Assistant"),
        json!(["Cook's Assistant"]),
        json!({ "name": "   " }),
        json!({ "name": "Cook's Assistant", "id": "cook" }),
        json!({ "id": "cook" }),
    ] {
        assert_helper_error(&j.begin(input), "invalid-args");
    }
    assert_helper_error(
        &j.begin(json!({ "name": "Cook's Assistant" })),
        "snapshot-unavailable",
    );

    let occupied = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(1).null_tab().pair(77, &occupied));
    assert_helper_error(
        &j.begin(json!({ "name": "Cook's Assistant" })),
        "quest-tab-unbound",
    );

    let rows = [row("Waterfall Quest", "notStarted", Some(42))];
    j.post(Post::at(2).rows(&rows).closed_pair());
    assert_helper_error(
        &j.begin(json!({ "name": "Cook's Assistant" })),
        "unknown-quest",
    );

    j.post(Post::at(3).rows(&rows).pair(77, &occupied));
    assert_helper_error(
        &j.begin(json!({ "name": "Waterfall Quest" })),
        "main-modal-occupied",
    );

    j.post(Post::at(4).rows(&rows).closed_pair());
    let admitted = j.begin(json!({ "name": "waterfall quest" }));
    assert_eq!(admitted["ok"], true, "{admitted}");
    assert_eq!(
        j.probe(
            "globalThis.__rs_api.questJournalRun({ token: -1 }).then(\
             value => { globalThis.__badJournalRun = value; }); true"
        ),
        json!(true)
    );
    assert_eq!(
        j.probe("globalThis.__badJournalRun"),
        json!({ "kind": "refused", "reason": "invalid-args" })
    );
    assert!(
        j.iso.drain_interacts().is_empty(),
        "begin and refusals do not enqueue; only the run may click"
    );
    j.iso.join();
}
#[test]
fn begin_requires_a_usable_pair_and_component_but_accepts_posted_zero() {
    let mut j = Journal::new();
    let usable = [row("Cook's Assistant", "notStarted", Some(1234))];

    // A default main-modal id is not the pair. The native pair itself must
    // have been posted.
    j.post(Post::at(1).rows(&usable));
    assert_helper_error(
        &j.begin(json!({ "name": "Cook's Assistant" })),
        "snapshot-unavailable",
    );

    // A closed root carrying text is internally inconsistent, not free.
    let orphan = vec!["orphaned modal text".to_string()];
    j.post(Post::at(2).rows(&usable).pair(-1, &orphan));
    assert_helper_error(
        &j.begin(json!({ "name": "Cook's Assistant" })),
        "snapshot-unavailable",
    );

    // The first matching row owns selection. If it omitted its component the
    // scan neither guesses zero nor falls through to a later duplicate.
    let missing = [
        row("Cook's Assistant", "notStarted", None),
        row("Cook's Assistant", "complete", Some(0)),
    ];
    j.post(Post::at(3).rows(&missing).closed_pair());
    assert_helper_error(
        &j.begin(json!({ "name": "Cook's Assistant" })),
        "snapshot-unavailable",
    );
    assert!(j.iso.drain_interacts().is_empty());

    // A zero that was actually posted on the selected row is a real component
    // and the awaited runner emits exactly that button id.
    let zero = [row("Cook's Assistant", "notStarted", Some(0))];
    j.post(Post::at(4).rows(&zero).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(4);
    assert_eq!(j.begin_result()["ok"], true);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 0 }]
    );
    assert!(j.outcome().is_null());
    j.iso.reset_session_work();
    j.tick(5);
    j.iso.join();
}

#[test]
fn a_frozen_begin_starts_no_token_or_click_then_thaws_normally() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.iso.pause();
    assert_helper_error(&j.begin(json!({ "name": "Cook's Assistant" })), "frozen");
    assert!(j.iso.drain_interacts().is_empty());

    j.iso.resume();
    j.post(Post::at(2).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(2);
    assert_eq!(j.begin_result()["ok"], true);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );
    j.iso.reset_session_work();
    j.tick(3);
    j.iso.join();
}

#[test]
fn replacement_modal_aborts_without_a_second_close() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );

    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    j.tick(2);
    assert_eq!(j.iso.drain_interacts(), vec![InteractReq::CloseModal]);

    let replacement = vec![
        "@dre@The Cook's Quest".to_string(),
        "@dre@replacement".to_string(),
    ];
    j.post(Post::at(3).pair(77, &replacement).full());
    j.tick(3);
    let outcome = j.outcome();
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["kind"], "aborted", "{outcome}");
    assert_eq!(outcome["value"]["reason"], "stale", "{outcome}");
    assert!(
        j.iso.drain_interacts().is_empty(),
        "a replacement pair is never closed"
    );
    j.iso.join();
}

#[test]
fn snapshot_unavailable_retries_within_the_window_then_closes() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );

    let unusable = vec!["orphaned modal text".to_string()];
    j.post(Post::at(2).pair(-1, &unusable));
    j.tick(2);
    assert!(
        j.outcome().is_null(),
        "an unusable acquisition frame is retried"
    );
    assert!(j.iso.drain_interacts().is_empty());

    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(3).pair(77, &texts));
    j.tick(3);
    assert_eq!(j.iso.drain_interacts(), vec![InteractReq::CloseModal]);
    assert!(j.outcome().is_null());

    let empty: Vec<String> = Vec::new();
    j.post(Post::at(4).pair(-1, &empty));
    j.tick(4);
    assert_eq!(j.outcome()["value"]["kind"], "done");
    assert!(j.iso.drain_interacts().is_empty());
    j.iso.join();
}

#[test]
fn closing_snapshot_unavailable_retries_then_recovers() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );

    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    j.tick(2);
    assert_eq!(j.iso.drain_interacts(), vec![InteractReq::CloseModal]);

    j.post(Post::at(3).full());
    j.tick(3);
    assert!(j.outcome().is_null(), "an omitted closing pair is retried");
    assert!(j.iso.drain_interacts().is_empty());

    let unusable = vec!["orphaned modal text".to_string()];
    j.post(Post::at(4).pair(-1, &unusable).full());
    j.tick(4);
    assert!(
        j.outcome().is_null(),
        "an inconsistent closing pair is retried"
    );
    assert!(j.iso.drain_interacts().is_empty());

    j.post(Post::at(5).closed_pair().full());
    j.tick(5);
    let outcome = j.outcome();
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["as_of_sequence"], 2, "{outcome}");
    assert_eq!(outcome["value"]["closed_as_of_sequence"], 5, "{outcome}");
    assert!(j.iso.drain_interacts().is_empty());
    j.iso.join();
}

#[test]
fn closing_timeout_settles_and_releases_the_journal() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );

    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(2).pair(77, &texts));
    j.tick(2);
    assert_eq!(j.iso.drain_interacts(), vec![InteractReq::CloseModal]);

    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    j.post(Post::at(3).full());
    j.tick(3);
    let outcome = j.outcome();
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["kind"], "aborted", "{outcome}");
    assert_eq!(outcome["value"]["reason"], "modal-timeout", "{outcome}");
    assert!(j.iso.drain_interacts().is_empty());

    j.post(Post::at(4).closed_pair().full());
    let _ = j.probe(
        "globalThis.__questStarted = false; \
         globalThis.__questOut = null; \
         globalThis.__questBegin = null; true",
    );
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(4);
    assert_eq!(j.begin_result()["ok"], true);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }],
        "the closing timeout released the journal"
    );
    j.iso.reset_session_work();
    j.tick(5);
    j.iso.join();
}

#[test]
fn acquisition_timeout_settles_the_run_without_closing() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );

    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    let unusable = vec!["orphaned modal text".to_string()];
    j.post(Post::at(2).pair(-1, &unusable));
    j.tick(2);
    let outcome = j.outcome();
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["kind"], "aborted", "{outcome}");
    assert_eq!(outcome["value"]["reason"], "modal-timeout", "{outcome}");
    assert!(j.iso.drain_interacts().is_empty());

    let empty: Vec<String> = Vec::new();
    j.post(Post::at(3).pair(-1, &empty));
    let _ = j.probe(
        "globalThis.__questStarted = false; \
         globalThis.__questOut = null; \
         globalThis.__questBegin = null; true",
    );
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(3);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }],
        "the timed-out token released the journal"
    );
    assert_eq!(j.begin_result()["ok"], true);
    j.iso.reset_session_work();
    j.tick(4);
    j.iso.join();
}

#[test]
fn pause_and_hold_freeze_the_acquisition_window_and_reset_aborts_the_await() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    j.request(json!({ "name": "Cook's Assistant" }));
    j.tick(1);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::IfButton { component_id: 1234 }]
    );

    j.iso.pause();
    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    j.tick(2);
    assert!(j.outcome().is_null(), "pause holds the machine");
    assert!(j.iso.drain_interacts().is_empty());
    j.iso.resume();

    j.post(Post::at(3).closed_pair().hold(true));
    j.tick(3);
    std::thread::sleep(Duration::from_millis(PAST_WINDOW_MS));
    j.post(Post::at(4).closed_pair().hold(true));
    j.tick(4);
    assert!(j.outcome().is_null(), "hold freezes the machine clock");
    assert!(j.iso.drain_interacts().is_empty());

    let texts = vec!["@dre@The Cook's Quest".to_string()];
    j.post(Post::at(5).pair(77, &texts).hold(false));
    j.tick(5);
    assert_eq!(
        j.iso.drain_interacts(),
        vec![InteractReq::CloseModal],
        "pause and hold did not consume the acquisition window"
    );

    j.iso.reset_session_work();
    j.tick(6);
    assert_eq!(j.outcome(), json!({ "kind": "aborted", "reason": "reset" }));
    assert!(j.iso.drain_interacts().is_empty());
    j.iso.join();
}

#[test]
fn journal_pages_stay_off_the_public_snapshot() {
    let mut j = Journal::new();
    let rows = [row("Cook's Assistant", "notStarted", Some(1234))];
    j.post(Post::at(1).rows(&rows).closed_pair());
    assert_eq!(
        j.probe(
            "({ quests: typeof globalThis.__rs_api.snapshot.quest_statuses, \
               pair: typeof globalThis.__rs_api.snapshot.main_modal_texts })"
        ),
        json!({ "quests": "undefined", "pair": "undefined" })
    );
    assert_eq!(
        j.probe(
            "({ quests: Array.isArray(globalThis.__rs2b0t_host.snapshot.quest_statuses), \
               pair: !!globalThis.__rs2b0t_host.snapshot.main_modal_texts })"
        ),
        json!({ "quests": true, "pair": true })
    );
    j.iso.join();
}
