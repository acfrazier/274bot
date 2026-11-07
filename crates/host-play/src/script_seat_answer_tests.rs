//! The native seat answer wake, end to end through the real observe adapter,
//! the real seat, the real page encoder and the real isolate. Every frame is
//! an unchanged host frame: no game tick and no changed fact follows the
//! first visit unless a test says so, so only the `SeatAnswer` class can
//! advance a callback (`docs/superpowers/adr/0017`, design section 8 S1).

use super::harness::HeldDriver;
use super::*;
use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
use script::slot::SeatProbe;
use std::collections::VecDeque;

const HERE: (i32, i32, i32) = (3091, 3958, 0);

/// One `depositAllMatching` whose predicate records every visit.
const KEEP_ALL: &str = r#"
        await Bank.depositAllMatching((name) => {
            globalThis.__matches.push(name);
            return false;
        });
        globalThis.__done = true;
"#;

/// The predicate throws an object on its second visit; the script keeps the
/// identity it saw.
const THROWS_ON_SECOND: &str = r#"
        try {
            await Bank.depositAllMatching((name) => {
                globalThis.__matches.push(name);
                if (globalThis.__matches.length === 2) {
                    globalThis.__thrown = { kind: 'b3' };
                    throw globalThis.__thrown;
                }
                return false;
            });
            globalThis.__result = { kind: 'done' };
        } catch (error) {
            globalThis.__result = { kind: 'threw', same: error === globalThis.__thrown };
        }
        globalThis.__done = true;
"#;

/// Two deposits over the same items, so a boundary in the first shows what
/// the second starts from.
const TWO_DEPOSITS: &str = r#"
        try {
            await Bank.depositAllMatching((name) => {
                globalThis.__matches.push(name);
                return false;
            });
            globalThis.__first = 'settled';
        } catch (error) {
            globalThis.__first = 'threw';
        }
        globalThis.__done = true;
        globalThis.__matches2 = [];
        await Bank.depositAllMatching((name) => {
            globalThis.__matches2.push(name);
            return false;
        });
        globalThis.__done2 = true;
"#;

/// How long the one-shot tick listener spins: past the isolate's 600 ms
/// execution horizon, so the dispatch counts as a slow tick.
const SPIN_MS: u32 = 1_200;

fn source(body: &str) -> String {
    format!(
        "
import {{ Bank }} from '../../api/bank/Bank.js';
import {{ BotHost }} from '../../runtime/BotHost.js';
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__started) return;
        globalThis.__started = true;
        globalThis.__matches = [];
        // One-shot: a script sets `__spin_next_tick` to make the next game
        // tick slow.
        BotHost.addTickListener(() => {{
            if (!globalThis.__spin_next_tick) return;
            globalThis.__spin_next_tick = false;
            globalThis.__spun = (globalThis.__spun ?? 0) + 1;
            const from = Date.now();
            while (Date.now() - from < {SPIN_MS}) {{}}
        }});
{body}
    }}
}}
"
    )
}

struct Frames {
    scripts: crate::ScriptWall,
    slot: Arc<Mutex<script::SlotScript>>,
    snapshot: GameSnapshot,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, super::super::NavBot>>>,
}

