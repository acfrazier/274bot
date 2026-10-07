use super::*;
use crate::native::{HostEffect, InteractionReceipt, WalkEnd, WalkReceipt};
use crate::quester::families::tests::{def, local_player, with_tick_output, with_tick_output_bank};
use crate::shim::InteractReq;
use api::named_banks::{NamedBank, NamedBankFacts};
use api::snapshot::{
    stat_name, stat_used, GameSnapshot, GroundItemView, ItemActionFamily, ItemContainer, ItemView,
    LocLayer, LocView, QuestStatusView, StatView, WorldTile,
};

#[derive(Default)]
struct Capture {
    logs: Vec<String>,
}
impl NativeOutput for Capture {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, line: &str) {
        self.logs.push(line.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

struct CookFixture {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Option<Box<crate::native::ledger::Ledger>>,
    /// The account's bank memory, filled the way the host fills it.
    bank: api::bank_memory::BankMemory,
    bank_fixture: NamedBank,
    stock: Vec<ItemView>,
    output: Capture,
    visits: usize,
    withdrawals: usize,
    deposits: usize,
    egg_taken: bool,
}

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

impl CookFixture {
    fn new(held: bool, banked: bool) -> Self {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
        // Compile the shipped document byte-for-byte: no owns_inventory override,
        // shortened recipes, substituted StepRuns, or injected bank memo.
        let document = super::super::compile::decode_cook().unwrap();
        let path = super::super::compile::compile_uncached_for_test(&document, &selected, &quests)
            .unwrap();
        let bank = NamedBank::new("Draynor Village", tile(3092, 3242));
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(tile(3209, 3215)));
        snapshot.seed_stats(
            (0..25)
                .filter(|index| stat_used(*index))
                .map(|index| StatView {
                    index: index as i32,
                    name: stat_name(index).to_owned(),
                    effective: 99,
                    base: 99,
                    xp: 13_034_431,
                    used: true,
                })
                .collect(),
        );
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf80000,
            }],
            true,
        );
        let script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            selected,
            quests,
            Arc::new(NamedBankFacts::from_banks(vec![bank])),
        );
        let mut fixture = Self {
            script,
            snapshot,
            ledger: None,
            bank: api::bank_memory::BankMemory::default(),
            bank_fixture: bank,
            stock: Vec::new(),
            output: Capture::default(),
            visits: 0,
            withdrawals: 0,
            deposits: 0,
            egg_taken: false,
        };
        let ingredients = ["egg", "bucket_milk", "pot_flour"];
        fixture.snapshot.seed_inventory(
            if held {
                ingredients
                    .iter()
                    .enumerate()
                    .map(|(slot, alias)| fixture.item(alias, slot as i32, ItemContainer::Inventory))
                    .collect()
            } else {
                Vec::new()
            },
            28,
        );
        if banked {
            fixture.stock = ingredients
                .iter()
                .enumerate()
                .map(|(slot, alias)| fixture.item(alias, slot as i32, ItemContainer::Bank))
                .collect();
        }
        fixture.snapshot.seed_locs(vec![LocView {
            id: 2213,
            name: Some("Bank booth".into()),
            actions: vec![Some("Use-quickly".into())],
            tile: bank.tile,
            distance: 0,
            typecode: 0,
            info: 0,
            description: None,
            layer: LocLayer::GroundDecoration,
            shape: 0,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
        fixture
    }

    fn item(&self, alias: &str, slot: i32, container: ItemContainer) -> ItemView {
        let item = self.script.selected.item_by_alias(alias).unwrap();
        ItemView {
            def: def(item.id, item.name.as_deref().unwrap()),
            container,
            action_family: if container == ItemContainer::Bank {
                ItemActionFamily::Component
            } else {
                ItemActionFamily::Held
            },
            slot,
            count: 1,
            actions: if container == ItemContainer::Bank {
                vec![Some("Withdraw-X".into()), Some("Withdraw-1".into())]
            } else {
                Vec::new()
            },
            component_id: 7,
        }
    }
    fn seed_in_progress(&mut self) {
        self.snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0xf8f800,
            }],
            true,
        );
    }
    fn hold(&mut self, alias: &str) {
        let mut inventory = self.snapshot.inventory().to_vec();
        inventory.push(self.item(alias, inventory.len() as i32, ItemContainer::Inventory));
        self.snapshot.seed_inventory(inventory, 28);
    }
    fn seed_junk(&mut self, rows: usize) {
        let mut inventory = self.snapshot.inventory().to_vec();
        for _ in 0..rows {
            inventory.push(self.item("logs", inventory.len() as i32, ItemContainer::Inventory));
        }
        self.snapshot.seed_inventory(inventory, 28);
    }

    fn observe_bank(&mut self, tick: u64) {
        let side = self
            .snapshot
            .inventory()
            .iter()
            .map(|row| ItemView {
                container: ItemContainer::BankSide,
                component_id: 2006,
                actions: vec![Some("Deposit-1".into()), Some("Deposit-All".into())],
                ..row.clone()
            })
            .collect();
        self.snapshot
            .seed_bank_observation(7, tick, Some(self.stock.clone()), side);
    }

    fn drive(&mut self, tick: u64) {
        let here = self.snapshot.local_player().unwrap().player.actor.tile;
        let egg_visible = here.level == 0 && (here.x - 3227).abs().max((here.z - 3300).abs()) <= 6;
        self.snapshot
            .seed_ground_items(if egg_visible && !self.egg_taken {
                vec![GroundItemView {
                    def: self.item("egg", 0, ItemContainer::Inventory).def,
                    count: 1,
                    actions: vec![Some("Take".into())],
                    tile: tile(3227, 3300),
                    distance: 0,
                }]
            } else {
                Vec::new()
            });
        // The host's per-frame observe (design-bank-snapshot §1.3): mirror the
        // open bank into the account memory before the scripted tick reads it.
        self.bank.track(&self.snapshot, tick);
        with_tick_output_bank(
            &self.snapshot,
            Some(&self.bank),
            &mut self.ledger,
            tick,
            &mut self.output,
            |native| {
                assert_eq!(self.script.tick(native).unwrap(), ScriptFlow::Continue);
            },
        );
        while self
            .ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            let action = self.ledger.as_mut().unwrap().outbox.remove(0);
            let authority = action.authority();
            let evidence = EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            };
            match action.effect {
                HostEffect::BankPick(_) => self.ledger.as_mut().unwrap().complete_bank_pick(
                    &authority,
                    crate::bank::BankPickReceipt {
                        request_id: authority.request_id().get(),
                        evidence,
                        selected: crate::bank::SelectedBank {
                            bank_index: 0,
                            access_tile: self.bank_fixture.tile,
                            kind: crate::bank::PickKind::Reachable,
                            access: Some(Arc::new(crate::bank::BankStandAccess {
                                bank: self.bank_fixture,
                                stand_tile: self.bank_fixture.tile,
                                kind: crate::bank::AccessKind::Booth,
                                stand_op: 1,
                                name: None,
                                choose: None,
                            })),
                        },
                    },
                ),
                HostEffect::Walk(request) => {
                    self.snapshot
                        .seed_local_player(local_player(request.target));
                    // Moving away closes the real bank modal. Do not keep an
                    // impossible open-bank fixture while collecting an egg.
                    self.snapshot
                        .seed_bank_observation(-1, tick, None, Vec::new());
                    self.ledger.as_mut().unwrap().walk = Some(WalkReceipt {
                        request_id: authority.request_id().get(),
                        evidence,
                        end: WalkEnd::Arrived,
                        blocked: None,
                        detail: None,
                        refusal: None,
                        assessment: None,
                        escape: None,
                    });
                }
                HostEffect::AssessWalk(_) => {
                    panic!("the Cook bank fixture emits no walk assessment")
                }
                HostEffect::Interaction(request) => {
                    match request {
                        InteractReq::OpenStand { .. } => {
                            self.visits += 1;
                            self.observe_bank(tick);
                        }
                        InteractReq::SetNoteMode { on: false } => {}
                        InteractReq::WithdrawX {
                            bank_item_id,
                            count,
                            ..
                        } => {
                            let row = self
                                .stock
                                .iter_mut()
                                .find(|row| row.def.id == bank_item_id)
                                .unwrap();
                            assert!(row.count >= count);
                            row.count -= count;
                            let mut inventory = self.snapshot.inventory().to_vec();
                            let rows = if row.def.stackable { 1 } else { count };
                            for _ in 0..rows {
                                let mut held = row.clone();
                                held.count = if row.def.stackable { count } else { 1 };
                                held.container = ItemContainer::Inventory;
                                held.action_family = ItemActionFamily::Held;
                                held.actions.clear();
                                held.slot = (0..28)
                                    .find(|slot| !inventory.iter().any(|row| row.slot == *slot))
                                    .expect("withdrawal needs a real free slot");
                                inventory.push(held);
                            }
                            self.snapshot.seed_inventory(inventory, 28);
                            self.observe_bank(tick);
                            self.withdrawals += 1;
                        }
                        InteractReq::InvButton {
                            id,
                            slot,
                            component: 2006,
                            operation,
                            ..
                        } => {
                            assert!(
                                matches!(operation, 1 | 2),
                                "fixture models Deposit-1 and Deposit-All only"
                            );
                            let mut inventory = self.snapshot.inventory().to_vec();
                            let mut removed = 0;
                            inventory.retain(|row| {
                                let deposit =
                                    row.def.id == id && (operation == 2 || row.slot == slot);
                                removed += usize::from(deposit);
                                !deposit
                            });
                            assert!(removed > 0);
                            self.deposits += removed;
                            self.snapshot.seed_inventory(inventory, 28);
                            self.observe_bank(tick);
                        }
                        InteractReq::Obj { name, action, .. } => {
                            assert_eq!(name.as_deref(), Some("Egg"));
                            assert_eq!(action, "Take");
                            let mut inventory = self.snapshot.inventory().to_vec();
                            let slot = (0..28)
                                .find(|slot| !inventory.iter().any(|row| row.slot == *slot))
                                .expect("ground acquisition needs a real free slot");
                            inventory.push(self.item("egg", slot, ItemContainer::Inventory));
                            self.snapshot.seed_inventory(inventory, 28);
                            self.egg_taken = true;
                        }
                        request => panic!("unexpected Cook fixture interaction: {request:?}"),
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
            }
        }
    }

    fn until_root_begin(&mut self) {
        for tick in 1..=100 {
            self.drive(tick);
            if self.script.step.is_some() {
                return;
            }
        }
        panic!(
            "Cook never began its authored start: {:?}",
            self.output.logs
        );
    }

    fn until_milk(&mut self) {
        let milk = self
            .script
            .selected
            .item_by_alias("bucket_milk")
            .unwrap()
            .name
            .clone()
            .unwrap();
        for tick in 1..=100 {
            self.drive(tick);
            if self.script.provisioner.status().item == Some(milk.as_str())
                && self.script.provisioner.status().phase
                    == super::super::provision::ProvisionPhase::Acquiring
            {
                return;
            }
        }
        panic!(
            "Cook never progressed from egg to milk: {:?}",
            self.output.logs
        );
    }
}

