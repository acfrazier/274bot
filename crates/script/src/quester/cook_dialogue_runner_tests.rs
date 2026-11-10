use super::*;
use crate::interact::InteractReq;
use crate::native::ledger::TickBudget;
use crate::native::{HostEffect, InteractionReceipt, NativeOutput};
use crate::quester::families::tests::{def, local_player, with_tick_budget_output};
use api::named_banks::{NamedBank, NamedBankFacts};
use api::quest_progress::EvidenceStamp;
use api::selected::ClientRevision;
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, NpcView, QuestStatusView, WorldTile,
};
use std::sync::Arc;

#[derive(Default)]
struct Capture {
    logs: Vec<String>,
}

impl NativeOutput for Capture {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.logs.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

struct CookDialogueFixture {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Option<Box<crate::native::ledger::Ledger>>,
    /// The slot's budget, which carries a guardian hold across ticks.
    budget: TickBudget,
    /// The host's reply to a Talk-to: the Cook's page opens. A guardian that
    /// took the player's interaction over leaves it unset.
    talk_opens_page: bool,
    output: Capture,
    completion_scroll_root: i32,
    cook_name: String,
    talks: usize,
    continues: usize,
    scroll_closes: usize,
    saw_completion_scroll: bool,
}

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

impl CookDialogueFixture {
    fn new() -> Self {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
        let document = super::super::compile::decode_cook().unwrap();
        let path = super::super::compile::compile_uncached_for_test(&document, &selected, &quests)
            .unwrap();
        let bank = NamedBank::new("Draynor Village", tile(3092, 3242));
        let script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&selected),
            quests,
            Arc::new(NamedBankFacts::from_banks(vec![bank])),
        );
        let cook = selected.npc_by_config("cook").unwrap();
        let cook_name = cook.display.as_deref().unwrap().to_owned();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(tile(3209, 3215)));
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf8f800,
            }],
            true,
        );
        snapshot.seed_inventory(
            [
                "egg",
                "bucket_milk",
                "pot_flour",
                "pot_empty",
                "grain",
                "bucket_empty",
            ]
            .into_iter()
            .enumerate()
            .map(|(slot, alias)| {
                let item = selected.item_by_alias(alias).unwrap();
                ItemView {
                    def: def(item.id, item.name.as_deref().unwrap()),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: slot as i32,
                    count: 1,
                    actions: Vec::new(),
                    component_id: 7,
                }
            })
            .collect(),
            28,
        );
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        snapshot.seed_main_modal(-1, vec![]);
        snapshot.seed_npcs(vec![NpcView {
            index: 42,
            r#type: Some(cook.id as usize),
            name: cook.display.clone(),
            actions: vec![Some("Talk-to".into())],
            tile: tile(3209, 3215),
            distance: 1,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 1,
            total_health: 1,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile(3209, 3215),
            x: 0,
            z: 0,
            yaw: 0,
        }]);
        Self {
            script,
            snapshot,
            ledger: None,
            budget: TickBudget::default(),
            talk_opens_page: true,
            output: Capture::default(),
            completion_scroll_root: selected.dialogue_ui().unwrap().quest_scroll_root,
            cook_name,
            talks: 0,
            continues: 0,
            scroll_closes: 0,
            saw_completion_scroll: false,
        }
    }

    fn tick(&mut self, tick: u64) -> ScriptFlow {
        let flow = with_tick_budget_output(
            &self.snapshot,
            &mut self.ledger,
            &mut self.budget,
            tick,
            &mut self.output,
            |native| self.script.tick(native).unwrap(),
        );
        let actions = self
            .ledger
            .as_mut()
            .map(|ledger| std::mem::take(&mut ledger.outbox))
            .unwrap_or_default();
        for action in actions {
            let authority = action.authority();
            let evidence = EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            };
            match action.effect {
                HostEffect::Interaction(request) => {
                    match request {
                        InteractReq::Npc { name, action, .. } => {
                            assert_eq!(name, self.cook_name);
                            assert_eq!(action, "Talk-to");
                            self.talks += 1;
                            if self.talk_opens_page {
                                self.snapshot.seed_chat_modal(4882, vec![]);
                                self.snapshot.seed_chat_options(vec![], 4883);
                            }
                        }
                        InteractReq::ContinueDialog { .. } => {
                            self.continues += 1;
                            self.snapshot.seed_chat_modal(-1, vec![]);
                            self.snapshot.seed_chat_options(vec![], -1);
                            self.snapshot
                                .seed_main_modal(self.completion_scroll_root, vec![]);
                            self.snapshot.seed_quest_statuses(
                                vec![QuestStatusView {
                                    name: "Cook's Assistant".into(),
                                    component_id: 42,
                                    colour: 0x00f800,
                                }],
                                true,
                            );
                            self.saw_completion_scroll = true;
                        }
                        InteractReq::CloseModal => {
                            assert_eq!(self.snapshot.modals().main, self.completion_scroll_root);
                            self.scroll_closes += 1;
                            self.snapshot.seed_main_modal(-1, vec![]);
                        }
                        unexpected => panic!("unexpected Cook hand-in interaction: {unexpected:?}"),
                    }
                    self.ledger.as_mut().unwrap().complete_interaction(
                        &authority,
                        InteractionReceipt {
                            request_id: authority.request_id().get(),
                            evidence,
                            accepted: true,
                            chat_since: 0,
                        },
                    );
                }
                HostEffect::BankPick(_) => panic!("held Cook items must not require a bank pick"),
                HostEffect::ServicePick(_) => {
                    panic!("held Cook items must not require a service pick")
                }
                HostEffect::Walk(_) => panic!("held Cook hand-in must not require a walk"),
                HostEffect::AssessWalk(_) => {
                    panic!("held Cook hand-in must not require a walk assessment")
                }
                HostEffect::ServerOrdered(_) => {
                    panic!("Cook hand-in must use ordinary interactions")
                }
            }
        }
        flow
    }
}

