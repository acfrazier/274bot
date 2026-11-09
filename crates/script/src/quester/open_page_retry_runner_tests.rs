//! A step that fails while its dialogue page is still open retries by
//! continuing that page from the current tile, not by walking to its anchor.
//! Merlin's Crystal teleports the player inside the page's script
//! (`quest_arthur.rs2:11-14`), so the authored anchor is far away while the
//! page stays open.
use super::tests::status_fixture;
use super::*;
use crate::interact::InteractReq;
use crate::native::{HostEffect, NativeOutput, WalkEnd, WalkReceipt};
use crate::quester::families::tests::{local_player, with_tick_output};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, WorldTile};
use std::collections::HashSet;

#[derive(Default)]
struct Capture(Vec<String>);

impl NativeOutput for Capture {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.0.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

type TestLedger = Option<Box<crate::native::ledger::Ledger>>;

const PLAYER: WorldTile = WorldTile {
    x: 2778,
    z: 9839,
    level: 0,
};

/// The Merlin's Crystal stage-3 step, as `arthur.json` authors it.
fn crate_document() -> super::super::path::PathDocument {
    use super::super::path::{PredicateDocument, StepDocument};

    let mut document = super::super::compile::decode_cook().unwrap();
    document.roles[0].sequences[0].steps = vec![StepDocument {
        id: FactKey::new("hide-in-delivery"),
        kind: "interact".to_owned(),
        version: 1,
        args: serde_json::json!({
            "target": {"loc": "merlincrate_empty"},
            "op": "Hide-in",
            "anchor": {"tile": [2801, 3442, 0], "source": "open page retry fixture"},
            "radius": 8,
            "dialogue": {"prefer": ["Yes."], "strict": true}
        }),
        comment: None,
        advances: Some(false),
        max_attempts: None,
        stop: None,
        skip_if: PredicateDocument::Any(vec![]),
        settle: PredicateDocument::Any(vec![]),
        hazard: None,
    }];
    document
}

fn open_page(snapshot: &mut GameSnapshot) {
    snapshot.seed_local_player(local_player(PLAYER));
    snapshot.seed_chat_modal(100, vec!["And wait...".to_owned()]);
    snapshot.seed_chat_options(vec![], 101);
}

/// Request ids of the effects the script has put in the ledger so far.
#[derive(Default)]
struct Requests {
    walks: HashSet<u64>,
    continues: HashSet<u64>,
}

impl Requests {
    fn observe(&mut self, ledger: &TestLedger) {
        for entry in &ledger.as_ref().unwrap().outbox {
            match &entry.effect {
                HostEffect::Walk(_) => {
                    self.walks.insert(entry.request_id.get());
                }
                HostEffect::Interaction(InteractReq::ContinueDialog { .. }) => {
                    self.continues.insert(entry.request_id.get());
                }
                _ => {}
            }
        }
    }
}

fn fail_walk(ledger: &mut TestLedger, tick: u64) {
    let request_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find_map(|entry| {
            matches!(&entry.effect, HostEffect::Walk(_)).then(|| entry.request_id.get())
        })
        .expect("the first attempt requested a walk");
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        request_id,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick,
            sequence: tick,
        },
        goal: None,
        end: WalkEnd::Cancelled,
        blocked: None,
        detail: Some(Arc::from("open page retry fixture")),
        refusal: None,
        assessment: None,
        escape: None,
        prereq_allow: None,
    });
}

struct Fixture {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: TestLedger,
    output: Capture,
    requests: Requests,
    tick: u64,
}

impl Fixture {
    fn new(page_open: bool) -> Self {
        let (script, mut snapshot) = status_fixture(crate_document());
        if page_open {
            open_page(&mut snapshot);
        } else {
            snapshot.seed_local_player(local_player(PLAYER));
            snapshot.seed_chat_modal(-1, vec![]);
            snapshot.seed_chat_options(vec![], -1);
        }
        Self {
            script,
            snapshot,
            ledger: None,
            output: Capture::default(),
            requests: Requests::default(),
            tick: 0,
        }
    }

    fn step(&mut self) {
        self.tick += 1;
        with_tick_output(
            &self.snapshot,
            &mut self.ledger,
            self.tick,
            &mut self.output,
            |native| {
                self.script.tick(native).unwrap();
            },
        );
        self.requests.observe(&self.ledger);
    }

    /// Attempt one walks toward the anchor, because the content's tile is far
    /// from it. The walk ends with an ordinary step error (a route failure
    /// would park the step), so the attempt fails with whatever page is open
    /// still open and the runner retries the step.
    fn fail_first_attempt_walk(&mut self) {
        for _ in 0..10 {
            self.step();
            if !self.requests.walks.is_empty() {
                break;
            }
        }
        assert_eq!(self.requests.walks.len(), 1, "{:?}", self.output.0);
        fail_walk(&mut self.ledger, self.tick);
    }

    fn run_until(&mut self, ticks: u64, done: impl Fn(&Self) -> bool) {
        for _ in 0..ticks {
            if done(self) {
                return;
            }
            self.step();
        }
    }
}

#[test]
fn a_retry_with_the_dialogue_page_open_continues_it_without_a_walk() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fixture = Fixture::new(true);
    fixture.fail_first_attempt_walk();
    assert!(fixture.requests.continues.is_empty());
    fixture.run_until(30, |fixture| !fixture.requests.continues.is_empty());
    let logs = format!("{:?}", fixture.output.0);

    assert_eq!(
        fixture.requests.walks.len(),
        1,
        "the retry must not request another walk to the anchor: {logs}"
    );
    assert!(
        !fixture.requests.continues.is_empty(),
        "the retry must continue the open page: {logs}"
    );
    assert_eq!(
        fixture.script.fail_streak, 1,
        "the retry keeps the failed attempt on the bounded budget: {logs}"
    );

    // Nothing acknowledges the Continue in this fixture, so the continued
    // page fails by its tick bound. The failure is the second on the budget
    // and its reason is the one that the status reports.
    fixture.run_until(40, |fixture| fixture.script.fail_streak >= 2);
    let logs = format!("{:?}", fixture.output.0);
    assert_eq!(fixture.script.fail_streak, 2, "{logs}");
    assert!(
        fixture
            .script
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("dialogue failed")),
        "the last failure detail is kept: {:?} {logs}",
        fixture.script.last_error
    );
    assert_eq!(fixture.requests.walks.len(), 1, "{logs}");
}

#[test]
fn a_retry_without_an_open_page_still_walks_to_the_anchor() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fixture = Fixture::new(false);
    fixture.fail_first_attempt_walk();
    fixture.run_until(30, |fixture| fixture.requests.walks.len() >= 2);
    let logs = format!("{:?}", fixture.output.0);
    assert!(
        fixture.requests.walks.len() >= 2,
        "with no page open, the retry walks to the anchor again: {logs}"
    );
    assert!(fixture.requests.continues.is_empty());
}