#[test]
fn real_cook_in_progress_held_ingredients_do_not_scan_bank_for_optional_tools() {
    let mut fixture = CookFixture::new(true, false);
    fixture.snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    fixture.until_root_begin();
    assert_eq!(
        fixture.visits, 0,
        "missing optional tools must not cause an unrelated-step bank trip"
    );
}

#[test]
fn real_cook_held_ingredients_do_not_scan_bank_for_optional_tools() {
    let mut fixture = CookFixture::new(true, false);
    fixture.until_root_begin();
    assert_eq!(
        fixture.visits, 0,
        "optional acquisition tools must not force a bank trip when all goals are held"
    );
    assert_eq!(fixture.withdrawals, 0);
}

#[test]
fn real_cook_not_started_talks_first_without_bank_scan() {
    let mut fixture = CookFixture::new(false, false);
    fixture.until_root_begin();
    assert_eq!(
        fixture.visits, 0,
        "gated items must not scan the bank before the start talk"
    );
    assert!(fixture
        .output
        .logs
        .iter()
        .any(|line| line.contains("step start (talk) begin")));
}

#[test]
fn real_cook_banked_ingredients_share_one_open_bank_visit() {
    let mut fixture = CookFixture::new(false, true);
    // Gated items are only due from cook:1, so the banked withdrawal
    // run happens after the start talk, not before it.
    fixture.seed_in_progress();
    fixture.until_root_begin();
    assert_eq!(
        fixture.visits, 1,
        "scan and all three withdrawals must share the same visit"
    );
    assert_eq!(fixture.withdrawals, 3);
    assert!(fixture
        .output
        .logs
        .iter()
        .any(|line| line.contains("step hand-in (talk) begin")));
}