#[test]
fn original_cook_hand_in_settles_after_owned_dialogue_transitions_to_completion_scroll() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fixture = CookDialogueFixture::new();
    let mut flow = ScriptFlow::Continue;
    for tick in 1..=80 {
        flow = fixture.tick(tick);
        if flow == ScriptFlow::Complete {
            break;
        }
        assert!(
            matches!(flow, ScriptFlow::Continue),
            "Cook run stopped early: {flow:?}"
        );
    }

    assert_eq!(flow, ScriptFlow::Complete);
    assert!(
        fixture.saw_completion_scroll,
        "the selected quest-completion scroll must be observed"
    );
    assert_eq!(
        fixture.talks, 1,
        "the original Path must drive one Cook Talk-to"
    );
    assert_eq!(
        fixture.continues, 1,
        "the owned Cook chat page must be continued once"
    );
    assert_eq!(
        fixture.scroll_closes, 1,
        "the observed completion scroll must be closed by Dialogue"
    );
    assert!(fixture
        .output
        .logs
        .iter()
        .any(|line| { line.contains("quester cook: stage cook:2 step hand-in (talk) settled") }));
    assert!(fixture
        .output
        .logs
        .iter()
        .any(|line| line == "quester cook: stage cook:1 → cook:2"));
    assert!(fixture
        .output
        .logs
        .iter()
        .any(|line| line == "quester cook: finish"));
    assert!(!fixture.output.logs.iter().any(|line| {
        line.contains("quester cook: stage cook:1 step hand-in (talk) failed: dialogue failed")
    }));
}

/// QUESTER-READ-KEEPS-STEP-R2, finding R1, on the real compiled Cook path: a
/// random event takes the player's Talk-to over (the guardian's own
/// interaction replaces the one the server accepted), so the Cook's page never
/// opens. The hand-in must neither begin again (the ordered guide is not
/// restarted: one begin of the step, progress and cursor kept) nor fail at its
/// first-page deadline: it sends exactly one more Talk-to, after the settle
/// tick, and then completes as an ordinary hand-in.
#[test]
fn a_random_event_that_takes_the_hand_in_talk_over_resends_it_once_without_a_restart() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fixture = CookDialogueFixture::new();
    fixture.talk_opens_page = false;
    let mut tick = 1;
    while fixture.talks == 0 {
        assert_eq!(fixture.tick(tick), ScriptFlow::Continue);
        tick += 1;
        assert!(tick < 60, "the hand-in sends its Talk-to");
    }
    let begun_before = fixture.script.begun.len();
    let cursor_before = fixture.script.cursor;
    assert!(fixture.script.step.is_some());

    // The event arrives, the guardian holds the slot for eight ticks.
    let event = DetectedRandom {
        kind: api::random::RandomKind::Dialog,
        name: "genie".into(),
        ours: true,
        npc_index: Some(0),
    };
    fixture.script.on_random(&event);
    fixture.budget.hold_begin();
    fixture.budget.hold_end();
    tick += 8;
    // The settle tick sends nothing; the next sends one Talk-to; then the
    // host's reply opens the page this time.
    assert_eq!(fixture.tick(tick), ScriptFlow::Continue);
    assert_eq!(
        fixture.talks, 1,
        "the first observed tick after the hold sends nothing"
    );
    tick += 1;
    fixture.talk_opens_page = true;
    assert_eq!(fixture.tick(tick), ScriptFlow::Continue);
    assert_eq!(fixture.talks, 2, "exactly one Talk-to is sent again");
    assert_eq!(fixture.script.begun.len(), begun_before, "no new begin");
    assert_eq!(fixture.script.cursor, cursor_before);
    assert!(fixture.script.progress.is_some());

    let mut flow = ScriptFlow::Continue;
    for _ in 0..80 {
        tick += 1;
        flow = fixture.tick(tick);
        if flow == ScriptFlow::Complete {
            break;
        }
    }
    assert_eq!(flow, ScriptFlow::Complete, "the hand-in completes");
    assert_eq!(fixture.talks, 2, "no third Talk-to");
    assert!(!fixture
        .output
        .logs
        .iter()
        .any(|line| line.contains("failed")));
}