impl Frames {
    /// A started Load script and a bank snapshot whose side view holds
    /// `kept` distinct items, frozen for the whole test.
    fn new(body: &str, kept: usize) -> Self {
        let scripts: crate::ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let slot = crate::script_slot_or_insert(&scripts, "test");
        {
            let mut slot = slot.lock().unwrap();
            let start = crate::script_runtime::js_surface_start(
                crate::script_runtime::js_start_card(
                    source(body),
                    script_js::LoadShape::CompatClass,
                    vec![],
                ),
                None,
                Some(api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()),
                Arc::new(NamedBankFacts::from_banks(vec![])),
                Arc::from(Vec::<api::loadout::Loadout>::new()),
            );
            slot.start_surface_with_settings(start).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match slot.poll_start() {
                    script::slot::StartPoll::Settled(script::slot::StartOutcome::Ready) => break,
                    script::slot::StartPoll::Pending => {
                        assert!(Instant::now() < deadline, "Load start stalled");
                        std::thread::yield_now();
                    }
                    other => panic!("Load start failed: {other:?}"),
                }
            }
        }
        let side: Vec<_> = (0..kept)
            .map(|index| ItemView {
                def: api::ItemDefView {
                    id: 100 + index as i32,
                    name: Some(format!("Kept item {index}")),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::BankSide,
                action_family: ItemActionFamily::Component,
                slot: index as i32,
                count: 1,
                actions: vec![Some("Deposit-1".into()), Some("Deposit-All".into())],
                component_id: 701,
            })
            .collect();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_tick(1);
        snapshot.seed_tile(WorldTile {
            x: HERE.0,
            z: HERE.1,
            level: HERE.2,
        });
        snapshot.seed_inventory(
            side.iter()
                .cloned()
                .map(|mut item| {
                    item.container = ItemContainer::Inventory;
                    item.action_family = ItemActionFamily::Held;
                    item.actions.clear();
                    item.component_id = -1;
                    item
                })
                .collect(),
            kept.max(28) as i32,
        );
        snapshot.seed_bank_observation(600, 1, Some(vec![]), side);
        snapshot.seed_side_modal(700);
        let mut bot = super::super::NavBot::default();
        bot.bank_pick.facts = Some(Arc::new(NamedBankFacts::from_banks(vec![])));
        Self {
            scripts,
            slot,
            snapshot,
            cheats: Arc::new(Mutex::new(HashMap::new())),
            navs: Arc::new(Mutex::new(HashMap::from([("test".to_string(), bot)]))),
        }
    }

    /// One observe frame: `edge` is a PLAYER_INFO tick edge. The return is
    /// whether the frame dispatched script work.
    fn frame(&self, tick: u64, edge: bool) -> bool {
        crate::script_runtime::script_observe::script_observe(
            &mut HeldDriver,
            "test",
            true,
            edge,
            tick,
            Some(HERE),
            None,
            None,
            Some(&self.snapshot),
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &None,
            false,
            false,
        )
    }

    /// A frame whose chat family also changed: an evidence wake candidate.
    fn frame_with_evidence(&self, tick: u64, edge: bool) -> bool {
        crate::script_runtime::script_observe::script_observe_cached_with_channels(
            &mut HeldDriver,
            "test",
            true,
            edge,
            true,
            tick,
            Some(HERE),
            None,
            None,
            Some(&self.snapshot),
            None,
            None,
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &None,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            crate::script_channels::BrokerWorld::Unavailable,
            None,
            None,
            None,
            None,
        )
        .wrote
    }

    /// Evaluate on the isolate thread. It is a round trip, so it also waits
    /// for every command queued before it.
    fn probe(&self, expression: &str) -> serde_json::Value {
        self.slot.lock().unwrap().probe(expression).unwrap()
    }

    fn matches(&self) -> u64 {
        self.probe("globalThis.__matches?.length ?? 0")
            .as_u64()
            .unwrap()
    }

    fn done(&self) -> bool {
        self.probe("globalThis.__done === true") == serde_json::json!(true)
    }

    fn seat(&self) -> SeatProbe {
        self.slot.lock().unwrap().seat_probe().expect("armed seat")
    }

    fn iso<R>(&self, f: impl FnOnce(&script_js::LoadIsolate) -> R) -> R {
        let mut slot = self.slot.lock().unwrap();
        script_js::surface::with_isolate_for_test(&mut slot, f)
    }

    /// The machine host as the isolate thread sees it.
    fn host(&self) -> serde_json::Value {
        self.iso(script_js::LoadIsolate::machine_host_for_test)
    }

