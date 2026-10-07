use super::*;
use crate::native::{HostEffect, InteractionReceipt, NativeOutput};
use crate::quester::families::tests::{def, local_player, with_tick_output};
use crate::shim::InteractReq;
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
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.logs.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

struct CookDialogueFixture {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Option<Box<crate::native::ledger::Ledger>>,
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
        let flow = with_tick_output(
            &self.snapshot,
            &mut self.ledger,
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
                            self.snapshot.seed_chat_modal(4882, vec![]);
                            self.snapshot.seed_chat_options(vec![], 4883);
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
                HostEffect::Walk(_) => panic!("held Cook hand-in must not require a walk"),
                HostEffect::AssessWalk(_) => {
                    panic!("held Cook hand-in must not require a walk assessment")
                }
            }
        }
        flow
    }
}

#[test]
fn original_cook_hand_in_settles_after_owned_dialogue_transitions_to_completion_scroll() {
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