#[test]
fn real_cook_ground_acquisition_preserves_empty_bank_memory_between_recipes() {
    let mut fixture = CookFixture::new(false, false);
    // Flour now leads the acquisition order, so hold it: the test
    // exercises the ground egg leg that follows.
    fixture.seed_in_progress();
    fixture.hold("pot_flour");
    fixture.until_milk();
    assert!(
        fixture.egg_taken,
        "exercise the real take-egg family, not an injected outcome"
    );
    assert_eq!(
        fixture.visits, 1,
        "ground egg acquisition must not erase the observed empty bank and trigger a second trip"
    );
    // Nothing in the script mutates the account memory: the empty Session
    // rows the scan observed stay known between recipes.
    assert!(
        fixture.bank.known(),
        "the observed empty bank must stay known between recipes"
    );
    assert_eq!(
        fixture.bank.origin(),
        api::bank_memory::Origin::Session,
        "the fixture bank is really opened, not hinted"
    );
    assert_eq!(
        fixture
            .bank
            .count(fixture.script.selected.item_by_alias("egg").unwrap().id),
        Some(0)
    );
}

#[test]
fn real_cook_provisioned_acquisition_emits_child_begin_and_terminal_settle() {
    let mut fixture = CookFixture::new(false, false);
    fixture.seed_in_progress();
    fixture.hold("pot_flour");
    fixture.until_milk();
    assert!(
        fixture
            .output
            .logs
            .iter()
            .any(|line| line.contains("recipe acquire:egg child take-egg begin")),
        "provisioner-owned child begin must reach the run trace"
    );
    assert!(
        fixture
            .output
            .logs
            .iter()
            .any(|line| line.contains("recipe acquire:egg child take-egg settled")),
        "drain terminal events before dropping the acquisition run"
    );
}

