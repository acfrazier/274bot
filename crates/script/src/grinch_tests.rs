use super::CARD;
use crate::native::{
    ledger, ActionContext, HostEffect, InteractionReceipt, NativeActions, NativeOutput,
    NativePhase, NativeTick, PreparedConfig, RetainedMemory, Script, ScriptFailure, ScriptFlow,
    ScriptStatus, SettingsBag, StatusValue, StopReason, WalkEnd, WalkReceipt,
};
use crate::native_bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
use crate::shim::InteractReq;
#[cfg(feature = "load")]
use api::game_data::SelectedGameData;
use api::named_banks::{NamedBank, NamedBankFacts};
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, FamilyPreparation, RunKey, Truth};
use api::snapshot::{
    GameSnapshot, GroundItemView, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView,
    SnapshotView, WorldStateView, WorldTile,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RUN: RunKey = RunKey {
    slot: 1,
    run: 1,
    session: 1,
};
const ANCHOR: WorldTile = tile(3216, 3216);
const BANK_TILE: WorldTile = tile(3220, 3216);
const ITEM: i32 = 1959;
const OTHER: i32 = 5001;

const DEFAULT_ITEMS: [i32; 18] = [
    1961, 981, 1989, 1959, 1053, 1055, 1057, 1050, 962, 1038, 1040, 1042, 1044, 1046, 1048, 1419,
    1037, 4079,
];

const fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn item_name(id: i32) -> &'static str {
    match id {
        1961 => "Easter egg",
        981 => "Disk of returning",
        1989 => "Half full wine jug",
        1959 => "Pumpkin",
        1053 => "Green mask",
        1055 => "Blue mask",
        1057 => "Red mask",
        1050 => "Santa hat",
        962 => "Christmas cracker",
        1038 => "Red partyhat",
        1040 => "Yellow partyhat",
        1042 => "Blue partyhat",
        1044 => "Green partyhat",
        1046 => "Purple partyhat",
        1048 => "White partyhat",
        1419 => "Scythe",
        1037 => "Bunny ears",
        4079 => "Yo-yo",
        _ => "Other item",
    }
}

fn settings(items: &[i32]) -> SettingsBag {
    let mut bag = SettingsBag::new();
    bag.insert(
        "items".into(),
        serde_json::json!(items.iter().map(ToString::to_string).collect::<Vec<_>>()),
    );
    bag
}

fn prepare_revision(
    bag: SettingsBag,
    banks: Arc<NamedBankFacts>,
    revision: u64,
) -> Result<Arc<PreparedConfig>, crate::native::StartError> {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let pin = selected.selected_pin().unwrap();
    FamilyPreparation::run(move |families| {
        let mut cx = crate::native::PrepareContext {
            selected,
            pin,
            banks,
            families,
        };
        (CARD.prepare)(&mut cx, revision, Arc::new(bag))
    })
    .unwrap()
    .join()
    .unwrap()
}

fn prepare(bag: SettingsBag) -> Result<Arc<PreparedConfig>, crate::native::StartError> {
    prepare_revision(bag, Arc::new(NamedBankFacts::empty()), 1)
}

#[derive(Default)]
struct CaptureOutput {
    statuses: Vec<ScriptStatus>,
    logs: Vec<String>,
}

impl NativeOutput for CaptureOutput {
    fn status(&mut self, status: ScriptStatus) {
        self.statuses.push(status);
    }

    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}

    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.logs.push(message.to_owned());
    }

    fn settings_applied(&mut self, _: u64) {}
}

struct Driver {
    run: RunKey,
    #[cfg(feature = "load")]
    selected: Arc<SelectedGameData>,
    pin: Arc<api::selected::SelectedPin>,
    retained: RetainedMemory,
    ledger: Option<Box<ledger::Ledger>>,
    pending_note_progress: Option<RunKey>,
    output: CaptureOutput,
}

struct TickResult {
    flow: Result<ScriptFlow, ScriptFailure>,
    noted: bool,
}

impl Driver {
    fn new(run: RunKey) -> Self {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let pin = selected.selected_pin().unwrap();
        Self {
            run,
            #[cfg(feature = "load")]
            selected,
            pin,
            retained: RetainedMemory::default(),
            ledger: None,
            pending_note_progress: None,
            output: CaptureOutput::default(),
        }
    }

    fn create(&mut self, config: Arc<PreparedConfig>) -> Box<dyn Script> {
        (CARD.create)(self.run, config, &mut self.retained).unwrap()
    }

    fn tick(
        &mut self,
        script: &mut dyn Script,
        snapshot: &GameSnapshot,
        reach: Option<&api::query::ReachQueryView>,
        bank_memory: Option<&api::bank_memory::BankMemory>,
        tick: u64,
    ) -> TickResult {
        let run = self.run;
        let evidence = EvidenceStamp {
            run,
            tick,
            sequence: tick,
        };
        #[cfg(feature = "load")]
        let selected = Arc::clone(&self.selected);
        let pin = Arc::clone(&self.pin);
        let mut budget = ledger::TickBudget::default();
        budget.observe(tick);
        let flow = {
            let mut actions = NativeActions { _private: () };
            let cx = ActionContext {
                evidence,
                pin: &pin,
                snapshot: SnapshotView::new(Some(snapshot), evidence)
                    .with_reach(reach)
                    .with_world_members(Truth::Unknown)
                    .with_bank_memory(bank_memory),
                retained: &mut self.retained,
                action_id: 0,
                active_now: Duration::from_millis(tick.saturating_mul(600)),
                wall_now: Instant::now(),
                ledger: &mut self.ledger,
                pending_note_progress: Some(&mut self.pending_note_progress),
                budget: &mut budget,
                eligible: true,
                observed_walk_outcome_seq: 0,
            };
            let mut native = NativeTick {
                actions: &mut actions,
                cx,
                output: &mut self.output,
                pairs: None,
                state: None,
                service_authority: None,
                prompt_port: None,
                #[cfg(feature = "load")]
                frame: crate::native::HostFrame {
                    here: None,
                    snapshot: Some(snapshot),
                    obj_names: None,
                    compiled: crate::CompiledTick {
                        selected: Some(&selected),
                        reach: None,
                        reach_flood: None,
                        bank_memory,
                        collision: None,
                        hold: false,
                        world_members: Truth::Unknown,
                        interacts: Some(Vec::new()),
                        service_gate: None,
                    },
                },
            };
            script.tick(&mut native)
        };
        let noted = self.pending_note_progress.take() == Some(run);
        TickResult { flow, noted }
    }
}

fn require_continue(result: TickResult) -> bool {
    let TickResult { flow, noted } = result;
    assert!(
        matches!(flow, Ok(ScriptFlow::Continue)),
        "unexpected flow: {flow:?}"
    );
    noted
}

fn ready_snapshot(
    here: WorldTile,
    rows: Vec<GroundItemView>,
    inventory: Vec<ItemView>,
    capacity: usize,
) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView {
        map_base_x: 3168,
        map_base_z: 3168,
        ..WorldStateView::default()
    });
    snapshot.seed_local_player(crate::quester::families::tests::local_player(here));
    snapshot.seed_inventory(inventory, capacity.try_into().unwrap());
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_ground_items(rows);
    snapshot
}

fn ground(id: i32, at: WorldTile, count: i32) -> GroundItemView {
    GroundItemView {
        def: crate::quester::families::tests::def(id, item_name(id)),
        count,
        actions: vec![Some("Take".into())],
        tile: at,
        distance: (at.x - ANCHOR.x).abs().max((at.z - ANCHOR.z).abs()),
    }
}