    fn settled(&self) -> Vec<serde_json::Value> {
        self.host()["settled"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Record every machine take, so a test can see which await received
    /// which outcome.
    fn record_takes(&self) {
        self.probe(
            "(() => { const take = globalThis.__rs2b0t_machine_take; globalThis.__takes = []; \
             globalThis.__rs2b0t_machine_take = (handle) => { const r = take(handle); \
             if (r !== undefined) globalThis.__takes.push({ handle, kind: r.kind, \
             value: r.value === undefined ? null : r.value }); return r; }; return true; })()",
        );
    }

    fn takes(&self) -> Vec<serde_json::Value> {
        self.probe("globalThis.__takes ?? []")
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Drive as the B3 regression does until the first predicate visit, and
    /// return the tick at which it happened.
    fn first_visit(&self) -> u64 {
        for _ in 0..4 {
            assert!(
                !self.frame(0, false),
                "no callback answer can wake before a tick"
            );
        }
        (1..=4)
            .find(|tick| {
                self.frame(*tick, true);
                for _ in 0..8 {
                    self.frame(*tick, false);
                    if self.matches() == 1 {
                        return true;
                    }
                }
                false
            })
            .expect("the real host starts the first bank predicate")
    }

    /// Unchanged frames at `tick` until `done` holds.
    fn run_until(&self, tick: u64, mut done: impl FnMut(&Self) -> bool) {
        for _ in 0..96 {
            if done(self) {
                return;
            }
            self.frame(tick, false);
            self.probe("true");
        }
        panic!(
            "unchanged frames did not reach the awaited state; matches {:?}, seat {:?}",
            self.probe("globalThis.__matches"),
            self.seat()
        );
    }

    fn run_to_done(&self, tick: u64) {
        self.run_until(tick, Self::done);
    }

    /// Leave exactly `depth` commands in the isolate's queue behind a blocked
    /// isolate thread, so that the next post and dispatch meet a full queue.
    fn fill_queue(&self, depth: usize) {
        self.iso(|iso| {
            while iso.queued_commands_for_test() < depth {
                iso.queue_noop_for_test();
            }
        });
    }

    fn names(count: usize) -> serde_json::Value {
        serde_json::json!((0..count)
            .map(|index| format!("Kept item {index}"))
            .collect::<Vec<_>>())
    }
}

fn five() -> serde_json::Value {
    Frames::names(5)
}

#[test]
fn observe_wake_matrix() {
    use crate::script_runtime::script_observe::{observe_wake, ObserveWake as W};
    // (tick_edge, tick, evidence, host results, answer owed, held) -> wake
    type Inputs = (bool, u64, bool, bool, bool, bool);
    let rows: &[(Inputs, W)] = &[
        ((true, 5, false, false, false, false), W::Tick),
        ((true, 5, true, true, true, true), W::Tick),
        ((true, 0, false, false, false, true), W::Tick),
        ((false, 0, true, true, true, false), W::None),
        ((false, 5, true, false, false, false), W::Evidence),
        ((false, 5, false, true, true, false), W::Evidence),
        ((false, 5, true, true, true, false), W::Evidence),
        ((false, 5, true, false, false, true), W::None),
        ((false, 5, false, false, true, false), W::SeatAnswer),
        ((false, 5, false, false, true, true), W::None),
        ((false, 5, false, false, false, false), W::None),
    ];
    for ((edge, tick, evidence, host, owed, held), expected) in rows.iter().copied() {
        assert_eq!(
            observe_wake(edge, tick, evidence, host, owed, held),
            expected,
            "edge {edge} tick {tick} evidence {evidence} host {host} owed {owed} held {held}"
        );
    }
}

#[test]
fn seat_answer_publishes_after_a_combined_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    // One quiet frame: the isolate's answer reaches the seat at its end.
    frames.frame(tick, false);
    frames.probe("true");
    // The next frame is a tick edge while that answer waits. Its poll takes
    // the answer and posts the next prompt after its own page was posted.
    frames.frame(tick + 1, true);
    frames.probe("true");
    frames.run_to_done(tick + 1);
    assert_eq!(frames.probe("globalThis.__matches"), five());
}

#[test]
fn seat_answer_publishes_after_a_combined_evidence_wake() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    frames.frame(tick, false);
    frames.probe("true");
    frames.frame_with_evidence(tick, false);
    frames.probe("true");
    frames.run_to_done(tick);
    assert_eq!(frames.probe("globalThis.__matches"), five());
}

#[test]
fn seat_answer_burst_keeps_tick_and_budget_domains() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    let game_tick = frames.probe("__rs2b0t_host.tick");
    let evidence_before = frames.iso(script_js::LoadIsolate::snapshot_evidence_sequence_for_test);
    let mut dispatches_seen = frames.seat().seat_pages.accepted;
    for _ in 0..96 {
        if frames.done() {
            break;
        }
        frames.frame(tick, false);
        frames.probe("true");
        for row in frames.host()["rows"].as_array().unwrap() {
            assert_eq!(
                row["calls"], 0,
                "the B3 predicate is synchronous: it never spends the callback budget"
            );
        }
        dispatches_seen = frames.seat().seat_pages.accepted;
    }
    assert!(frames.done());
    assert_eq!(
        frames.probe("__rs2b0t_host.tick"),
        game_tick,
        "no tick time advanced: Game.tick() is the tick of the first visit"
    );
    let evidence_after = frames.iso(script_js::LoadIsolate::snapshot_evidence_sequence_for_test);
    assert!(
    evidence_after - evidence_before <= u64::from(dispatches_seen),
    "only the encoded pages move the evidence sequence ({evidence_before} -> {evidence_after} over {dispatches_seen} dispatches); the seat page command adds none"
);
    assert!(
        !frames.frame(tick, false),
        "a completed bank has no callback wake"
    );
}