#[test]
fn real_cook_terminal_stage_finishes_without_any_bank_or_walk_work() {
    let mut fixture = CookFixture::new(false, false);
    fixture.seed_junk(26);
    let finished_at = fixture.snapshot.local_player().unwrap().player.actor.tile;
    fixture.snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    let flow = with_tick_output(
        &fixture.snapshot,
        &mut fixture.ledger,
        1,
        &mut fixture.output,
        |native| fixture.script.tick(native).unwrap(),
    );
    assert_eq!(
        flow,
        ScriptFlow::Complete,
        "terminal progress must finish immediately, not schedule a completion retreat"
    );
    assert!(fixture
        .ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
    assert!(fixture
        .output
        .logs
        .iter()
        .any(|line| line == "quester cook: finish"));
    assert_eq!(fixture.snapshot.inventory().len(), 26);
    assert_eq!(
        fixture.snapshot.local_player().unwrap().player.actor.tile,
        finished_at
    );
}

#[test]
fn real_cook_unrelated_junk_with_enough_slots_makes_no_bank_trip() {
    let mut fixture = CookFixture::new(true, false);
    fixture.seed_junk(1);
    fixture.until_root_begin();
    assert_eq!(
        fixture.visits, 0,
        "unrelated junk is not an obstruction when all ingredients fit"
    );
    assert_eq!(fixture.deposits, 0);
    assert_eq!(fixture.snapshot.inventory().len(), 4);
}

#[test]
fn real_cook_unrelated_junk_deposits_only_the_missing_slot() {
    let mut fixture = CookFixture::new(false, true);
    fixture.seed_junk(26);
    // Gated items are only due from cook:1; at cook:0 this fixture
    // would talk first and never touch the bank.
    fixture.seed_in_progress();
    fixture.until_root_begin();
    assert_eq!(
        fixture.visits, 1,
        "selective deposit, scan and withdrawals share one bank visit"
    );
    assert_eq!(
        fixture.deposits, 1,
        "three ingredient slots minus two free slots needs exactly one junk row, not a sweep"
    );
    assert_eq!(fixture.withdrawals, 3);
    assert_eq!(fixture.snapshot.inventory().len(), 28);
}

#[test]
fn completed_queued_path_leaves_junk_until_next_path_needs_one_slot() {
    use crate::quester::queue::{Queue, QueueSettings, ReleaseIndex};

    let mut fixture = CookFixture::new(false, true);
    fixture.seed_junk(26);
    let index: ReleaseIndex = serde_json::from_str(super::super::compile::INDEX_JSON).unwrap();
    let queue = Queue::from_index(
        &index,
        QueueSettings {
            quests: vec!["sheep".into(), "cook".into()],
            order_override: vec!["sheep".into(), "cook".into()],
            ..QueueSettings::default()
        },
    )
    .unwrap();
    let mut queued = QueuedQuester::new(
        fixture.script.run,
        Arc::clone(&fixture.script.selected),
        Arc::clone(&fixture.script.quests),
        Arc::clone(&fixture.script.banks),
        queue,
    );
    let statuses = |sheep_colour| {
        vec![
            QuestStatusView {
                name: "Sheep Shearer".into(),
                component_id: 43,
                colour: sheep_colour,
            },
            QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                // Gated items are only due from cook:1; at cook:0 the
                // queued Cook would talk first and never touch the bank.
                colour: 0xf8f800,
            },
        ]
    };
    fixture
        .snapshot
        .seed_quest_statuses(statuses(0xf80000), true);
    queued.active_index = queued.queue.next_candidate();
    assert_eq!(queued.queue.id(queued.active_index.unwrap()), Some("sheep"));
    let selected = Arc::clone(&queued.selected);
    let quests = Arc::clone(&queued.quests);
    let sheep = super::super::compile::prepare_for_test(move |cap| {
        super::super::compile::compile_path(
            super::super::registry::bundled_path("sheep").unwrap(),
            &selected,
            &quests,
            cap,
        )
    })
    .unwrap();
    with_tick_output(
        &fixture.snapshot,
        &mut fixture.ledger,
        1,
        &mut fixture.output,
        |native| {
            queued.activate(native, sheep);
        },
    );
    assert!(queued.active.is_some());
    fixture
        .snapshot
        .seed_quest_statuses(statuses(0x00f800), true);
    with_tick_output(
        &fixture.snapshot,
        &mut fixture.ledger,
        2,
        &mut fixture.output,
        |native| {
            assert_eq!(queued.tick(native).unwrap(), ScriptFlow::Continue);
        },
    );
    assert_eq!(queued.completed, 1);
    assert!(
        fixture
            .ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()),
        "completed Sheep must not bank the junk or move the player"
    );
    assert_eq!(fixture.snapshot.inventory().len(), 26);
    assert_eq!(
        fixture
            .output
            .logs
            .iter()
            .filter(|line| line.as_str() == "quester queue: preparing next Path cook")
            .count(),
        1
    );
    let cook = queued
        .preparing
        .take()
        .expect("real next-Path compiler")
        .join()
        .unwrap()
        .unwrap();
    with_tick_output(
        &fixture.snapshot,
        &mut fixture.ledger,
        3,
        &mut fixture.output,
        |native| {
            queued.activate(native, cook);
        },
    );
    fixture.script = *queued.active.take().expect("queued Cook admitted");
    for tick in 4..=103 {
        fixture.drive(tick);
        if fixture.script.step.is_some() {
            break;
        }
    }
    assert!(fixture.script.step.is_some());
    assert_eq!(fixture.visits, 1);
    assert_eq!(
        fixture.deposits, 1,
        "only Cook's one blocked slot may be banked"
    );
    assert_eq!(fixture.withdrawals, 3);
    assert_eq!(fixture.snapshot.inventory().len(), 28);
}