fn held(id: i32, slot: i32, count: i32) -> ItemView {
    ItemView {
        def: crate::quester::families::tests::def(id, item_name(id)),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count,
        actions: vec![Some("Use".into())],
        component_id: 3214,
    }
}

fn full_pack(selected: Option<i32>) -> Vec<ItemView> {
    (0..28)
        .map(|slot| {
            let id = if slot == 0 {
                selected.unwrap_or(5000)
            } else {
                5000 + slot
            };
            held(id, slot, 1)
        })
        .collect()
}

fn reach_view(here: WorldTile, blocked: &[WorldTile]) -> api::query::ReachQueryView {
    let mut scene = api::snapshot::SceneView {
        available: true,
        base_x: 3180,
        base_z: 3180,
        level: here.level,
        width: 72,
        height: 72,
        collision_flags: vec![0; 72 * 72],
    };
    for at in blocked {
        let index = ((at.x - scene.base_x) * scene.height + at.z - scene.base_z) as usize;
        scene.collision_flags[index] |= client::dash3d::CollisionFlag::SQ_BLOCKED;
    }
    let flooded = api::query::SceneQuery::new(&scene, Some(here)).flood_reach();
    api::query::pack_reach_query(&scene, flooded.as_ref())
}

fn set_position(snapshot: &mut GameSnapshot, at: WorldTile) {
    snapshot.seed_local_player(crate::quester::families::tests::local_player(at));
}

fn stamp(run: RunKey, tick: u64) -> EvidenceStamp {
    EvidenceStamp {
        run,
        tick,
        sequence: tick,
    }
}

fn latest_obj(driver: &Driver) -> Option<(u64, i32, WorldTile)> {
    driver
        .ledger
        .as_ref()?
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::Interaction(InteractReq::Obj {
                id: Some(id),
                x,
                z,
                level,
                ..
            }) => Some((
                action.request_id.get(),
                *id,
                WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                },
            )),
            _ => None,
        })
}

fn latest_walk(driver: &Driver) -> Option<(u64, WorldTile, u16)> {
    driver
        .ledger
        .as_ref()?
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::Walk(request) => {
                Some((action.request_id.get(), request.target, request.radius))
            }
            _ => None,
        })
}

type BankPickObservation = (u64, Vec<u16>, Option<u16>, bool, WorldTile);

fn latest_bank_pick(driver: &Driver) -> Option<BankPickObservation> {
    driver
        .ledger
        .as_ref()?
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::BankPick(request) => Some((
                action.request_id.get(),
                request.eligible.to_vec(),
                request.explicit_bank,
                request.allow_air_fallback,
                request.from,
            )),
            _ => None,
        })
}

fn respond_interaction(driver: &mut Driver, tick: u64, accepted: bool, chat_since: i32) -> u64 {
    let (authority, request_id, run) = {
        let action = driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, HostEffect::Interaction(_)))
            .expect("native interaction request");
        (action.authority(), action.request_id.get(), action.run())
    };
    driver.ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id,
            evidence: stamp(run, tick),
            accepted,
            chat_since,
        },
    );
    request_id
}

fn respond_walk(driver: &mut Driver, tick: u64, end: WalkEnd) -> (u64, WorldTile) {
    let (request_id, run, target) = {
        let action = driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find_map(|action| match &action.effect {
                HostEffect::Walk(request) => {
                    Some((action.request_id.get(), action.run(), request.target))
                }
                _ => None,
            })
            .expect("native walk request");
        action
    };
    driver.ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        goal: None,
        request_id,
        evidence: stamp(run, tick),
        end,
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    });
    (request_id, target)
}

fn respond_bank_pick(
    driver: &mut Driver,
    tick: u64,
    bank_index: u16,
    bank: NamedBank,
    access_tile: WorldTile,
) -> u64 {
    let (authority, request_id, run) = {
        let action = driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, HostEffect::BankPick(_)))
            .expect("bank selection request");
        (action.authority(), action.request_id.get(), action.run())
    };
    driver.ledger.as_mut().unwrap().complete_bank_pick(
        &authority,
        BankPickReceipt {
            request_id,
            evidence: stamp(run, tick),
            selected: SelectedBank {
                bank_index,
                access_tile,
                kind: PickKind::Reachable,
                access: Some(Arc::new(BankStandAccess {
                    bank,
                    stand_tile: access_tile,
                    kind: AccessKind::Booth,
                    stand_op: 1,
                    name: None,
                    choose: None,
                })),
            },
        },
    );
    request_id
}

fn status(driver: &Driver) -> &ScriptStatus {
    driver
        .output
        .statuses
        .last()
        .expect("native status publication")
}

fn field<'a>(status: &'a ScriptStatus, key: &str) -> &'a StatusValue {
    &status
        .fields
        .iter()
        .find(|field| field.key == key)
        .unwrap_or_else(|| panic!("missing status field {key}"))
        .value
}

fn text_field<'a>(status: &'a ScriptStatus, key: &str) -> &'a str {
    match field(status, key) {
        StatusValue::Text(value) => value,
        other => panic!("{key} is not text: {other:?}"),
    }
}

fn integer_field(status: &ScriptStatus, key: &str) -> i64 {
    match field(status, key) {
        StatusValue::Integer(value) => *value,
        other => panic!("{key} is not an integer: {other:?}"),
    }
}

fn bank_row(id: i32, name: &str, count: i32) -> ItemView {
    ItemView {
        def: crate::quester::families::tests::def(id, name),
        container: ItemContainer::Bank,
        action_family: ItemActionFamily::Component,
        slot: 0,
        count,
        actions: vec![Some("Withdraw-1".into()), Some("Withdraw-X".into())],
        component_id: 7,
    }
}

fn bank_side_row(id: i32, name: &str, count: i32) -> ItemView {
    ItemView {
        def: crate::quester::families::tests::def(id, name),
        container: ItemContainer::BankSide,
        action_family: ItemActionFamily::Component,
        slot: 0,
        count,
        actions: vec![
            Some("Deposit-1".into()),
            Some("Deposit-5".into()),
            Some("Deposit-10".into()),
            Some("Deposit-All".into()),
        ],
        component_id: 7,
    }
}

fn bank_booth(at: WorldTile) -> LocView {
    LocView {
        id: 2213,
        name: Some("Bank booth".into()),
        actions: vec![Some("Use-quickly".into())],
        tile: at,
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
    }
}

fn set_bank_open(snapshot: &mut GameSnapshot, bank_rows: Vec<ItemView>, side: Vec<ItemView>) {
    snapshot.seed_bank_observation(10, 1, Some(bank_rows), side);
}

fn set_bank_closed(snapshot: &mut GameSnapshot) {
    snapshot.seed_bank_observation(-1, 2, None, Vec::new());
}