#[test]
fn seat_answer_burst_is_bounded_by_the_transition_allowance() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 40);
    let tick = frames.first_visit();
    let mut previous = frames.matches();
    let mut stalled = 0;
    for _ in 0..200 {
        frames.frame(tick, false);
        frames.probe("true");
        let now = frames.matches();
        stalled = if now == previous { stalled + 1 } else { 0 };
        previous = now;
        if stalled >= 12 {
            break;
        }
    }
    assert!(
    (5..=32).contains(&previous),
    "inside one tick the burst stops at the 32-transition allowance, not before 5 and not at 40: {previous}"
);
    assert!(!frames.done());
    frames.frame(tick + 1, true);
    frames.probe("true");
    frames.run_to_done(tick + 1);
    assert_eq!(frames.probe("globalThis.__matches"), Frames::names(40));
}

#[test]
fn seat_answer_wake_retries_when_the_dispatch_is_refused() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    frames.run_until(tick, |frames| frames.matches() >= 2);
    let before = frames.seat();
    let block = frames.iso(script_js::LoadIsolate::block_thread_for_test);
    frames.fill_queue(63);
    for _ in 0..8 {
        frames.frame(tick, false);
        if frames.seat().seat_pages.refused_dispatch == 1 {
            break;
        }
    }
    let refused = frames.seat();
    assert_eq!(refused.seat_pages.refused_dispatch, 1);
    assert_eq!(refused.seat_pages.refused_page, 0);
    assert_eq!(refused.seat_pages.accepted, before.seat_pages.accepted);
    assert_eq!(refused.untaken_answers, 0, "the poll took the answer");
    assert!(
        refused.answer_seq > refused.answer_dispatched,
        "the obligation stays open after a refused dispatch"
    );
    assert_eq!(refused.answer_dispatched, before.answer_dispatched);
    assert_eq!(refused.terminal_dispatched, before.terminal_dispatched);
    drop(block);
    frames.run_to_done(tick);
    assert_eq!(frames.probe("globalThis.__matches"), five());
    assert!(frames.seat().seat_pages.accepted > refused.seat_pages.accepted);
}