fn sheep_fixture(held_wool: usize, banked_goals: bool) -> CookFixture {
    let mut fixture = CookFixture::new(false, false);
    let document = serde_json::from_str(super::super::compile::SHEEP_JSON).unwrap();
    fixture.script.path = super::super::compile::compile_uncached_for_test(
        &document,
        &fixture.script.selected,
        &fixture.script.quests,
    )
    .unwrap();
    fixture.snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Sheep Shearer".into(),
            component_id: 43,
            colour: 0xf80000,
        }],
        true,
    );
    fixture.snapshot.seed_inventory(
        (0..held_wool)
            .map(|slot| fixture.item("wool", slot as i32, ItemContainer::Inventory))
            .collect(),
        28,
    );
    if banked_goals {
        fixture.stock = [("shears", 1), ("wool", 20), ("ball_of_wool", 20)]
            .into_iter()
            .enumerate()
            .map(|(slot, (alias, count))| ItemView {
                count,
                ..fixture.item(alias, slot as i32, ItemContainer::Bank)
            })
            .collect();
    }
    fixture
}
fn r2_fixture_from_bundled_path(
    id: &str,
    patch: impl FnOnce(&mut serde_json::Value),
) -> CookFixture {
    let mut fixture = CookFixture::new(false, false);
    let bytes = super::super::registry::bundled_path(id).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    patch(&mut value);
    let document: super::super::path::PathDocument = serde_json::from_value(value).unwrap();
    fixture.script.path = super::super::compile::compile_uncached_for_test(
        &document,
        &fixture.script.selected,
        &fixture.script.quests,
    )
    .unwrap();
    let display = fixture
        .script
        .selected
        .quest_identity()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.id == id)
        .unwrap()
        .display
        .clone();
    fixture.snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: display,
            component_id: 43,
            colour: 0xf80000,
        }],
        true,
    );
    fixture
}