#[test]
fn card_schema_exposes_native_defaults_all_items_and_mask_labels() {
    assert_eq!(CARD.id.0, "Grinch");
    assert_eq!(CARD.name, "Grinch");
    assert_eq!(CARD.category, "Gathering");
    assert_eq!(CARD.schema_version, 1);
    assert!(CARD.per_account_settings.is_empty());

    let schema = (CARD.schema)();
    let definition = |name: &str| schema.iter().find(|field| field.id == name).unwrap();
    let items = definition("items");
    assert_eq!(items.ty, "string[]");
    assert_eq!(items.options.len(), DEFAULT_ITEMS.len());
    assert_eq!(items.option_labels.len(), DEFAULT_ITEMS.len());
    let options: Vec<i32> = items
        .options
        .iter()
        .map(|option| option.parse().unwrap())
        .collect();
    let defaults: Vec<String> = serde_json::from_str(items.default.as_deref().unwrap()).unwrap();
    let defaults: Vec<i32> = defaults
        .iter()
        .map(|option| option.parse().unwrap())
        .collect();
    assert_eq!(options.len(), DEFAULT_ITEMS.len());
    assert_eq!(defaults.len(), DEFAULT_ITEMS.len());
    for id in DEFAULT_ITEMS {
        assert!(options.contains(&id), "missing item choice {id}");
        assert!(
            defaults.contains(&id),
            "item {id} is not selected by default"
        );
        let label = items
            .option_labels
            .iter()
            .find(|label| label.contains(&id.to_string()))
            .unwrap_or_else(|| panic!("item label omits ID {id}"));
        assert!(label.contains(&id.to_string()));
    }
    for (id, colour) in [(1053, "green"), (1055, "blue"), (1057, "red")] {
        let label = items
            .option_labels
            .iter()
            .find(|label| label.contains(&id.to_string()))
            .unwrap();
        assert!(label.to_ascii_lowercase().contains(colour));
    }

    let radius = definition("radius");
    assert_eq!(radius.default.as_deref(), Some("12"));
    assert_eq!(radius.min.as_deref(), Some("1"));
    assert_eq!(radius.max.as_deref(), Some("16"));
    let bank = definition("bank");
    assert_eq!(bank.default.as_deref(), Some("Nearest"));
    assert_eq!(bank.options_from.as_deref(), Some("named-banks"));
    for key in ["allow_wilderness", "allow_danger_zones", "allow_teleports"] {
        let setting = definition(key);
        assert_eq!(setting.ty, "boolean");
        assert_eq!(setting.default.as_deref(), Some("false"));
    }
}

#[test]
fn prepare_rejects_invalid_item_sets_and_radius_but_accepts_both_edges() {
    for items in [
        serde_json::json!([]),
        serde_json::json!(["999999"]),
        serde_json::json!([1959]),
        serde_json::json!(vec!["1959"; 33]),
    ] {
        let mut bag = SettingsBag::new();
        bag.insert("items".into(), items);
        assert!(prepare(bag).is_err());
    }
    for radius in [0, 17] {
        let mut bag = settings(&[ITEM]);
        bag.insert("radius".into(), serde_json::json!(radius));
        assert!(prepare(bag).is_err(), "radius {radius} must be rejected");
    }
    for radius in [1, 16] {
        let mut bag = settings(&[ITEM]);
        bag.insert("radius".into(), serde_json::json!(radius));
        assert!(prepare(bag).is_ok(), "radius {radius} must be accepted");
    }

    let mut old_settings = settings(&[ITEM]);
    old_settings.insert("radius".into(), serde_json::json!(16));
    let config = prepare(old_settings).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let next = prepare_revision(
        {
            let mut next = settings(&[1055]);
            next.insert("radius".into(), serde_json::json!(1));
            next
        },
        Arc::new(NamedBankFacts::empty()),
        2,
    )
    .unwrap();
    assert_eq!(
        script.configure(next).unwrap(),
        crate::native::SettingsApply::RestartRequired
    );
}

#[test]
fn prepare_stores_choices_in_the_typed_runtime_payload() {
    let bank = NamedBank::new("Fixture bank", ANCHOR);
    let banks = Arc::new(NamedBankFacts::from_banks(vec![bank]));
    let mut bag = settings(&[1055, ITEM]);
    bag.insert("radius".into(), serde_json::json!(7));
    bag.insert("bank".into(), serde_json::json!("Fixture bank"));
    let config = prepare_revision(bag, Arc::clone(&banks), 2).unwrap();
    let prepared = config
        .get::<Arc<super::Prepared>>()
        .expect("prepare stores the public typed Grinch settings");

    let mut ids = prepared.ids.to_vec();
    ids.sort_unstable();
    assert_eq!(ids, vec![1055, ITEM]);
    assert_eq!(prepared.radius, 7);
    assert_eq!(prepared.bank.as_deref(), Some("Fixture bank"));
    assert!(Arc::ptr_eq(&prepared.banks, &banks));
}

#[test]
fn first_ready_anchor_and_exact_square_plane_and_boundary_drive_shared_reach() {
    let config = {
        let mut bag = settings(&[ITEM]);
        bag.insert("radius".into(), serde_json::json!(1));
        prepare(bag).unwrap()
    };
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut before_ready = ready_snapshot(tile(3190, 3190), Vec::new(), Vec::new(), 28);
    before_ready.seed_ingame(1);
    let before_reach = reach_view(tile(3190, 3190), &[]);
    require_continue(driver.tick(&mut *script, &before_ready, Some(&before_reach), None, 1));
    assert_eq!(script.recovery_anchor(), None);

    let boundary = tile(3217, 3216);
    let mut other_plane = boundary;
    other_plane.level = 1;
    let outside = tile(3218, 3216);
    let snapshot = ready_snapshot(
        ANCHOR,
        vec![
            ground(ITEM, boundary, 1),
            ground(ITEM, outside, 1),
            ground(ITEM, other_plane, 1),
        ],
        Vec::new(),
        28,
    );
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    assert_eq!(
        latest_obj(&driver).map(|(_, id, target)| (id, target)),
        Some((ITEM, boundary)),
        "the adjacent boundary row is a legal exact Take from the anchor",
    );
    assert!(latest_walk(&driver).is_none());
    assert_eq!(latest_obj(&driver).unwrap().2, boundary);
    assert_eq!(script.recovery_anchor(), None);
}

#[test]
fn selected_mask_identity_is_emitted_exactly_from_three_same_tile_rows() {
    let config = prepare(settings(&[1055])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let snapshot = ready_snapshot(
        ANCHOR,
        vec![
            ground(1053, ANCHOR, 1),
            ground(1055, ANCHOR, 1),
            ground(1057, ANCHOR, 1),
        ],
        Vec::new(),
        28,
    );
    let reach = reach_view(ANCHOR, &[]);
    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        1
    )));
    assert_eq!(
        latest_obj(&driver).map(|(_, id, target)| (id, target)),
        Some((1055, ANCHOR)),
    );
    assert!(driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| !matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::Obj { id: Some(id), .. }) if *id != 1055
        )));
}