#[test]
fn seat_answer_wake_rearms_after_a_refused_page_post() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    frames.run_until(tick, |frames| frames.matches() >= 2);
    let block = frames.iso(script_js::LoadIsolate::block_thread_for_test);
    frames.fill_queue(64);
    for _ in 0..8 {
        frames.frame(tick, false);
        if frames.seat().seat_pages.refused_page == 1 {
            break;
        }
    }
    let refused = frames.seat();
    assert_eq!(refused.seat_pages.refused_page, 1);
    assert_eq!(refused.seat_pages.refused_dispatch, 0);
    assert!(refused.answer_seq > refused.answer_dispatched);
    drop(block);
    frames.run_to_done(tick);
    assert_eq!(frames.probe("globalThis.__matches"), five());
}

#[test]
fn seat_answer_final_terminal_survives_a_refused_dispatch() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    frames.record_takes();
    // Frozen frames until all five names were visited and the fifth answer
    // waits in the seat.
    frames.run_until(tick, |frames| {
        frames.matches() == 5 && frames.seat().untaken_answers == 1
    });
    let before = frames.seat();
    let block = frames.iso(script_js::LoadIsolate::block_thread_for_test);
    frames.fill_queue(63);
    // The poll takes the fifth answer, finishes the row and publishes its
    // terminal. The page is accepted and its dispatch is refused.
    frames.frame(tick, false);
    let refused = frames.seat();
    assert_eq!(refused.untaken_answers, 0);
    assert!(
        refused.result_seq > before.result_seq,
        "the terminal was published"
    );
    assert!(refused.answer_seq > before.answer_seq);
    assert_eq!(refused.answer_dispatched, before.answer_dispatched);
    assert_eq!(refused.terminal_dispatched, before.terminal_dispatched);
    assert_eq!(
        refused.seat_pages.refused_dispatch,
        before.seat_pages.refused_dispatch + 1
    );
    assert_eq!(
        refused.seat_pages.refused_page,
        before.seat_pages.refused_page
    );
    drop(block);
    let settled = frames.settled();
    assert_eq!(settled.len(), 1, "the page settled the row: {settled:?}");
    assert!(
        settled[0]["seat_seq"].as_u64().unwrap() > before.terminal_dispatched,
        "the entry carries a stamp above the floor the next dispatch uses"
    );
    assert!(!frames.done(), "no dispatch has released the await yet");
    let evidence = frames.iso(script_js::LoadIsolate::snapshot_evidence_sequence_for_test);
    // The retry: a seat answer frame with no untaken answer. Its page carries
    // no seat change, and its dispatch still releases the terminal.
    assert!(frames.frame(tick, false), "the obligation is still owed");
    frames.probe("true");
    assert!(frames.done(), "the retried dispatch released the terminal");
    assert_eq!(
        frames.iso(script_js::LoadIsolate::snapshot_evidence_sequence_for_test),
        evidence,
        "the retry page carried no change"
    );
    let after = frames.seat();
    assert_eq!(after.answer_dispatched, after.answer_seq);
    assert!(after.terminal_dispatched >= settled[0]["seat_seq"].as_u64().unwrap());
    let takes = frames.takes();
    assert_eq!(takes.len(), 1, "one take, once: {takes:?}");
    assert_eq!(takes[0]["kind"], "done", "not an abort: {takes:?}");
    for _ in 0..8 {
        assert!(
            !frames.frame(tick, false),
            "nothing is owed after completion"
        );
    }
    assert_eq!(frames.seat().seat_pages.accepted, after.seat_pages.accepted);
}

/// The JS handle list of a seat release exists only for a qualifying
/// terminal: over the five visits and the final terminal of one burst, the
/// shim's release entry point receives a list exactly once, and it holds the
/// one ended request.
#[test]
fn seat_release_materializes_its_handle_list_once_per_ended_request() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    let tick = frames.first_visit();
    frames.probe(
        "(() => { const settle = globalThis.__rs2b0t_settle_machines; globalThis.__lists = []; \
     globalThis.__rs2b0t_settle_machines = (...args) => { \
     globalThis.__lists.push(args.length === 0 ? null : args[0]); return settle(...args); }; \
     return true; })()",
    );
    let dispatched = frames.seat().seat_pages.accepted;
    frames.run_to_done(tick);
    assert!(
        frames.seat().seat_pages.accepted >= dispatched + 4,
        "the burst dispatched a page per remaining prompt and the terminal"
    );
    let lists = frames.probe("globalThis.__lists");
    let lists = lists.as_array().expect("recorded calls");
    let with_handles: Vec<_> = lists.iter().filter(|list| !list.is_null()).collect();
    assert_eq!(
        with_handles.len(),
        1,
        "one list for the one ended request: {lists:?}"
    );
    assert_eq!(with_handles[0].as_array().map(Vec::len), Some(1));
}