fn r2_seed_rows(fixture: &mut CookFixture, rows: &[(&str, usize)]) {
    let mut inventory = Vec::new();
    for (alias, count) in rows {
        for _ in 0..*count {
            let mut row = fixture.item(alias, inventory.len() as i32, ItemContainer::Inventory);
            if *alias == "coins" {
                row.def.stackable = true;
            }
            inventory.push(row);
        }
    }
    fixture.snapshot.seed_inventory(inventory, 28);
}

fn r2_seed_bank(fixture: &mut CookFixture, rows: &[(&str, i32)]) {
    fixture.stock = rows
        .iter()
        .enumerate()
        .map(|(slot, (alias, count))| {
            let mut row = fixture.item(alias, slot as i32, ItemContainer::Bank);
            row.count = *count;
            if *alias == "coins" {
                row.def.stackable = true;
            }
            row
        })
        .collect();
}

fn r2_until_provision_decision(fixture: &mut CookFixture) {
    for tick in 1..=100 {
        fixture.drive(tick);
        let phase = fixture.script.provisioner.status().phase;
        if fixture.script.parked
            || fixture.script.step.is_some()
            || phase == super::super::provision::ProvisionPhase::Acquiring
            || phase == super::super::provision::ProvisionPhase::Blocked
        {
            return;
        }
    }
    panic!(
        "Path did not reach a provisioning decision: {:?}",
        fixture.output.logs
    );
}