#[test]
fn stale_live_zone_row_inside_old_anchor_square_is_not_selected() {
    let config = {
        let mut bag = settings(&[ITEM]);
        bag.insert("radius".into(), serde_json::json!(16));
        prepare(bag).unwrap()
    };
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let here = tile(3247, 3216);
    let anchor = tile(3231, 3216);
    let stale = tile(3215, 3216);
    let mut stale_row = ground(ITEM, stale, 1);
    stale_row.distance = 32;
    let snapshot = ready_snapshot(here, vec![stale_row], Vec::new(), 28);
    let reach = reach_view(here, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    assert_eq!(script.recovery_anchor(), Some(here));
    assert!(script.waits_for_evidence());
    assert!(latest_obj(&driver).is_none());
    assert!(driver
        .ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
    let waiting = status(&driver);
    assert_eq!(waiting.phase, NativePhase::Waiting);
    assert_eq!(text_field(waiting, "status"), "waiting for items, r=16");
    assert_ne!(anchor, here, "the currently ready tile is the Start anchor");
}

#[test]
fn idle_waits_for_fresh_relevant_evidence_and_only_publishes_changes() {
    let config = prepare(settings(&[ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = ready_snapshot(ANCHOR, Vec::new(), Vec::new(), 28);
    snapshot.seed_inventory_packet_generations(Some(1), None);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    assert!(script.waits_for_evidence());
    let status_count = driver.output.statuses.len();
    assert_eq!(status(&driver).phase, NativePhase::Waiting);
    assert_eq!(
        text_field(status(&driver), "status"),
        "waiting for items, r=12"
    );
    assert_eq!(integer_field(status(&driver), "picked"), 0);
    assert_eq!(integer_field(status(&driver), "banked"), 0);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    assert_eq!(driver.output.statuses.len(), status_count);
    snapshot.seed_ground_items(vec![ground(ITEM, ANCHOR, 1)]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 3));
    assert!(
        script.waits_for_evidence(),
        "seeded ground rows alone are not fresh evidence"
    );
    assert!(latest_obj(&driver).is_none());
    assert_eq!(driver.output.statuses.len(), status_count);

    snapshot.seed_inventory_packet_generations(Some(2), None);
    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        4
    )));
    assert!(!script.waits_for_evidence());
    assert_eq!(latest_obj(&driver).unwrap().1, ITEM);
}

#[test]
fn confirmed_pack_delta_counts_once_and_only_then_notes_progress() {
    let config = prepare(settings(&[ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = ready_snapshot(ANCHOR, vec![ground(ITEM, ANCHOR, 1)], Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[]);
    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        1
    )));
    respond_interaction(&mut driver, 1, true, 0);
    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        2
    )));
    assert_eq!(integer_field(status(&driver), "picked"), 0);

    snapshot.seed_inventory(vec![held(ITEM, 0, 1)], 28);
    snapshot.seed_ground_items(Vec::new());
    snapshot.seed_inventory_packet_generations(Some(2), None);
    assert!(require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        3
    )));
    assert_eq!(integer_field(status(&driver), "picked"), 1);
    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        4
    )));
    assert_eq!(integer_field(status(&driver), "picked"), 1);
    assert!(driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| !matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::NoteProgress)
        )));
}

#[test]
fn duplicate_refusals_exclude_only_scythe_or_bunny_and_continue_other_ids() {
    for (duplicate, refusal) in [
        (
            1419,
            "You already have a scythe, you don't need another one.",
        ),
        (1037, "You don't need another set of bunny ears."),
    ] {
        let config = prepare(settings(&[duplicate, ITEM])).unwrap();
        let mut driver = Driver::new(RUN);
        let mut script = driver.create(config);
        let mut snapshot = ready_snapshot(
            ANCHOR,
            vec![ground(duplicate, ANCHOR, 1), ground(ITEM, ANCHOR, 1)],
            Vec::new(),
            28,
        );
        let reach = reach_view(ANCHOR, &[]);
        assert!(!require_continue(driver.tick(
            &mut *script,
            &snapshot,
            Some(&reach),
            None,
            1
        )));
        assert_eq!(latest_obj(&driver).unwrap().1, duplicate);
        let previous = respond_interaction(&mut driver, 1, true, 0);
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            sequence: 1,
            type_: 0,
            username: None,
            text: refusal.into(),
        }]);
        assert!(!require_continue(driver.tick(
            &mut *script,
            &snapshot,
            Some(&reach),
            None,
            2
        )));
        assert!(text_field(status(&driver), "excluded").contains(&duplicate.to_string()));
        let log_count = driver
            .output
            .logs
            .iter()
            .filter(|message| {
                message.starts_with("excluded ") && message.contains(&duplicate.to_string())
            })
            .count();
        assert_eq!(log_count, 1, "one exclusion log per duplicate ID");

        for tick in 3..=5 {
            assert!(!require_continue(driver.tick(
                &mut *script,
                &snapshot,
                Some(&reach),
                None,
                tick
            )));
            if let Some((request, id, _)) = latest_obj(&driver) {
                if request > previous {
                    assert_eq!(id, ITEM, "the other selected ID must continue");
                    break;
                }
            }
        }
        let (request, id, _) = latest_obj(&driver).unwrap();
        assert!(request > previous);
        assert_eq!(id, ITEM);
    }
}

#[test]
fn repeated_yoyo_takes_are_legal_and_count_each_confirmed_delta() {
    let config = prepare(settings(&[4079])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = ready_snapshot(
        ANCHOR,
        vec![ground(4079, ANCHOR, 1), ground(4079, ANCHOR, 1)],
        Vec::new(),
        28,
    );
    let reach = reach_view(ANCHOR, &[]);
    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        1
    )));
    let first = latest_obj(&driver).unwrap().0;
    respond_interaction(&mut driver, 1, true, 0);
    snapshot.seed_inventory(vec![held(4079, 0, 1)], 28);
    snapshot.seed_ground_items(vec![ground(4079, ANCHOR, 1)]);
    snapshot.seed_inventory_packet_generations(Some(2), None);
    assert!(require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        2
    )));
    assert_eq!(integer_field(status(&driver), "picked"), 1);

    assert!(!require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        3
    )));
    let second = latest_obj(&driver).unwrap().0;
    assert!(second > first);
    assert_eq!(latest_obj(&driver).unwrap().1, 4079);
    respond_interaction(&mut driver, 3, true, 0);
    snapshot.seed_inventory(vec![held(4079, 0, 1), held(4079, 1, 1)], 28);
    snapshot.seed_ground_items(Vec::new());
    snapshot.seed_inventory_packet_generations(Some(3), None);
    assert!(require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        4
    )));
    assert_eq!(integer_field(status(&driver), "picked"), 2);
    assert!(!text_field(status(&driver), "excluded").contains("4079"));
}

#[test]
fn three_lost_races_do_not_fail_or_exclude_the_same_ground_key() {
    let config = prepare(settings(&[1053])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut remaining = 4;
    let mut snapshot = ready_snapshot(
        ANCHOR,
        (0..remaining).map(|_| ground(1053, ANCHOR, 1)).collect(),
        Vec::new(),
        28,
    );
    let reach = reach_view(ANCHOR, &[]);
    let mut tick = 1;
    let mut last_request = 0;
    for _ in 0..3 {
        let mut found = None;
        for candidate_tick in tick..=tick + 3 {
            require_continue(driver.tick(
                &mut *script,
                &snapshot,
                Some(&reach),
                None,
                candidate_tick,
            ));
            if let Some((request, id, at)) = latest_obj(&driver) {
                if request > last_request {
                    found = Some((request, id, at, candidate_tick));
                    break;
                }
            }
        }
        let (request, id, at, sent_at) = found.expect("fresh Take for each identical copy");
        assert_eq!((id, at), (1053, ANCHOR));
        last_request = request;
        respond_interaction(&mut driver, sent_at, true, 0);
        remaining -= 1;
        snapshot.seed_ground_items((0..remaining).map(|_| ground(1053, ANCHOR, 1)).collect());
        snapshot.seed_inventory_packet_generations(Some(remaining as u64 + 1), None);
        for edge in sent_at + 1..=sent_at + 5 {
            assert!(!require_continue(driver.tick(
                &mut *script,
                &snapshot,
                Some(&reach),
                None,
                edge,
            )));
        }
        assert_eq!(integer_field(status(&driver), "picked"), 0);
        assert!(!text_field(status(&driver), "excluded").contains("1053"));
        tick = sent_at + 6;
    }
    let mut sent_again = false;
    for next in tick..=tick + 3 {
        require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, next));
        if latest_obj(&driver).is_some_and(|(request, id, _)| request > last_request && id == 1053)
        {
            sent_again = true;
            break;
        }
    }
    assert!(
        sent_again,
        "a lost race cannot exclude a respawn or remaining copy"
    );
}