/// QUESTER-READ-KEEPS-STEP-R3, finding R1.2, on the real compiled Cook path: a
/// teleporting event takes the player out and back inside one observed tick
/// while the hand-in's Talk-to is in flight. The host's frames of that tick
/// show the start tile, the remote square and then the start tile again, so
/// the hold's ends show the same tile and every pair is zero ticks apart. The
/// budget keeps the teleport flag through the return and the end of the hold;
/// the runner takes it once, drops the in-flight step with progress and
/// cursor kept, and the hand-in starts again from its own checks: one new
/// Talk-to, and it completes.
#[test]
fn a_teleport_out_and_back_inside_one_tick_drops_the_talk_once_and_the_hand_in_recovers() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fixture = CookDialogueFixture::new();
    fixture.talk_opens_page = false;
    let start = tile(3209, 3215);
    let mut tick = 1;
    while fixture.talks == 0 {
        fixture.budget.observe_fix(tick, Some(start));
        assert_eq!(fixture.tick(tick), ScriptFlow::Continue);
        tick += 1;
        assert!(tick < 60, "the hand-in sends its Talk-to");
    }
    let begun_before = fixture.script.begun.len();
    let cursor_before = fixture.script.cursor;
    assert!(fixture.script.step.is_some());

    fixture.script.on_random(&DetectedRandom {
        kind: api::random::RandomKind::Maze,
        name: "maze".into(),
        ours: true,
        npc_index: None,
    });
    // One observed tick, three frames: the start tile before the hold, then
    // the Maze square and the start tile again inside it. All three carry the
    // same tick, so only the zero-elapsed comparison can see the teleport.
    fixture.budget.observe_fix(tick, Some(start));
    fixture.budget.hold_begin();
    fixture.budget.observe_fix(tick, Some(tile(2893, 4555)));
    fixture.budget.observe_fix(tick, Some(start));
    fixture.budget.hold_end();
    assert!(
        fixture.budget.hold_teleport_latched(),
        "the return inside the same tick does not clear the teleport"
    );

    tick += 1;
    fixture.talk_opens_page = true;
    fixture.budget.observe_fix(tick, Some(start));
    assert_eq!(fixture.tick(tick), ScriptFlow::Continue);
    assert!(
        !fixture.budget.hold_teleport_latched(),
        "the runner took the teleport"
    );
    // The old Talk is not resumed (that sends a Talk-to with no new begin,
    // as in the hold without a teleport above). The step is dropped and
    // begins again at the kept cursor, and its fresh Talk sends the Talk-to.
    assert_eq!(
        fixture.script.begun.len(),
        begun_before + 1,
        "the dropped step begins once more"
    );
    assert_eq!(fixture.talks, 2, "the new step's one Talk-to");
    assert_eq!(fixture.script.cursor, cursor_before, "the cursor stays");
    assert!(fixture.script.progress.is_some(), "progress stays");

    let mut flow = ScriptFlow::Continue;
    for _ in 0..80 {
        tick += 1;
        fixture.budget.observe_fix(tick, Some(start));
        flow = fixture.tick(tick);
        if flow == ScriptFlow::Complete {
            break;
        }
    }
    assert_eq!(flow, ScriptFlow::Complete, "the hand-in completes");
    assert_eq!(fixture.talks, 2, "no further Talk-to");
    assert_eq!(
        fixture.script.begun.len(),
        begun_before + 1,
        "no further begin: the teleport is taken once"
    );
    assert!(!fixture
        .output
        .logs
        .iter()
        .any(|line| line.contains("failed")));
}