#[test]
fn r2_sheep_optional_coin_float_does_not_park_a_full_path_pack() {
    let mut fixture = r2_fixture_from_bundled_path("sheep", |_| {});
    r2_seed_rows(&mut fixture, &[("shears", 1), ("wool", 27)]);
    r2_seed_bank(&mut fixture, &[("coins", 100)]);

    r2_until_provision_decision(&mut fixture);

    assert!(
        !fixture.script.parked,
        "optional coins must not turn a full but runnable Path pack into a capacity park: {:?}",
        fixture.output.logs
    );
    assert!(fixture.script.step.is_some());
    assert_eq!(
        fixture.visits, 1,
        "the bank is scanned before optional coins are skipped"
    );
    assert_eq!(fixture.withdrawals, 0);
    assert_eq!(fixture.deposits, 0);
}

#[test]
fn r2_cook_hint_admission_preserves_the_active_egg_slot() {
    let mut fixture = r2_fixture_from_bundled_path("cook", |value| {
        value["quest"]["items"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "obj": "wool", "qty": 20, "kind": "acquirable", "acquire": null
            }));
    });
    r2_seed_rows(&mut fixture, &[("pot_empty", 8)]);
    r2_seed_bank(&mut fixture, &[("wool", 20)]);
    // Flour leads the bundled order, so hold it: this test exercises the egg leg.
    fixture.seed_in_progress();
    fixture.hold("pot_flour");

    r2_until_provision_decision(&mut fixture);

    let status = fixture.script.provisioner.status();
    assert_eq!(
        status.phase,
        super::super::provision::ProvisionPhase::Acquiring,
        "Cook should begin its egg recipe after deferring the non-fitting wool hint: {:?}",
        fixture.output.logs
    );
    assert_eq!(status.item, Some("Egg"));
    let free = 28 - fixture.snapshot.inventory().len() as i32;
    assert!(
        free >= 1,
        "the egg acquisition must retain its peak free slot; got free={free}, status={:?}",
        status.phase
    );
}

#[test]
fn r2_cook_does_not_start_an_acquisition_without_its_final_slot() {
    let mut fixture = r2_fixture_from_bundled_path("cook", |_| {});
    // A full pack of preserved tools that overlap no flour peak input:
    // neither the grain nor the final flour has a slot.
    r2_seed_rows(&mut fixture, &[("bucket_empty", 28)]);
    // Gated items are only due from cook:1; at cook:0 this parks nothing and talks instead.
    fixture.seed_in_progress();

    r2_until_provision_decision(&mut fixture);

    let status = fixture.script.provisioner.status();
    assert_ne!(
        status.phase,
        super::super::provision::ProvisionPhase::Acquiring,
        "an acquisition whose output cannot fit must not start: {:?}",
        fixture.output.logs
    );
    assert!(fixture.script.parked);
    assert!(fixture.script.step.is_none());
}

#[test]
fn r1_real_sheep_empty_and_twenty_wool_reach_first_root_without_capacity_park() {
    for held_wool in [0, 20] {
        let mut fixture = sheep_fixture(held_wool, false);
        fixture.until_root_begin();
        assert!(!fixture.script.parked, "{:?}", fixture.output.logs);
        assert_eq!(fixture.deposits, 0);
        assert_eq!(fixture.snapshot.inventory().len(), held_wool);
    }
}

#[test]
fn r1_real_sheep_banked_sequential_goals_do_not_require_forty_one_slots() {
    let mut fixture = sheep_fixture(0, true);
    fixture.until_root_begin();
    assert!(!fixture.script.parked, "{:?}", fixture.output.logs);
    assert_eq!(fixture.deposits, 0);
    assert_eq!(fixture.snapshot.inventory().len(), 21);
    assert_eq!(
        fixture
            .stock
            .iter()
            .find(|row| row.def.id == 1759)
            .unwrap()
            .count,
        20,
        "defer a later authored goal that does not fit beside this acquisition"
    );
}