#[test]
fn three_failed_key_attempts_expire_then_exclude_only_that_id() {
    let config = prepare(settings(&[1959, 1961])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = ready_snapshot(ANCHOR, vec![ground(1959, ANCHOR, 1)], Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[]);
    let mut send_tick = 1;
    let mut last_request = 0;

    for failure in 0..3 {
        let mut found = None;
        for candidate_tick in send_tick..=send_tick + 2 {
            require_continue(driver.tick(
                &mut *script,
                &snapshot,
                Some(&reach),
                None,
                candidate_tick,
            ));
            if let Some((request, id, at)) = latest_obj(&driver) {
                if request > last_request {
                    found = Some((request, id, at, candidate_tick));
                    break;
                }
            }
        }
        let (request, id, at, sent_at) = found.expect("failed-key retry after avoid expiry");
        assert_eq!((id, at), (1959, ANCHOR));
        last_request = request;
        respond_interaction(&mut driver, sent_at, true, 0);
        for edge in sent_at + 1..=sent_at + 4 {
            assert!(!require_continue(driver.tick(
                &mut *script,
                &snapshot,
                Some(&reach),
                None,
                edge
            )));
        }
        assert_eq!(integer_field(status(&driver), "picked"), 0);
        if failure < 2 {
            assert!(script.waits_for_evidence());
            let expiry = sent_at + 14;
            for edge in sent_at + 5..expiry {
                assert!(!require_continue(driver.tick(
                    &mut *script,
                    &snapshot,
                    Some(&reach),
                    None,
                    edge,
                )));
                assert_eq!(latest_obj(&driver).unwrap().0, last_request);
            }
            send_tick = expiry + 2; // The due tick and next tick are skipped; time still wakes the idle run.
        }
    }

    let after_third_failure = send_tick.max(1) + 5;
    require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        after_third_failure,
    ));
    assert_eq!(latest_obj(&driver).unwrap().0, last_request);

    snapshot.seed_ground_items(vec![ground(1959, ANCHOR, 1), ground(1961, ANCHOR, 1)]);
    snapshot.seed_inventory_packet_generations(Some(10), None);
    let next_tick = after_third_failure + 1;
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, next_tick));
    let (request, id, at) = latest_obj(&driver).unwrap();
    assert!(request > last_request);
    assert_eq!((id, at), (1961, ANCHOR));
}

#[test]
fn recovery_anchor_is_absent_before_ready_and_during_foreground_walk() {
    let config = prepare(settings(&[ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut unready = ready_snapshot(ANCHOR, Vec::new(), Vec::new(), 28);
    unready.seed_ingame(1);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &unready, Some(&reach), None, 1));
    assert_eq!(script.recovery_anchor(), None);

    let target = tile(3218, 3216);
    let snapshot = ready_snapshot(ANCHOR, vec![ground(ITEM, target, 1)], Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[target]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    assert_eq!(script.recovery_anchor(), None);
    assert!(latest_walk(&driver).is_some());
}

#[test]
fn stop_and_death_cancel_work_and_prevent_later_sends() {
    let config = prepare(settings(&[ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(Arc::clone(&config));
    let snapshot = ready_snapshot(ANCHOR, vec![ground(ITEM, ANCHOR, 1)], Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    let sent = latest_obj(&driver).unwrap().0;
    script.on_stop(StopReason::Operator);
    assert!(!driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == sent)
        .unwrap()
        .live());
    let result = driver.tick(&mut *script, &snapshot, Some(&reach), None, 2);
    assert!(driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| action.request_id.get() <= sent));
    assert!(matches!(result.flow, Ok(ScriptFlow::Complete) | Err(_)));

    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut empty = ready_snapshot(ANCHOR, Vec::new(), Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &empty, Some(&reach), None, 1));
    empty.seed_ground_items(vec![ground(ITEM, ANCHOR, 1)]);
    empty.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
    }]);
    let death = driver.tick(&mut *script, &empty, Some(&reach), None, 2);
    assert!(matches!(&death.flow, Err(failure) if failure.code.as_ref() == "death"));
    assert!(latest_obj(&driver).is_none());
    assert!(matches!(
        driver
            .tick(&mut *script, &empty, Some(&reach), None, 3)
            .flow,
        Ok(ScriptFlow::Complete) | Err(_)
    ));
    assert!(latest_obj(&driver).is_none());
}

#[test]
fn full_pack_without_a_selected_depositable_item_fails_closed() {
    let config = prepare(settings(&[ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let snapshot = ready_snapshot(ANCHOR, Vec::new(), full_pack(None), 28);
    let reach = reach_view(ANCHOR, &[]);
    let result = driver.tick(&mut *script, &snapshot, Some(&reach), None, 1);
    assert!(matches!(&result.flow, Err(failure) if failure.code.as_ref() == "inventory-blocked"));
    assert!(!result.noted);
    assert!(driver
        .ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

fn full_inventory_snapshot(at: WorldTile, selected_id: i32) -> GameSnapshot {
    let mut snapshot = ready_snapshot(at, Vec::new(), full_pack(Some(selected_id)), 28);
    snapshot.seed_locs(vec![bank_booth(BANK_TILE)]);
    snapshot
}

#[test]
fn deposit_only_run_uses_shared_bank_observations_closes_and_returns_exactly() {
    let bank = NamedBank::new("Fixture bank", BANK_TILE);
    let banks = Arc::new(NamedBankFacts::from_banks(vec![bank]));
    let mut bag = settings(&[ITEM]);
    bag.insert("bank".into(), serde_json::json!("Nearest"));
    let config = prepare_revision(bag, Arc::clone(&banks), 1).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = full_inventory_snapshot(ANCHOR, ITEM);
    snapshot.seed_inventory_packet_generations(Some(1), Some(1));
    let reach = reach_view(ANCHOR, &[]);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    let (pick, eligible, explicit, allow_air, from) = latest_bank_pick(&driver).unwrap();
    assert!(!eligible.is_empty());
    assert_eq!(explicit, None);
    assert!(
        !allow_air,
        "native bank selection must not use air-distance fallback"
    );
    assert_eq!(from, ANCHOR);
    respond_bank_pick(&mut driver, 1, 0, bank, BANK_TILE);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    let (walk, bank_target, _) = latest_walk(&driver).expect("shared acquisition walk to bank");
    assert!(walk > pick);
    assert_eq!(bank_target, BANK_TILE);
    let (receipt, arrived_at) = respond_walk(&mut driver, 2, WalkEnd::Arrived);
    assert_eq!(receipt, walk);
    set_position(&mut snapshot, arrived_at);
    let reach = reach_view(arrived_at, &[]);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 3));
    assert!(matches!(
        &driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .effect,
        HostEffect::Interaction(InteractReq::OpenStand { .. })
    ));
    respond_interaction(&mut driver, 3, true, 0);
    set_bank_open(
        &mut snapshot,
        Vec::new(),
        vec![bank_side_row(ITEM, item_name(ITEM), 1)],
    );

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 4));
    let deposit = driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) => {
                Some((action.request_id.get(), *id))
            }
            _ => None,
        })
        .expect("shared selected-ID deposit request");
    assert_eq!(deposit.1, ITEM);
    assert!(!matches!(status(&driver).phase, NativePhase::Complete));
    respond_interaction(&mut driver, 4, true, 0);

    let mut deposited = full_pack(None);
    deposited.retain(|row| row.slot != 0);
    snapshot.seed_inventory(deposited, 28);
    snapshot.seed_inventory_packet_generations(Some(2), Some(2));
    set_bank_open(
        &mut snapshot,
        vec![bank_row(ITEM, item_name(ITEM), 1)],
        Vec::new(),
    );
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 5));
    assert_eq!(
        snapshot
            .inventory()
            .iter()
            .map(|row| row.def.id)
            .filter(|id| *id == ITEM)
            .count(),
        0
    );
    assert_eq!(
        snapshot
            .inventory()
            .iter()
            .map(|row| row.def.id)
            .filter(|id| *id == OTHER)
            .count(),
        1,
        "unselected inventory rows remain present under their original IDs"
    );
    assert!(snapshot.inventory().iter().any(|row| row.def.id == 5001));
    assert!(snapshot
        .bank()
        .iter()
        .any(|row| row.def.id == ITEM && row.count == 1));

    assert!(matches!(
        &driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .effect,
        HostEffect::Interaction(InteractReq::Close)
    ));
    assert!(latest_obj(&driver).is_none());
    respond_interaction(&mut driver, 5, true, 0);
    set_bank_closed(&mut snapshot);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 6));
    assert_eq!(integer_field(status(&driver), "banked"), 1);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 7));
    let (return_walk, target, radius) = latest_walk(&driver).expect("caller-owned anchor return");
    assert!(return_walk > walk);
    assert_eq!((target, radius), (ANCHOR, 0));
    assert!(
        latest_obj(&driver).is_none(),
        "selection waits for return arrival"
    );
    assert!(matches!(status(&driver).phase, NativePhase::Working));
    assert_eq!(text_field(status(&driver), "status"), "returning to start");

    let (return_receipt, returned_at) = respond_walk(&mut driver, 7, WalkEnd::Arrived);
    assert_eq!(return_receipt, return_walk);
    set_position(&mut snapshot, returned_at);
    let reach = reach_view(returned_at, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 8));
    assert_eq!(script.recovery_anchor(), Some(ANCHOR));
    assert_eq!(integer_field(status(&driver), "banked"), 1);
    assert_eq!(integer_field(status(&driver), "picked"), 0);
    assert!(
        script.waits_for_evidence(),
        "arrival settles into idle in the same native tick"
    );
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 9));
    assert!(script.waits_for_evidence());
}

