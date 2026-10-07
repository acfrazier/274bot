//! Runner-level safety pins for Sir Vyvin's portrait search in `paths/289/squire.json`.
use super::*;
use crate::native::{NativeOutput, ScriptFlow, ScriptStatus};
use crate::quester::families::tests::with_tick_output_bank;
use crate::quester::path::PathDocument;
use api::bank_memory::{BankMemory, Origin};
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, NpcView,
};
use api::WorldTile;
use std::sync::Arc;

const PLAYER_STAND: WorldTile = WorldTile {
    x: 2985,
    z: 3335,
    level: 2,
};
const ADJACENT_VYVIN: WorldTile = WorldTile {
    x: 2984,
    z: 3335,
    level: 2,
};
const DISTANT_VYVIN: WorldTile = WorldTile {
    x: 2983,
    z: 3335,
    level: 2,
};
const OPEN_CUPBOARD: WorldTile = WorldTile {
    x: 2984,
    z: 3336,
    level: 2,
};

#[derive(Default)]
struct Capture;

impl NativeOutput for Capture {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, _: &str) {}
    fn settings_applied(&mut self, _: u64) {}
}

fn squire_path() -> (
    Arc<api::game_data::SelectedGameData>,
    Arc<QuestCatalog>,
    Arc<CompiledPath>,
) {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let _gathering = super::super::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        move |worker| selected.prepare_gathering(worker)
    })
    .unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
    let document: PathDocument = serde_json::from_str(super::super::compile::SQUIRE_JSON).unwrap();
    let path =
        super::super::compile::compile_uncached_for_test(&document, &selected, &quests).unwrap();
    (selected, quests, path)
}

fn coins_item(selected: &api::game_data::SelectedGameData) -> ItemView {
    let item = selected.item_by_alias("coins").unwrap();
    ItemView {
        def: api::obj_names::ItemDefView {
            id: item.id,
            name: Some("Coins".into()),
            stackable: true,
            members: false,
            base_value: 1,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 300,
        actions: Vec::new(),
        component_id: 3214,
    }
}

fn portrait_item(selected: &api::game_data::SelectedGameData) -> ItemView {
    let item = selected.item_by_alias("knights_portrait").unwrap();
    ItemView {
        def: api::obj_names::ItemDefView {
            id: item.id,
            name: Some("Portrait".into()),
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 1,
        count: 1,
        actions: Vec::new(),
        component_id: 3214,
    }
}

fn vyvin(selected: &api::game_data::SelectedGameData, tile: WorldTile) -> NpcView {
    let npc = selected.npc_by_config("sir_vyvin").unwrap();
    NpcView {
        index: 42,
        r#type: Some(npc.id as usize),
        name: npc.display.clone(),
        actions: Vec::new(),
        tile,
        distance: if tile == ADJACENT_VYVIN { 1 } else { 2 },
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
        level: 1,
        size: 1,
        network: tile,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn snapshot(
    selected: &api::game_data::SelectedGameData,
    vyvin_tile: WorldTile,
    holds_portrait: bool,
) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    let mut inventory = vec![coins_item(selected)];
    if holds_portrait {
        inventory.push(portrait_item(selected));
    }
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_local_player(super::super::families::tests::local_player(PLAYER_STAND));
    snapshot.seed_npcs(vec![vyvin(selected, vyvin_tile)]);
    let loc = selected.loc_by_config("vyvincupboardopen").unwrap();
    snapshot.seed_locs(vec![LocView {
        id: loc.id,
        name: Some("Cupboard".into()),
        actions: vec![Some("Search".into()), Some("Shut".into())],
        tile: OPEN_CUPBOARD,
        distance: 1,
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
    snapshot
}

/// The authored ordered choice observes both the one-tile NPC gate and its
/// portrait bypass: adjacency selects the wait, distance selects the search.
#[test]
fn portrait_choice_waits_when_vyvin_is_adjacent_and_searches_when_clear() {
    let (selected, quests, path) = squire_path();
    let bank = BankMemory::seeded(&[], Origin::Session);
    let probe = crate::quester::probe::Probe {
        path: &path,
        selected: &selected,
        quests: &quests,
        progress: &[],
        bank: &bank,
    };

    let adjacent = snapshot(&selected, ADJACENT_VYVIN, false);
    assert_eq!(
        probe.choice("squire:5", 0, &adjacent),
        crate::quester::probe::Choice::Step(FactKey::new("wait-vyvin-clear")),
        "an adjacent Vyvin must prevent the Search step from starting"
    );

    let clear = snapshot(&selected, DISTANT_VYVIN, false);
    assert_eq!(
        probe.choice("squire:5", 0, &clear),
        crate::quester::probe::Choice::Step(FactKey::new("search-vyvin-cupboard")),
        "the wait is skipped when Vyvin is outside the refusal radius"
    );

    let portrait_held = snapshot(&selected, ADJACENT_VYVIN, true);
    assert_eq!(
        probe.choice("squire:5", 0, &portrait_held),
        crate::quester::probe::Choice::Step(FactKey::new("give-portrait")),
        "a held portrait skips both the wait and search even while Vyvin is adjacent"
    );
}

struct PortraitWaitFixture {
    script: Quester,
    snapshot: GameSnapshot,
    bank: BankMemory,
    ledger: Option<Box<crate::native::ledger::Ledger>>,
}

impl PortraitWaitFixture {
    fn new() -> Self {
        let (selected, quests, path) = squire_path();
        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&selected),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let stage = FactKey::new("squire:5");
        script.seq_index =
            sequence_for_stage(&script.path, stage.0.as_ref()).expect("squire stage five sequence");
        script.stage = Some(stage);
        script.needs_read = false;

        Self {
            script,
            snapshot: snapshot(&selected, ADJACENT_VYVIN, false),
            bank: BankMemory::seeded(&[], Origin::Session),
            ledger: None,
        }
    }

    fn drive(&mut self, tick: u64, output: &mut Capture) -> ScriptFlow {
        self.bank.track(&self.snapshot, tick);
        with_tick_output_bank(
            &self.snapshot,
            Some(&self.bank),
            &mut self.ledger,
            tick,
            output,
            |native| self.script.tick(native).unwrap(),
        )
    }
}

/// A continuously adjacent Sir Vyvin exhausts the authored 200-tick wait and
/// parks the path with its explicit `wait exhausted` reason.
#[test]
fn portrait_wait_timeout_parks_with_clear_message() {
    let mut fixture = PortraitWaitFixture::new();
    let mut output = Capture;
    let first = fixture.drive(1, &mut output);
    assert!(!matches!(first, ScriptFlow::Blocked(_)));
    assert_eq!(
        fixture.script.current_step().map(|step| step.id.0.as_ref()),
        Some("wait-vyvin-clear")
    );

    let failure = (2..=220)
        .find_map(|tick| match fixture.drive(tick, &mut output) {
            ScriptFlow::Blocked(failure) => Some(failure),
            _ => None,
        })
        .expect("the bounded wait should park when Vyvin never clears");
    assert!(fixture.script.parked, "timeout must park the Squire path");
    assert_eq!(failure.message.as_ref(), "wait exhausted");
    assert_eq!(
        fixture.script.blocked_failure().message.as_ref(),
        "wait exhausted"
    );
}