/// Sol N1 and Grok F1 through the whole host path: a slow game tick makes the
/// isolate thread skip the ticks queued behind it, and the seat page that
/// carries an accepted answer survives the skip. Every step goes through the
/// real observe adapter, the real page encoder and the real dispatch
/// accounting; nothing probes the isolate between the slow tick's start and
/// the last frame that queues work behind it, because a probe ends the stale
/// window.
#[test]
fn seat_answer_wake_survives_a_slow_tick_skip() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(KEEP_ALL, 5);
    frames.iso(script_js::LoadIsolate::suppress_execution_watchdog_for_test);
    let tick = frames.first_visit();
    assert_eq!(frames.matches(), 1, "one visit before the slow tick");
    frames.probe("globalThis.__spin_next_tick = true");
    let before = frames.seat();
    let ran = frames.iso(script_js::LoadIsolate::executed_seat_pages_for_test);

    // The slow tick starts on the tick edge and spins in its listener.
    assert!(frames.frame(tick + 1, true), "the tick edge dispatches");
    // Unchanged frames: the tick frame's poll took the answer and published
    // prompt 2 after its own page was posted, so the page is owed. It is
    // accepted while the thread still spins.
    let mut queued_behind_the_spin = 0;
    for _ in 0..8 {
        frames.frame(tick + 1, false);
        let seat = frames.seat();
        if seat.answer_dispatched == seat.answer_seq && seat.untaken_answers == 0 {
            queued_behind_the_spin = frames.iso(|iso| iso.queued_commands_for_test());
            break;
        }
    }
    let accepted = frames.seat();
    assert!(
        accepted.answer_seq > before.answer_seq,
        "the wake published the next prompt: {before:?} -> {accepted:?}"
    );
    assert_eq!(
        accepted.answer_dispatched, accepted.answer_seq,
        "the seat page was accepted: {accepted:?}"
    );
    assert_eq!(accepted.seat_pages.refused_page, 0);
    assert_eq!(accepted.seat_pages.refused_dispatch, 0);
    assert!(
        queued_behind_the_spin >= 1,
        "the accepted page was still queued behind the slow tick"
    );
    assert_eq!(
        frames.iso(script_js::LoadIsolate::executed_seat_pages_for_test),
        ran,
        "no new seat page has run: the thread is inside the slow tick"
    );

    // The next tick's snapshot and tick queue behind the page: the skip
    // drops that tick and must keep the page.
    assert!(frames.frame(tick + 2, true));
    assert_eq!(
        frames.iso(script_js::LoadIsolate::executed_seat_pages_for_test),
        ran,
        "still inside the slow tick"
    );

    // Only unchanged frames from here. The retained page runs after the slow
    // tick and the burst finishes with no tick edge and no changed fact.
    frames.run_to_done(tick + 2);
    assert_eq!(frames.probe("globalThis.__matches"), five());
    assert_eq!(
        frames.probe("globalThis.__spun"),
        serde_json::json!(1),
        "the listener spun exactly once"
    );
    let logs = frames.slot.lock().unwrap().take_pending_logs();
    let skipped = format!("skipped stale ticks -> {}", tick + 2);
    assert!(logs.contains(&skipped), "the skip ran: {logs:?}");
    assert_eq!(
        frames.probe("__rs2b0t_host.tick"),
        serde_json::json!(tick + 1),
        "the skipped tick never ran"
    );
    assert!(
        frames.iso(script_js::LoadIsolate::executed_seat_pages_for_test) > ran,
        "the retained page ran"
    );
    let after = frames.seat();
    assert_eq!(after.answer_dispatched, after.answer_seq);
    assert_eq!(after.seat_pages.refused_page, 0);
    assert_eq!(after.seat_pages.refused_dispatch, 0);
    for _ in 0..8 {
        assert!(
            !frames.frame(tick + 2, false),
            "nothing is owed after completion"
        );
    }
    assert_eq!(frames.seat().seat_pages.accepted, after.seat_pages.accepted);
}