#[test]
fn return_accepts_network_arrival_while_the_rendered_tile_lags() {
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(prepare(settings(&[ITEM])).unwrap());
    let mut snapshot = ready_snapshot(ANCHOR, Vec::new(), Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    assert!(script.waits_for_evidence());

    let adjacent = tile(ANCHOR.x - 1, ANCHOR.z);
    set_position(&mut snapshot, adjacent);
    let reach = reach_view(adjacent, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    let (request, target, radius) = latest_walk(&driver).expect("exact anchor return");
    assert_eq!((target, radius), (ANCHOR, 0));
    assert_eq!(respond_walk(&mut driver, 2, WalkEnd::Arrived).0, request);

    let mut player = crate::quester::families::tests::local_player(adjacent);
    player.player.network = ANCHOR;
    player.player.actor.moving = true;
    snapshot.seed_local_player(player);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 3));
    assert!(script.waits_for_evidence());
    assert_eq!(script.recovery_anchor(), Some(ANCHOR));
    assert_eq!(status(&driver).phase, NativePhase::Waiting);
    assert_eq!(latest_walk(&driver).unwrap().0, request);

    set_position(&mut snapshot, ANCHOR);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 4));
    assert!(script.waits_for_evidence());
    assert_eq!(latest_walk(&driver).unwrap().0, request);
}

#[test]
fn return_uses_native_walk_receipt_even_when_already_at_anchor() {
    let bank = NamedBank::new("Anchor bank", ANCHOR);
    let banks = Arc::new(NamedBankFacts::from_banks(vec![bank]));
    let mut bag = settings(&[ITEM]);
    bag.insert("bank".into(), serde_json::json!("Anchor bank"));
    let config = prepare_revision(bag, Arc::clone(&banks), 1).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = full_inventory_snapshot(ANCHOR, ITEM);
    snapshot.seed_locs(vec![bank_booth(ANCHOR)]);
    snapshot.seed_inventory_packet_generations(Some(1), Some(1));
    let reach = reach_view(ANCHOR, &[]);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    respond_bank_pick(&mut driver, 1, 0, bank, ANCHOR);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    assert!(
        latest_walk(&driver).is_none(),
        "bank access already matches the Start tile"
    );
    assert!(matches!(
        &driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .effect,
        HostEffect::Interaction(InteractReq::OpenStand { .. })
    ));
    respond_interaction(&mut driver, 2, true, 0);
    set_bank_open(
        &mut snapshot,
        Vec::new(),
        vec![bank_side_row(ITEM, item_name(ITEM), 1)],
    );

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 3));
    let deposit = driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .rev()
        .find_map(|action| match &action.effect {
            HostEffect::Interaction(InteractReq::InvButton { id, .. }) => {
                Some((action.request_id.get(), *id))
            }
            _ => None,
        })
        .expect("shared selected-ID deposit request");
    assert_eq!(deposit.1, ITEM);
    respond_interaction(&mut driver, 3, true, 0);

    let mut deposited = full_pack(None);
    deposited.retain(|row| row.slot != 0);
    snapshot.seed_inventory(deposited, 28);
    snapshot.seed_inventory_packet_generations(Some(2), Some(2));
    set_bank_open(
        &mut snapshot,
        vec![bank_row(ITEM, item_name(ITEM), 1)],
        Vec::new(),
    );
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 4));
    assert!(matches!(
        &driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .effect,
        HostEffect::Interaction(InteractReq::Close)
    ));
    respond_interaction(&mut driver, 4, true, 0);
    set_bank_closed(&mut snapshot);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 5));
    assert_eq!(integer_field(status(&driver), "banked"), 1);
    let (walk, target, radius) =
        latest_walk(&driver).expect("return starts in the settlement tick");
    assert_eq!((target, radius), (ANCHOR, 0));
    assert!(
        !script.waits_for_evidence(),
        "even on-anchor return needs its native arrival receipt"
    );
    assert_eq!(script.recovery_anchor(), None);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 6));
    assert_eq!(
        latest_walk(&driver).unwrap().0,
        walk,
        "no duplicate return while the receipt is pending"
    );
    let (receipt, arrived_at) = respond_walk(&mut driver, 6, WalkEnd::Arrived);
    assert_eq!(receipt, walk);
    assert_eq!(arrived_at, ANCHOR);
    set_position(&mut snapshot, arrived_at);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 7));
    assert_eq!(script.recovery_anchor(), Some(ANCHOR));
}

#[test]
fn named_bank_choice_is_explicit_and_no_route_has_no_air_fallback() {
    let near = NamedBank::new("Near bank", BANK_TILE);
    let far_tile = tile(3240, 3216);
    let far = NamedBank::new("Far bank", far_tile);
    let banks = Arc::new(NamedBankFacts::from_banks(vec![near, far]));
    let mut bag = settings(&[ITEM]);
    bag.insert("bank".into(), serde_json::json!("Far bank"));
    let config = prepare_revision(bag, Arc::clone(&banks), 1).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let snapshot = full_inventory_snapshot(ANCHOR, ITEM);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    let (_, eligible, explicit, allow_air, _) = latest_bank_pick(&driver).unwrap();
    assert_eq!(eligible.len(), 2);
    assert_eq!(explicit, Some(1));
    assert!(!allow_air);

    let (authority, request_id, run) = {
        let action = driver
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .rev()
            .find(|action| matches!(&action.effect, HostEffect::BankPick(_)))
            .unwrap();
        (action.authority(), action.request_id.get(), action.run())
    };
    driver.ledger.as_mut().unwrap().complete_bank_pick(
        &authority,
        BankPickReceipt {
            request_id,
            evidence: stamp(run, 1),
            selected: SelectedBank {
                bank_index: u16::MAX,
                access_tile: far_tile,
                kind: PickKind::NoCandidate,
                access: None,
            },
        },
    );
    let failure = driver.tick(&mut *script, &snapshot, Some(&reach), None, 2);
    assert!(
        failure.flow.is_err(),
        "no routable bank is an explicit failure"
    );
    assert!(
        latest_walk(&driver).is_none(),
        "never walk to an air-distance fallback"
    );
    assert!(latest_obj(&driver).is_none());
}