#[test]
fn r1_provisioner_bank_scan_and_withdrawal_are_traced() {
    let mut fixture = CookFixture::new(false, true);
    // Gated items are only due from cook:1; at cook:0 there is no bank run to trace.
    fixture.seed_in_progress();
    fixture.until_root_begin();
    for phase in ["scan", "withdraw"] {
        for outcome in ["begin", "settled"] {
            let expected = format!("quester cook: provision bank {phase} {outcome}");
            assert!(
                fixture.output.logs.iter().any(|line| line == &expected),
                "missing {expected:?}: {:?}",
                fixture.output.logs
            );
        }
    }
}

#[test]
fn r1_provisioner_capacity_deposit_is_traced() {
    let mut fixture = CookFixture::new(false, true);
    fixture.seed_junk(26);
    // Gated items are only due from cook:1; at cook:0 there is no bank run to trace.
    fixture.seed_in_progress();
    fixture.until_root_begin();
    for outcome in ["begin", "settled"] {
        let expected = format!("quester cook: provision bank deposit-capacity {outcome}");
        assert!(
            fixture.output.logs.iter().any(|line| line == &expected),
            "missing {expected:?}: {:?}",
            fixture.output.logs
        );
    }
}

#[test]
fn r1_finish_is_emitted_after_trace_event_cap() {
    let mut fixture = CookFixture::new(false, false);
    for event in 0..=RUN_TRACE_EVENT_LIMIT {
        fixture.script.trace.record(
            &mut fixture.output,
            api::hostlog::Level::Info,
            format_args!("quester cook: distinct event {event}"),
        );
    }
    fixture.snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    with_tick_output(
        &fixture.snapshot,
        &mut fixture.ledger,
        1,
        &mut fixture.output,
        |native| assert_eq!(fixture.script.tick(native).unwrap(), ScriptFlow::Complete),
    );
    assert_eq!(
        fixture
            .output
            .logs
            .iter()
            .filter(|line| line.as_str() == "quester cook: finish")
            .count(),
        1,
        "{:?}",
        fixture.output.logs
    );
}

#[test]
fn r1_bundled_non_inventory_paths_reach_first_root_from_empty_pack() {
    // Bundled gather Paths compile against the prepared catalog; keep it
    // alive for the whole loop.
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let _gathering =
        super::super::compile::prepare_for_test(move |worker| selected.prepare_gathering(worker))
            .expect("289 gather catalog");
    for entry in super::super::registry::BUNDLED_INDEX.paths.iter() {
        let Some(bytes) = super::super::registry::bundled_path(&entry.id) else {
            continue;
        };
        let mut fixture = CookFixture::new(false, false);
        let document: super::super::path::PathDocument = serde_json::from_slice(bytes).unwrap();
        fixture.script.path = super::super::compile::compile_uncached_for_test(
            &document,
            &fixture.script.selected,
            &fixture.script.quests,
        )
        .unwrap();
        if fixture.script.path.provisioning.owns_inventory {
            continue;
        }
        fixture.snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: fixture
                    .script
                    .selected
                    .quest_identity()
                    .unwrap()
                    .rows
                    .iter()
                    .find(|row| row.id == entry.id)
                    .unwrap()
                    .display
                    .clone(),
                component_id: 43,
                colour: 0xf80000,
            }],
            true,
        );
        // Required supplies are available in the bank, not preloaded in the
        // pack. This sweep tests inventory capacity, not a supply shortfall.
        fixture.stock = fixture
            .script
            .path
            .provisioning
            .items
            .iter()
            .enumerate()
            .map(|(slot, item)| {
                let mut definition = def(item.id, &item.name);
                definition.stackable = item.stackable;
                ItemView {
                    def: definition,
                    container: ItemContainer::Bank,
                    action_family: ItemActionFamily::Component,
                    slot: slot as i32,
                    count: item.qty as i32,
                    actions: vec![Some("Withdraw-X".into()), Some("Withdraw-1".into())],
                    component_id: 7,
                }
            })
            .collect();
        fixture.until_root_begin();
        assert!(
            !fixture.script.parked,
            "{}: {:?}",
            entry.id, fixture.output.logs
        );
        assert_eq!(
            fixture.deposits, 0,
            "empty {} pack needs no deposits",
            entry.id
        );
    }
}