#[test]
fn seat_answer_throwing_predicate_rejects_with_identity_on_frozen_frames() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(THROWS_ON_SECOND, 5);
    let tick = frames.first_visit();
    frames.run_until(tick, |frames| {
        frames.probe("globalThis.__result !== undefined") == serde_json::json!(true)
    });
    assert_eq!(
        frames.probe("globalThis.__result"),
        serde_json::json!({ "kind": "threw", "same": true }),
        "the rejection handler saw the thrown object itself, with no tick and no changed fact"
    );
    assert_eq!(frames.probe("__rs2b0t_host.tick"), serde_json::json!(tick));
    frames.run_to_done(tick);
    assert_eq!(
        frames.matches(),
        2,
        "the second visit threw: no third visit"
    );
}

/// Boundary scenarios: a kept reconnect, as production calls it.
fn kept_reconnect(frames: &Frames) -> script::slot::SeatProbe {
    let before = frames.seat();
    {
        let mut slot = frames.slot.lock().unwrap();
        assert!(slot.reconnect_session_work());
        slot.align_seat_session(None);
    }
    let held = frames.seat();
    assert!(held.held);
    assert_eq!(
        (
            held.answer_seq,
            held.answer_dispatched,
            held.terminal_dispatched,
            held.result_seq
        ),
        (
            before.answer_seq,
            before.answer_dispatched,
            before.terminal_dispatched,
            before.result_seq
        ),
        "a kept reconnect leaves the three counters and the result sequence alone"
    );
    held
}

fn reup(frames: &Frames) {
    let mut slot = frames.slot.lock().unwrap();
    slot.on_is_up(true);
    let session = api::identity::SessionKey::new(
        slot.slot_key_for_test().expect("slot key"),
        slot.work_epoch(),
    );
    slot.align_seat_session(Some(session));
}