#[test]
fn refused_or_incomplete_full_pack_transfer_does_not_count_or_return() {
    let bank = NamedBank::new("Fixture bank", ANCHOR);
    let banks = Arc::new(NamedBankFacts::from_banks(vec![bank]));
    let config = prepare_revision(settings(&[ITEM]), Arc::clone(&banks), 1).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let mut snapshot = full_inventory_snapshot(ANCHOR, ITEM);
    snapshot.seed_locs(vec![bank_booth(ANCHOR)]);
    snapshot.seed_inventory_packet_generations(Some(1), Some(1));
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    respond_bank_pick(&mut driver, 1, 0, bank, ANCHOR);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    respond_interaction(&mut driver, 2, true, 0);
    set_bank_open(
        &mut snapshot,
        Vec::new(),
        vec![bank_side_row(ITEM, item_name(ITEM), 1)],
    );

    let mut failure = None;
    let mut refused = std::collections::HashSet::new();
    for tick in 3..=80 {
        let result = driver.tick(&mut *script, &snapshot, Some(&reach), None, tick);
        if let Err(found) = result.flow {
            failure = Some(found);
            break;
        }
        assert!(
            !result.noted,
            "no transfer evidence means no progress stamp"
        );
        if let Some(action) = driver.ledger.as_ref().and_then(|ledger| {
            ledger.outbox.iter().rev().find(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::InvButton { id: ITEM, .. })
                )
            })
        }) {
            if refused.insert(action.request_id.get()) {
                respond_interaction(&mut driver, tick, false, 0);
            }
        }
    }
    let failure = failure.expect("unsettled or refused transfer must fail closed");
    assert!(
        !refused.is_empty(),
        "the shared bank machine actually emitted a deposit"
    );
    assert_eq!(failure.code.as_ref(), "action-failed");
    assert_eq!(integer_field(status(&driver), "banked"), 0);
    assert!(
        latest_walk(&driver).is_none(),
        "a failed bank trip cannot start return"
    );
    assert!(latest_obj(&driver).is_none());
}

#[test]
fn pause_hold_and_same_run_relog_cancel_stale_children_and_keep_state() {
    let config = prepare(settings(&[1053, 1419, ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let remote = tile(3218, 3216);
    let mut snapshot = ready_snapshot(
        ANCHOR,
        vec![
            ground(1053, ANCHOR, 1),
            ground(1419, ANCHOR, 1),
            ground(ITEM, remote, 1),
        ],
        Vec::new(),
        28,
    );
    let reach = reach_view(ANCHOR, &[remote]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    assert_eq!(latest_obj(&driver).unwrap().1, 1053);
    respond_interaction(&mut driver, 1, true, 0);
    snapshot.seed_inventory(vec![held(1053, 0, 1)], 28);
    snapshot.seed_ground_items(vec![ground(1419, ANCHOR, 1), ground(ITEM, remote, 1)]);
    snapshot.seed_inventory_packet_generations(Some(2), None);
    assert!(require_continue(driver.tick(
        &mut *script,
        &snapshot,
        Some(&reach),
        None,
        2
    )));
    assert_eq!(integer_field(status(&driver), "picked"), 1);

    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 3));
    assert_eq!(latest_obj(&driver).unwrap().1, 1419);
    respond_interaction(&mut driver, 3, true, 0);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: "You already have a scythe, you don't need another one.".into(),
    }]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 4));
    assert!(text_field(status(&driver), "excluded").contains("1419"));

    let mut child_tick = 5;
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, child_tick));
    let (old_walk, _, _) = latest_walk(&driver).expect("other selected item starts Reach walk");
    assert_eq!(script.recovery_anchor(), None);
    script.interrupt(crate::native::Interrupt::Pause);
    assert!(!driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == old_walk)
        .unwrap()
        .live());
    driver.ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        goal: None,
        request_id: old_walk,
        evidence: stamp(RUN, child_tick),
        end: WalkEnd::Arrived,
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    });
    script.interrupt(crate::native::Interrupt::Resume);
    child_tick += 1;
    snapshot.seed_inventory_packet_generations(Some(3), None);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, child_tick));
    let (after_pause, _, _) = latest_walk(&driver).expect("resume revalidates and walks again");
    assert!(after_pause > old_walk);
    assert_eq!(script.recovery_anchor(), None);
    assert_eq!(integer_field(status(&driver), "picked"), 1);
    assert!(text_field(status(&driver), "excluded").contains("1419"));

    script.interrupt(crate::native::Interrupt::Hold(true));
    assert!(!driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == after_pause)
        .unwrap()
        .live());
    script.interrupt(crate::native::Interrupt::Hold(false));
    driver.ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        goal: None,
        request_id: after_pause,
        evidence: stamp(RUN, child_tick),
        end: WalkEnd::Arrived,
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    });
    child_tick += 1;
    snapshot.seed_inventory_packet_generations(Some(4), None);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, child_tick));
    let (after_hold, _, _) =
        latest_walk(&driver).expect("held child receipt cannot finish a new walk");
    assert!(after_hold > after_pause);
    assert_eq!(integer_field(status(&driver), "picked"), 1);
    assert!(text_field(status(&driver), "excluded").contains("1419"));

    script.interrupt(crate::native::Interrupt::SessionEnded);
    assert!(!driver
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == after_hold)
        .unwrap()
        .live());
    script.interrupt(crate::native::Interrupt::SessionReady);
    driver.run = RunKey { session: 2, ..RUN };
    child_tick += 1;
    snapshot.seed_inventory_packet_generations(Some(5), None);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, child_tick));
    assert_eq!(script.recovery_anchor(), None);
    let (after_relog, _, _) =
        latest_walk(&driver).expect("fresh relog evidence starts a new child");
    assert!(after_relog > after_hold);
    assert_eq!(integer_field(status(&driver), "picked"), 1);
    assert!(text_field(status(&driver), "excluded").contains("1419"));
}