#[test]
fn seat_answer_kept_reconnect_delivers_at_the_first_full_tick() {
    if !api::test_support::game_data_available() {
        return;
    } // (a) the boundary falls after the frame that posts prompt 3.
    {
        let frames = Frames::new(KEEP_ALL, 5);
        let tick = frames.first_visit();
        frames.record_takes();
        frames.run_until(tick, |frames| frames.matches() >= 2);
        let block = frames.iso(script_js::LoadIsolate::block_thread_for_test);
        let accepted = frames.seat().seat_pages.accepted;
        for _ in 0..8 {
            frames.frame(tick, false);
            if frames.seat().seat_pages.accepted > accepted {
                break;
            }
        }
        assert!(
            frames.seat().seat_pages.accepted > accepted,
            "prompt 3 was dispatched"
        );
        kept_reconnect(&frames);
        drop(block);
        frames.probe("true");
        assert_eq!(
            frames.matches(),
            2,
            "the old-generation dispatch ran nothing"
        );
        reup(&frames);
        for _ in 0..6 {
            frames.frame(tick, false);
            frames.probe("true");
        }
        assert_eq!(
            frames.matches(),
            2,
            "no hook runs before the first full tick"
        );
        assert!(!frames.done());
        frames.frame(tick + 1, true);
        frames.probe("true");
        frames.run_to_done(tick + 1);
        assert_eq!(
            frames.probe("globalThis.__matches"),
            Frames::names(3),
            "the retained prompt is delivered once, then the closed-bank exit"
        );
        let takes = frames.takes();
        assert_eq!(takes.len(), 1, "{takes:?}");
        assert_eq!(takes[0]["kind"], "done");
        assert_eq!(takes[0]["value"], serde_json::Value::Null);
        assert!(
            !frames.slot.lock().unwrap().has_native_actions(),
            "no old verb replayed and no new deposit verb was emitted"
        );
    }
    // (b) the boundary falls after the frame that publishes the terminal.
    {
        let frames = Frames::new(KEEP_ALL, 5);
        let tick = frames.first_visit();
        frames.record_takes();
        frames.run_until(tick, |frames| {
            frames.matches() == 5 && frames.seat().untaken_answers == 1
        });
        let block = frames.iso(script_js::LoadIsolate::block_thread_for_test);
        let before = frames.seat();
        frames.frame(tick, false);
        assert!(
            frames.seat().result_seq > before.result_seq,
            "terminal published"
        );
        assert!(
            frames.seat().seat_pages.accepted > before.seat_pages.accepted,
            "its dispatch was accepted before the boundary"
        );
        kept_reconnect(&frames);
        drop(block);
        assert!(!frames.done());
        reup(&frames);
        for _ in 0..6 {
            frames.frame(tick, false);
            frames.probe("true");
        }
        assert!(
            !frames.done(),
            "nothing releases before the first full tick"
        );
        frames.frame(tick + 1, true);
        frames.probe("true");
        frames.run_to_done(tick + 1);
        assert_eq!(frames.probe("globalThis.__matches"), five());
        let takes = frames.takes();
        assert_eq!(takes.len(), 1, "{takes:?}");
        assert_eq!(takes[0]["kind"], "done");
    }
    // (c) the boundary falls with the third answer untaken.
    {
        let frames = Frames::new(KEEP_ALL, 5);
        let tick = frames.first_visit();
        frames.record_takes();
        frames.run_until(tick, |frames| {
            frames.matches() == 3 && frames.seat().untaken_answers == 1
        });
        kept_reconnect(&frames);
        reup(&frames);
        for _ in 0..6 {
            frames.frame(tick, false);
            frames.probe("true");
        }
        frames.frame(tick + 1, true);
        frames.probe("true");
        frames.run_to_done(tick + 1);
        assert_eq!(frames.probe("globalThis.__matches"), Frames::names(3));
        let takes = frames.takes();
        assert_eq!(takes.len(), 1, "{takes:?}");
        assert_eq!(takes[0]["kind"], "done");
        assert_eq!(takes[0]["value"], serde_json::Value::Null);
        assert!(
            !frames.slot.lock().unwrap().has_native_actions(),
            "no old verb replayed and no new deposit verb was emitted"
        );
    }
}

#[test]
fn seat_answer_wake_owes_nothing_across_a_destructive_boundary() {
    if !api::test_support::game_data_available() {
        return;
    }
    let frames = Frames::new(TWO_DEPOSITS, 5);
    let tick = frames.first_visit();
    frames.run_until(tick, |frames| {
        frames.matches() == 5 && frames.seat().untaken_answers == 1
    });
    let block = frames.iso(script_js::LoadIsolate::block_thread_for_test);
    frames.frame(tick, false);
    assert!(frames.seat().result_seq > 0, "the terminal was published");
    // A non-kept reset: the production call for an operator logout.
    {
        let mut slot = frames.slot.lock().unwrap();
        slot.reset_session_work();
        slot.align_seat_session(None);
    }
    let reset = frames.seat();
    assert_eq!(reset.answer_dispatched, reset.answer_seq, "nothing is owed");
    assert_eq!(reset.terminal_dispatched, 0);
    assert_eq!(reset.result_seq, 0);
    assert_eq!(
        (reset.live, reset.results, reset.untaken_answers),
        (0, 0, 0)
    );
    drop(block);
    frames.probe("true");
    for entry in frames.settled() {
        assert_eq!(
            entry["seat_seq"], 0,
            "the reset zeroed every stamp: {entry:?}"
        );
    }
    reup(&frames);
    for _ in 0..6 {
        assert!(
            !frames.frame(tick, false),
            "no wake owed after the boundary"
        );
    }
    let after = frames.seat();
    assert_eq!(after.answer_seq, after.answer_dispatched);
}