#[test]
fn a_new_start_resets_exclusions_counters_and_anchor() {
    let config = prepare(settings(&[1053, 1419, ITEM])).unwrap();
    let mut first = Driver::new(RUN);
    let mut old_script = first.create(Arc::clone(&config));
    let mut snapshot = ready_snapshot(
        ANCHOR,
        vec![
            ground(1053, ANCHOR, 1),
            ground(1419, ANCHOR, 1),
            ground(ITEM, ANCHOR, 1),
        ],
        Vec::new(),
        28,
    );
    let reach = reach_view(ANCHOR, &[]);
    require_continue(first.tick(&mut *old_script, &snapshot, Some(&reach), None, 1));
    assert_eq!(latest_obj(&first).unwrap().1, 1053);
    respond_interaction(&mut first, 1, true, 0);
    snapshot.seed_inventory(vec![held(1053, 0, 1)], 28);
    snapshot.seed_ground_items(vec![ground(1419, ANCHOR, 1), ground(ITEM, ANCHOR, 1)]);
    snapshot.seed_inventory_packet_generations(Some(2), None);
    require_continue(first.tick(&mut *old_script, &snapshot, Some(&reach), None, 2));
    assert_eq!(integer_field(status(&first), "picked"), 1);

    require_continue(first.tick(&mut *old_script, &snapshot, Some(&reach), None, 3));
    assert_eq!(latest_obj(&first).unwrap().1, 1419);
    respond_interaction(&mut first, 3, true, 0);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: "You already have a scythe, you don't need another one.".into(),
    }]);
    require_continue(first.tick(&mut *old_script, &snapshot, Some(&reach), None, 4));
    assert!(text_field(status(&first), "excluded").contains("1419"));
    assert_eq!(integer_field(status(&first), "picked"), 1);

    let new_anchor = tile(3232, 3216);
    let new_snapshot = ready_snapshot(
        new_anchor,
        vec![ground(1419, new_anchor, 1), ground(ITEM, new_anchor, 1)],
        Vec::new(),
        28,
    );
    let new_reach = reach_view(new_anchor, &[]);
    let mut second = Driver::new(RunKey { run: 2, ..RUN });
    let mut new_script = second.create(config);
    require_continue(second.tick(&mut *new_script, &new_snapshot, Some(&new_reach), None, 1));
    assert_eq!(latest_obj(&second).unwrap().1, 1419);
    assert_eq!(latest_obj(&second).unwrap().2, new_anchor);
    assert!(!text_field(status(&second), "excluded").contains("1419"));
    assert_eq!(integer_field(status(&second), "picked"), 0);
}

#[test]
fn first_ready_anchor_does_not_wait_for_the_reach_flood() {
    let config = prepare(settings(&[ITEM])).unwrap();
    let mut driver = Driver::new(RUN);
    let mut script = driver.create(config);
    let snapshot = ready_snapshot(ANCHOR, vec![ground(ITEM, ANCHOR, 1)], Vec::new(), 28);
    require_continue(driver.tick(&mut *script, &snapshot, None, None, 1));
    assert_eq!(script.recovery_anchor(), Some(ANCHOR));
    assert!(script.waits_for_evidence());
    assert!(latest_obj(&driver).is_none());

    let live_reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&live_reach), None, 2));
    assert_eq!(latest_obj(&driver).unwrap().1, ITEM);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn retained_layouts_stay_within_reviewed_per_bot_budgets() {
    // The measured 64-bit runtime is 896 B; allow 128 B of layout headroom.
    const RUNTIME_BUDGET: usize = 1024;
    // Prepared is 56 B, excluding Arc/shared payloads; allow 8 B of headroom.
    const PREPARED_BUDGET: usize = 64;
    let runtime = std::mem::size_of::<super::Grinch>();
    let prepared = std::mem::size_of::<super::Prepared>();
    println!(
        "Grinch retained runtime={runtime}B budget={RUNTIME_BUDGET}B prepared={prepared}B budget={PREPARED_BUDGET}B"
    );
    assert!(runtime <= RUNTIME_BUDGET, "Grinch runtime is {runtime} B");
    assert!(
        prepared <= PREPARED_BUDGET,
        "Grinch prepared value is {prepared} B"
    );
}

#[test]
fn steady_empty_and_crowded_wait_ticks_allocate_nothing() {
    let config = prepare(settings(&[ITEM])).unwrap();
    for rows in [0, 2048] {
        let snapshot = ready_snapshot(
            ANCHOR,
            (0..rows).map(|_| ground(OTHER, ANCHOR, 1)).collect(),
            Vec::new(),
            28,
        );
        let live_reach = reach_view(ANCHOR, &[]);
        let mut driver = Driver::new(RUN);
        let mut script = driver.create(Arc::clone(&config));
        require_continue(driver.tick(&mut *script, &snapshot, Some(&live_reach), None, 1));
        assert!(script.waits_for_evidence());
        let statuses = driver.output.statuses.len();
        let measured = allocation_counter::measure(|| {
            for tick in 2..102 {
                assert!(!require_continue(driver.tick(
                    &mut *script,
                    &snapshot,
                    Some(&live_reach),
                    None,
                    tick,
                )));
            }
        });
        println!(
            "Grinch rows={rows} idle_ticks=100 allocations={} retained={}B runtime={}B prepared={}B",
            measured.count_total,
            measured.bytes_current,
            std::mem::size_of::<super::Grinch>(),
            std::mem::size_of::<super::Prepared>(),
        );
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.bytes_current, 0);
        assert_eq!(driver.output.statuses.len(), statuses);
        assert!(driver.ledger.is_none());
    }
}

#[test]
fn debug_state_tracks_actual_owner_and_failed_key_without_idle_churn() {
    const CHILD: &str = "GRINCH_DEBUG_STATE_TEST_CHILD";
    const MARKER: &str = "GRINCH_DEBUG_STATE_TEST_OK";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "grinch::tests::debug_state_tracks_actual_owner_and_failed_key_without_idle_churn",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("BOT_DEBUG", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "diagnostic child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(MARKER));
        return;
    }
    fn last(driver: &Driver, prefix: &str) -> serde_json::Value {
        serde_json::from_str(
            driver
                .output
                .logs
                .iter()
                .rev()
                .find_map(|line| line.strip_prefix(prefix))
                .expect("actual native diagnostic"),
        )
        .unwrap()
    }

    let mut driver = Driver::new(RUN);
    let mut script = driver.create(prepare(settings(&[ITEM])).unwrap());
    let mut snapshot = ready_snapshot(ANCHOR, Vec::new(), Vec::new(), 28);
    let reach = reach_view(ANCHOR, &[]);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 1));
    let initial = last(&driver, "GRINCH_STATE ");
    assert_eq!(
        initial["run"],
        serde_json::json!({"slot": 1, "run": 1, "session": 1})
    );
    assert_eq!(
        initial["anchor"],
        serde_json::json!({"x": 3216, "z": 3216, "level": 0})
    );
    assert_eq!(initial["phase"], "waiting");
    assert_eq!(initial["foreground_children"], 0);
    assert_eq!(initial["failed_keys"], serde_json::json!([]));
    assert_eq!(initial["id_exclusions"], serde_json::json!([]));
    let idle_logs = driver.output.logs.len();
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 2));
    assert_eq!(driver.output.logs.len(), idle_logs);

    snapshot.seed_ground_items(vec![ground(ITEM, ANCHOR, 1)]);
    snapshot.seed_inventory_packet_generations(Some(3), None);
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 3));
    assert!(latest_obj(&driver).is_some());
    let active = last(&driver, "GRINCH_STATE ");
    assert_eq!(active["phase"], "taking");
    assert_eq!(active["foreground_children"], 1);
    respond_interaction(&mut driver, 3, true, 0);
    for tick in 4..=7 {
        require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, tick));
    }
    let settled = last(&driver, "GRINCH_TAKE ");
    assert_eq!(settled["tick"], 7);
    assert_eq!(settled["item_id"], ITEM);
    assert_eq!(settled["outcome"], "failed_key");
    assert_eq!(settled["units"], 0);
    let failed = last(&driver, "GRINCH_STATE ");
    assert_eq!(failed["phase"], "waiting");
    assert_eq!(failed["foreground_children"], 0);
    assert_eq!(failed["failed_keys"][0]["item_id"], ITEM);
    assert_eq!(failed["failed_keys"][0]["failures"], 1);
    assert_eq!(failed["failed_keys"][0]["last_failure_tick"], 7);
    assert_eq!(failed["failed_keys"][0]["excluded"], false);
    let cooling_logs = driver.output.logs.len();
    require_continue(driver.tick(&mut *script, &snapshot, Some(&reach), None, 8));
    assert_eq!(driver.output.logs.len(), cooling_logs);
    println!("{MARKER}");
}
