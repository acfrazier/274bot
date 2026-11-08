use super::tests::{status_fixture, survival_condition, survival_step_fixture, SurvivalTestCounts};
use super::*;
use crate::combat::{ClearPrayers, ClearPrayersArgs, RaisedPrayers};
use crate::native::{
    ledger, ActionHandle, HostEffect, InteractionReceipt, NativeOutput, NativeTick, Script,
    ScriptFlow, ScriptStatus,
};
use crate::paint::ScriptPaint;
use crate::quester::compile::{CompiledStep, PredicatePlan, StepPlan};
use crate::quester::path::{
    HazardDocument, PathDocument, PredicateDocument, SequenceOrder, StepDocument,
};
use crate::survival::{CurePoison, HazardStat, StepHazard};
use api::condition::PoisonState;
use api::hostlog::Level;
use api::selected::{FactKey, Truth};
use api::snapshot::{
    ChatLineView, GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily, ItemContainer,
    ItemView, LocLayer, LocView, StatView, WorldStateView,
};
use api::WorldTile;
use client::client::{Client, ClientConfig};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::io::ServerProt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;

const RAILING_HAZARD: StepHazard = StepHazard {
    of: HazardStat::Base,
    mul: 1,
    div: 10,
    plus: 0,
    cap: Some(3),
};
const RAILING_SOURCE: &str = "quest_mcannon/scripts/locs/railings.rs2:87-91";

#[derive(Default)]
struct CapturedOutput {
    logs: Vec<(Level, String)>,
    interactions: Vec<ObservedInteraction>,
}

struct ObservedInteraction {
    ledger: usize,
    request_id: u64,
    request: crate::interact::InteractReq,
}

impl NativeOutput for CapturedOutput {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<ScriptPaint>) {}
    fn log(&mut self, level: Level, message: &str) {
        self.logs.push((level, message.to_owned()));
    }
    fn settings_applied(&mut self, _: u64) {}
}

struct NeverPredicate;
impl PredicatePlan for NeverPredicate {
    fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
        Truth::False
    }
}

struct AlwaysPredicate;
impl PredicatePlan for AlwaysPredicate {
    fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
        Truth::True
    }
}

struct AtomicPredicate(Arc<AtomicBool>);
impl PredicatePlan for AtomicPredicate {
    fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
        if self.0.load(Ordering::Relaxed) {
            Truth::True
        } else {
            Truth::False
        }
    }
}

struct TogglePlan {
    complete: Arc<AtomicBool>,
    began: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
}
impl StepPlan for TogglePlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        self.began.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(ToggleRun {
            complete: Arc::clone(&self.complete),
            cancelled: Arc::clone(&self.cancelled),
        }))
    }

    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}

struct ToggleRun {
    complete: Arc<AtomicBool>,
    cancelled: Arc<AtomicUsize>,
}
impl StepRun for ToggleRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if self.complete.load(Ordering::Relaxed) {
            Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            }))
        } else {
            Poll::Pending
        }
    }

    fn cancel(&mut self, _: &mut crate::native::NativeActions) {
        self.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}

struct ImmediatePlan(Arc<AtomicUsize>);
impl StepPlan for ImmediatePlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(ImmediateRun))
    }
}

struct ImmediateRun;
impl StepRun for ImmediateRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
    }

    fn cancel(&mut self, _: &mut crate::native::NativeActions) {}
}

struct PendingPlan(Arc<AtomicUsize>);
impl StepPlan for PendingPlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(PendingRun))
    }
}

struct PendingRun;
impl StepRun for PendingRun {
    fn poll(&mut self, _: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Pending
    }

    fn cancel(&mut self, _: &mut crate::native::NativeActions) {}
}

fn compiled_step(
    id: &str,
    kind: &str,
    plan: Arc<dyn StepPlan>,
    hazard: Option<StepHazard>,
    settle: Arc<dyn PredicatePlan>,
) -> CompiledStep {
    CompiledStep {
        id: FactKey::new(id),
        kind: Arc::from(kind),
        comment: None,
        loadout: None,
        tactic: None,
        advances: false,
        skip_if: Arc::new(NeverPredicate),
        skip_if_summary: Arc::from("runner test step"),
        settle,
        plan,
        hazard,
        hazard_source: hazard.map(|_| Arc::from(RAILING_SOURCE)),
        attempt_budget: None,
    }
}

fn replace_steps(script: &mut Quester, steps: Vec<CompiledStep>) {
    let path = Arc::get_mut(&mut script.path).expect("test path is not shared");
    path.sequences[script.seq_index].order = SequenceOrder::Ordered;
    path.sequences[script.seq_index].steps = steps;
    script.step_index = 0;
    script.cursor = 0;
    script.needs_read = false;
    script.settling = false;
    script.step = None;
}

fn remove_first_step_hazard(script: &mut Quester) {
    let path = Arc::get_mut(&mut script.path).expect("test path is not shared");
    let step = &mut path.sequences[script.seq_index].steps[0];
    step.hazard = None;
    step.hazard_source = None;
}

fn fixture(
    kind: &str,
    hazard: Option<StepHazard>,
) -> (Quester, GameSnapshot, Arc<SurvivalTestCounts>) {
    let (mut script, snapshot, counts) = survival_step_fixture(kind);
    let path = Arc::get_mut(&mut script.path).expect("test path is not shared");
    let step = &mut path.sequences[script.seq_index].steps[0];
    step.hazard = hazard;
    step.hazard_source = hazard.map(|_| Arc::from(RAILING_SOURCE));
    (script, snapshot, counts)
}

fn seed_frame(snapshot: &mut GameSnapshot, tick: u64, hp: u8, hp_max: u8) {
    let at = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    snapshot.seed_tick(tick as u32);
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView {
        map_base_x: 3200,
        map_base_z: 3200,
        members: true,
        ..WorldStateView::default()
    });
    snapshot.seed_local_player(super::super::families::tests::local_player(at));
    snapshot.seed_stats(vec![StatView {
        index: 3,
        name: "hitpoints".into(),
        base: i32::from(hp_max),
        effective: i32::from(hp),
        xp: 0,
        used: true,
    }]);
    snapshot.seed_equipment(Vec::new());
    clear_hit(snapshot, 600 + (tick as i32 * 30));
}

fn clear_hit(snapshot: &mut GameSnapshot, loop_cycle: i32) {
    snapshot.seed_hitmarks(HitmarksView {
        marks: [HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        }; 4],
        loop_cycle,
    });
}

fn seed_hit(snapshot: &mut GameSnapshot, damage: i32, loop_cycle: i32) {
    let mut marks = [HitmarkView {
        value: 0,
        kind: 0,
        cycle: 0,
    }; 4];
    marks[0] = HitmarkView {
        value: damage,
        kind: 1,
        cycle: loop_cycle + 30,
    };
    snapshot.seed_hitmarks(HitmarksView { marks, loop_cycle });
}

#[derive(Clone, Copy)]
struct TestFrame {
    tick: u64,
    hp: u8,
    hp_max: u8,
    poison: PoisonState,
    cure: CurePoison,
}

fn with_frame_tick<R>(
    script: &mut Quester,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    frame: TestFrame,
    output: &mut CapturedOutput,
    f: impl FnOnce(&mut Quester, &mut NativeTick<'_>) -> R,
) -> R {
    let TestFrame {
        tick,
        hp,
        hp_max,
        poison,
        cure,
    } = frame;
    api::hostlog::set_tick(tick as u32);
    let result =
        super::super::families::tests::with_tick_output(snapshot, ledger, tick, output, |native| {
            let mut condition = survival_condition(poison, hp);
            condition.tick = tick as u32;
            condition.hp_max = hp_max;
            native.cx.condition = Some(condition);
            native.cx.cure_poison = cure;
            native.cx.poison_unknown_pulse = 11;
            f(script, native)
        });
    // A cancelled owner clears its outbox. Retain each real request once
    // before the next poll can retire it, separated by its test-owned ledger.
    if let Some(ledger) = ledger.as_deref() {
        let key = std::ptr::from_ref(ledger) as usize;
        for action in &ledger.outbox {
            if let HostEffect::Interaction(request) = &action.effect {
                let request_id = action.authority().request_id().get();
                if !output
                    .interactions
                    .iter()
                    .any(|observed| observed.ledger == key && observed.request_id == request_id)
                {
                    output.interactions.push(ObservedInteraction {
                        ledger: key,
                        request_id,
                        request: request.clone(),
                    });
                }
            }
        }
    }
    result
}

fn with_runner_tick<R>(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    frame: TestFrame,
    output: &mut CapturedOutput,
    f: impl FnOnce(&mut Quester, &mut NativeTick<'_>) -> R,
) -> R {
    seed_frame(snapshot, frame.tick, frame.hp, frame.hp_max);
    with_frame_tick(script, snapshot, ledger, frame, output, f)
}

fn with_runner_hit<R>(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    frame: TestFrame,
    damage: i32,
    output: &mut CapturedOutput,
    f: impl FnOnce(&mut Quester, &mut NativeTick<'_>) -> R,
) -> R {
    seed_frame(snapshot, frame.tick, frame.hp, frame.hp_max);
    seed_hit(snapshot, damage, 600 + (frame.tick as i32 * 30));
    with_frame_tick(script, snapshot, ledger, frame, output, f)
}

fn drive(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    frame: TestFrame,
    output: &mut CapturedOutput,
) -> ScriptFlow {
    with_runner_tick(script, snapshot, ledger, frame, output, |script, native| {
        script.tick(native).expect("Quester tick")
    })
}

fn assert_no_stale_hazard_eat(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    output: &mut CapturedOutput,
    tick: u64,
) {
    let flow = drive(
        script,
        snapshot,
        ledger,
        TestFrame {
            tick,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        output,
    );
    assert!(
        matches!(flow, ScriptFlow::Continue),
        "a low-HP poll after clearing a hazard must not stop the run"
    );
    assert_eq!(
        eat_count(ledger, output),
        0,
        "the old hazard must not raise the eat line"
    );
    assert!(
        script.survival_action.is_none(),
        "a cleared hazard must not start a survival action"
    );
    assert_eq!(
        script.survival.report().decision,
        crate::survival::Decision::None,
        "the low-HP frame stays below the ordinary eat line"
    );
}

fn drive_with_hit(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    frame: TestFrame,
    damage: i32,
    output: &mut CapturedOutput,
) -> ScriptFlow {
    with_runner_hit(
        script,
        snapshot,
        ledger,
        frame,
        damage,
        output,
        |script, native| script.tick(native).expect("Quester tick"),
    )
}

fn accept_latest_interaction(tick: &mut NativeTick<'_>) {
    let evidence = tick.cx.evidence();
    let authority = tick
        .cx
        .ledger
        .as_ref()
        .expect("action ledger")
        .outbox
        .iter()
        .rev()
        .find(|action| action.live() && matches!(&action.effect, HostEffect::Interaction(_)))
        .expect("live interaction request")
        .authority();
    tick.cx.ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence,
            accepted: true,
            chat_since: 0,
        },
    );
}

fn interaction_count(
    ledger: &Option<Box<ledger::Ledger>>,
    output: &CapturedOutput,
    predicate: impl Fn(&crate::interact::InteractReq) -> bool,
) -> usize {
    let key = ledger
        .as_deref()
        .map(|ledger| std::ptr::from_ref(ledger) as usize);
    output
        .interactions
        .iter()
        .filter(|observed| Some(observed.ledger) == key && predicate(&observed.request))
        .count()
}

fn eat_count(ledger: &Option<Box<ledger::Ledger>>, output: &CapturedOutput) -> usize {
    interaction_count(
        ledger,
        output,
        |request| matches!(request, crate::interact::InteractReq::Held { action, .. } if action == "Eat"),
    )
}

fn close_count(ledger: &Option<Box<ledger::Ledger>>, output: &CapturedOutput) -> usize {
    interaction_count(ledger, output, |request| {
        matches!(request, crate::interact::InteractReq::CloseModal)
    })
}

fn live_close_count(ledger: &Option<Box<ledger::Ledger>>) -> usize {
    ledger.as_ref().map_or(0, |ledger| {
        ledger
            .outbox
            .iter()
            .filter(|action| {
                action.live()
                    && matches!(
                        &action.effect,
                        HostEffect::Interaction(crate::interact::InteractReq::CloseModal)
                    )
            })
            .count()
    })
}

fn count_answers(ledger: &Option<Box<ledger::Ledger>>, output: &CapturedOutput) -> Vec<i32> {
    let key = ledger
        .as_deref()
        .map(|ledger| std::ptr::from_ref(ledger) as usize);
    output
        .interactions
        .iter()
        .filter_map(|observed| {
            if Some(observed.ledger) != key {
                return None;
            }
            match &observed.request {
                crate::interact::InteractReq::AnswerCount { value } => Some(*value),
                _ => None,
            }
        })
        .collect()
}

fn warning_lines(output: &CapturedOutput) -> Vec<&str> {
    output
        .logs
        .iter()
        .filter_map(|(level, message)| {
            (*level == Level::Warn && message.starts_with("quester undeclared damage "))
                .then_some(message.as_str())
        })
        .collect()
}

fn hazard_bound_lines(output: &CapturedOutput) -> Vec<&str> {
    output
        .logs
        .iter()
        .filter_map(|(level, message)| {
            (*level == Level::Info && message.starts_with("quester hazard park "))
                .then_some(message.as_str())
        })
        .collect()
}

fn survival_unpark_count(slot: &str) -> usize {
    crate::slot::compiled::tests::status_lines_for(slot)
        .iter()
        .filter(|line| line.message.contains("survival unpark"))
        .count()
}

fn count_item(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn chat_page(snapshot: &mut GameSnapshot, root: i32) {
    snapshot.seed_chat_modal(root, Vec::new());
    snapshot.seed_chat_options(Vec::new(), -1);
}

fn main_page(snapshot: &mut GameSnapshot, client: &mut Client, root: i32) {
    client.main_modal_id = root;
    client.bump_gens(if root >= 0 {
        ServerProt::IF_OPENMAIN
    } else {
        ServerProt::IF_CLOSE
    });
    snapshot.rebuild_family(client, api::snapshot::Family::Modals);
}

fn begin_clear_owner(
    script: &mut Quester,
    native: &mut NativeTick<'_>,
    owned: RaisedPrayers,
) -> ActionHandle<ClearPrayers> {
    let handle = native
        .actions
        .begin::<ClearPrayers>(
            ClearPrayersArgs::owned(Arc::clone(&script.selected), owned),
            &mut native.cx,
        )
        .expect("begin real ClearPrayers machine");
    assert!(native.actions.poll(&handle, &mut native.cx).is_pending());
    handle
}

fn owned_prayer(script: &Quester) -> RaisedPrayers {
    let prayer = script
        .selected
        .prayers()
        .first()
        .expect("selected prayer row");
    let mut owned = RaisedPrayers::empty();
    owned.accepted(prayer.varp, true, 0);
    owned
}

fn active_prayer_varp(script: &Quester) -> api::snapshot::VarpView {
    api::snapshot::VarpView {
        index: script
            .selected
            .prayers()
            .first()
            .expect("selected prayer row")
            .varp,
        value: 1,
    }
}

fn item_row(id: i32, name: &str, count: i32, slot: i32) -> ItemView {
    ItemView {
        def: super::super::families::tests::def(id, name),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count,
        actions: Vec::new(),
        component_id: 0,
    }
}

fn selected_item_id(script: &Quester, alias: &str) -> i32 {
    script
        .selected
        .item_by_alias(alias)
        .expect("selected item alias")
        .id
}

fn lobster_id(script: &Quester) -> i32 {
    selected_item_id(script, "lobster")
}

fn bar_id(script: &Quester) -> i32 {
    selected_item_id(script, "bronze_bar")
}

fn antipoison_id() -> i32 {
    crate::combat::guard::shared_tables()
        .expect("combat tables")
        .potion(crate::combat::tables::PotionKind::Antipoison)
        .expect("antipoison table")
        .doses[0]
        .expect("antipoison dose")
        .id
}

fn set_inventory(script: &Quester, snapshot: &mut GameSnapshot, bars: i32, food: i32, doses: i32) {
    let mut rows = Vec::new();
    let mut slot = 0;
    if bars > 0 {
        rows.push(item_row(bar_id(script), "Bronze bar", bars, slot));
        slot += 1;
    }
    if food > 0 {
        rows.push(item_row(lobster_id(script), "Lobster", food, slot));
        slot += 1;
    }
    if doses > 0 {
        rows.push(item_row(antipoison_id(), "Antipoison", doses, slot));
    }
    snapshot.seed_inventory(rows, 28);
}

fn make_client(product_id: i32, product_name: &str) -> Client {
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp/274bot-upkeep-make-fixture".into(),
        members: true,
        lowmem: false,
    });
    let cache = Arc::get_mut(&mut client.cache).expect("new client cache is unique");
    cache
        .objs
        .resize(product_id as usize + 1, client::config::ObjType::default());
    cache.objs[product_id as usize] = client::config::ObjType {
        id: product_id,
        name: product_name.into(),
        ..Default::default()
    };
    client.set_iface(
        2100,
        IfType {
            id: 2100,
            layer_id: 2100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2110, 2120]),
            ..Default::default()
        },
    );
    client.set_iface(
        2110,
        IfType {
            id: 2110,
            layer_id: 2100,
            r#type: ComponentType::TYPE_MODEL,
            ..Default::default()
        },
    );
    client.set_iface_mut(
        2110,
        IfTypeMut {
            model1_type: 4,
            model1_id: product_id,
            ..Default::default()
        },
    );
    client.set_iface(
        2120,
        IfType {
            id: 2120,
            layer_id: 2100,
            r#type: ComponentType::TYPE_TEXT,
            button_text: "Make X".into(),
            ..Default::default()
        },
    );
    client.set_iface_mut(
        2120,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );
    client
}

fn make_loc(script: &Quester) -> LocView {
    let loc = script
        .selected
        .loc_by_config("furnace1")
        .expect("selected furnace1 config");
    let at = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    LocView {
        typecode: loc.id << 6,
        info: 0,
        id: loc.id,
        name: Some(loc.display.as_deref().unwrap_or("Furnace").to_owned()),
        description: None,
        actions: vec![Some("Smelt".to_owned())],
        tile: at,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 10,
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

fn install_make_document(document: &mut PathDocument) {
    let role = document.roles.first_mut().expect("Cook role");
    let not_started = role
        .progress
        .as_ref()
        .expect("Cook progress")
        .colour
        .not_started
        .clone();
    let sequence = role
        .sequences
        .iter_mut()
        .find(|sequence| sequence.stage == not_started)
        .expect("not-started sequence");
    sequence.order = SequenceOrder::Ordered;
    sequence.steps = vec![StepDocument {
        id: FactKey::new("make-five-bars"),
        kind: "make".to_owned(),
        version: 1,
        args: serde_json::json!({
            "loc": {"name": "furnace1", "op": "Smelt"},
            "anchor": {"tile": [3200, 3200, 0], "source": "runner test anchor"},
            "product": "bronze_bar",
            "menu": {"obj": "bronze_bar", "source": "runner test Make-X menu"},
            "qty": 5,
            "make_x": true
        }),
        comment: None,
        advances: Some(false),
        max_attempts: None,
        stop: None,
        skip_if: PredicateDocument::Any(Vec::new()),
        settle: PredicateDocument::Fact {
            kind: "item_count_at_least".to_owned(),
            version: 1,
            args: serde_json::json!({"obj": "bronze_bar", "qty": 5}),
        },
        hazard: None::<HazardDocument>,
    }];
}

#[test]
fn quester_prices_the_hazard_from_selection_until_another_step_is_selected() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = survival_step_fixture("interact");
    let completes = Arc::new(AtomicBool::new(false));
    let began = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let settle_gate = Arc::new(AtomicBool::new(false));
    let first = compiled_step(
        "hazard-step-1",
        "interact",
        Arc::new(TogglePlan {
            complete: Arc::clone(&completes),
            began: Arc::clone(&began),
            cancelled: Arc::clone(&cancelled),
        }),
        Some(RAILING_HAZARD),
        Arc::new(AtomicPredicate(Arc::clone(&settle_gate))),
    );
    let second_began = Arc::new(AtomicUsize::new(0));
    let second = compiled_step(
        "plain-step-2",
        "talk",
        Arc::new(PendingPlan(Arc::clone(&second_began))),
        None,
        Arc::new(NeverPredicate),
    );
    replace_steps(&mut script, vec![first, second]);
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        began.load(Ordering::Relaxed),
        0,
        "the selected hazard's first begin is deferred"
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));

    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(began.load(Ordering::Relaxed), 1);
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        eat_count(&ledger, &output),
        1,
        "the selected hazard prices the first low-HP poll"
    );
    assert!(
        script.step.is_none(),
        "the bite cancels the active first step"
    );
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        began.load(Ordering::Relaxed),
        2,
        "the count drop restarts step 1"
    );

    completes.store(true, Ordering::Relaxed);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 24,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.settling,
        "step 1 has completed but its settle predicate is still false"
    );
    for tick in 25..=49 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 40,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert!(
            script.settling,
            "the hazard stays selected during settlement at tick {tick}"
        );
        assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));
    }
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 50,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        eat_count(&ledger, &output),
        2,
        "the hazard remains priced while the completed step settles"
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));

    settle_gate.store(true, Ordering::Relaxed);
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 51,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 52,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 53,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        script.selected_hazard.slot,
        Some(StepSlot {
            in_prelude: false,
            seq_index: script.seq_index,
            step_index: 1
        })
    );
    assert_eq!(script.selected_hazard.hazard, None);
    assert!(script.step.is_some(), "the second step is active");
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 54,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival_action.is_none());
    assert!(script.survival.parked().is_none());
    assert_eq!(
        eat_count(&ledger, &output),
        2,
        "the old hazard does not leak into step 2"
    );
}

#[test]
fn food_park_on_a_hazard_step_holds_across_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "new hazard selection defers begin"
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "the step remains unbegun at the Need edge"
    );
    for tick in 22..=71 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert!(
            script.survival.parked().is_some(),
            "park released at tick {tick}"
        );
        assert_eq!(
            counts.began.load(Ordering::Relaxed),
            0,
            "step began while parked at tick {tick}"
        );
    }
}

#[test]
fn regeneration_to_the_line_plus_one_releases_the_park_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    const HOSTLOG_SLOT: &str = "status-upkeep-r1b";
    crate::slot::compiled::tests::install_status_line_sink();
    api::hostlog::bind_slot(HOSTLOG_SLOT);
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "the park begins before any step attempt"
    );
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 121,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| script.poll_survival(native),
    );
    assert!(script.survival.parked().is_none());
    assert!(script.needs_read, "unpark requests a fresh progress read");
    assert_eq!(survival_unpark_count(HOSTLOG_SLOT), 1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 122,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(survival_unpark_count(HOSTLOG_SLOT), 1);
}

#[test]
fn food_that_appears_while_parked_is_eaten() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival_action.is_some());
    assert_eq!(
        script.survival.report().decision,
        crate::survival::Decision::Eating
    );
    assert_eq!(eat_count(&ledger, &output), 1);
}

#[test]
fn a_dropped_bite_keeps_its_hazard_until_the_retry_settles() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 0,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 1,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 2,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(eat_count(&ledger, &output), 1);
    for tick in 3..=9 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert_eq!(eat_count(&ledger, &output), 1);
    }
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 10,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        eat_count(&ledger, &output),
        2,
        "the unapplied first bite retries on its deadline tick"
    );
    assert_eq!(
        script.selected_hazard.hazard,
        Some(RAILING_HAZARD),
        "the retry still uses the selected hazard's line 7"
    );
    assert_eq!(
        script.survival.report().decision,
        crate::survival::Decision::Eating
    );
    assert!(script.survival_action.is_some());
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 11,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.survival_action.is_none(),
        "the count drop settles the retry"
    );
    assert_eq!(eat_count(&ledger, &output), 2);
}

#[test]
fn a_real_step_transition_removes_the_old_hazard() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = survival_step_fixture("interact");
    let first_begins = Arc::new(AtomicUsize::new(0));
    let second_begins = Arc::new(AtomicUsize::new(0));
    let first = compiled_step(
        "hazard-step-1",
        "interact",
        Arc::new(ImmediatePlan(Arc::clone(&first_begins))),
        Some(RAILING_HAZARD),
        Arc::new(AlwaysPredicate),
    );
    let second = compiled_step(
        "plain-step-2",
        "talk",
        Arc::new(PendingPlan(Arc::clone(&second_begins))),
        None,
        Arc::new(NeverPredicate),
    );
    replace_steps(&mut script, vec![first, second]);
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));
    assert_eq!(
        first_begins.load(Ordering::Relaxed),
        0,
        "selection defers the first begin"
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(first_begins.load(Ordering::Relaxed), 1);
    assert_eq!(
        second_begins.load(Ordering::Relaxed),
        1,
        "step 1 settles and step 2 starts"
    );
    assert_eq!(
        script.selected_hazard.slot,
        Some(StepSlot {
            in_prelude: false,
            seq_index: script.seq_index,
            step_index: 1
        })
    );
    assert_eq!(script.selected_hazard.hazard, None);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        script.survival.report().decision,
        crate::survival::Decision::None
    );
    assert!(script.survival_action.is_none());
    assert!(script.survival.parked().is_none());
}

#[test]
fn quester_defers_the_begin_of_a_newly_selected_hazard_step_one_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "selection defers the step begin"
    );
    assert!(
        script.begun.is_empty(),
        "selection emits no begin trace on tick t"
    );
    assert!(script.survival_deferred);
    assert_eq!(eat_count(&ledger, &output), 0);

    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 2,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(counts.began.load(Ordering::Relaxed), 0);
    assert!(
        script.begun.is_empty(),
        "the t+1 survival click still does not begin the step"
    );
    assert_eq!(
        eat_count(&ledger, &output),
        1,
        "the first post-selection poll begins the bite"
    );
    assert!(script.survival_action.is_some());
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        1,
        "the first Nothing tick after count drop begins the step"
    );
    assert!(script.step.is_some());
    assert!(!script.survival_deferred);
    let selected = script.selected_hazard.slot;
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(counts.began.load(Ordering::Relaxed), 1);
    assert_eq!(
        script.selected_hazard.slot, selected,
        "the same slot does not defer a second time"
    );

    let (mut held, mut held_snapshot, held_counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut held_ledger = None;
    let mut held_output = CapturedOutput::default();
    set_inventory(&held, &mut held_snapshot, 0, 1, 1);
    drive(
        &mut held,
        &mut held_snapshot,
        &mut held_ledger,
        TestFrame {
            tick: 30,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::WhenNeeded,
        },
        &mut held_output,
    );
    assert_eq!(held_counts.began.load(Ordering::Relaxed), 0);
    assert!(held.begun.is_empty());
    drive(
        &mut held,
        &mut held_snapshot,
        &mut held_ledger,
        TestFrame {
            tick: 31,
            hp: 2,
            hp_max: 40,
            poison: PoisonState::Onset {
                since: 31,
                q: Some(11),
                hit_since_line: false,
            },
            cure: CurePoison::WhenNeeded,
        },
        &mut held_output,
    );
    assert!(
        matches!(held_ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect), Some(HostEffect::Interaction(crate::interact::InteractReq::Held { action, .. })) if action == "Drink")
    );
    assert_eq!(held_counts.began.load(Ordering::Relaxed), 0);
    assert!(
        held.begun.is_empty(),
        "drinking at the click lock does not emit a begin trace"
    );
    set_inventory(&held, &mut held_snapshot, 0, 1, 0);
    for tick in 32..=33 {
        drive(
            &mut held,
            &mut held_snapshot,
            &mut held_ledger,
            TestFrame {
                tick,
                hp: 40,
                hp_max: 40,
                poison: PoisonState::Immune { until_lower: 80 },
                cure: CurePoison::WhenNeeded,
            },
            &mut held_output,
        );
        assert!(
            held.survival_deferred,
            "the drink lock returns Hold at tick {tick}"
        );
        assert_eq!(
            held_counts.began.load(Ordering::Relaxed),
            0,
            "Hold never begins the hazard step"
        );
    }
    assert!(
        held.begun.is_empty(),
        "Hold ticks do not emit a begin trace"
    );
    drive(
        &mut held,
        &mut held_snapshot,
        &mut held_ledger,
        TestFrame {
            tick: 34,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Immune { until_lower: 80 },
            cure: CurePoison::WhenNeeded,
        },
        &mut held_output,
    );
    assert_eq!(
        held_counts.began.load(Ordering::Relaxed),
        1,
        "the first Nothing tick after the three-tick lock begins the step"
    );
    assert!(!held.survival_deferred);
}

#[test]
fn quester_refuses_a_hazard_the_account_cannot_clear_at_full_hitpoints() {
    if !api::test_support::game_data_available() {
        return;
    }
    let lethal = StepHazard {
        of: HazardStat::None,
        mul: 0,
        div: 1,
        plus: 15,
        cap: None,
    };
    let (mut script, mut snapshot, counts) = fixture("interact", Some(lethal));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    let flow = drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 10,
            hp_max: 10,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        matches!(flow, ScriptFlow::Blocked(failure) if failure.code.as_ref() == "hazard-unsurvivable")
    );
    assert_eq!(
        script.last_error_kind,
        QuesterFailureKind::HazardUnsurvivable
    );
    assert_eq!(counts.began.load(Ordering::Relaxed), 0);
    assert!(script.step.is_none());

    let (mut safe, mut safe_snapshot, safe_counts) = fixture("interact", Some(lethal));
    let mut safe_ledger = None;
    let mut safe_output = CapturedOutput::default();
    set_inventory(&safe, &mut safe_snapshot, 0, 1, 0);
    drive(
        &mut safe,
        &mut safe_snapshot,
        &mut safe_ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut safe_output,
    );
    assert_eq!(
        safe_counts.began.load(Ordering::Relaxed),
        0,
        "the safe selection still defers its begin"
    );
    drive(
        &mut safe,
        &mut safe_snapshot,
        &mut safe_ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut safe_output,
    );
    assert_eq!(safe_counts.began.load(Ordering::Relaxed), 1);
    assert!(safe.step.is_some());
}
#[test]
fn quester_waits_for_hitpoints_before_admitting_a_hazard_step() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    seed_frame(&mut snapshot, 20, 40, 40);
    snapshot.seed_stats(Vec::new());

    let flow = with_frame_tick(
        &mut script,
        &snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| script.tick(native).expect("Quester tick"),
    );
    assert!(
        matches!(flow, ScriptFlow::Continue),
        "a missing Hitpoints stat must not stop the run"
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));
    assert!(!script.selected_hazard.handled);
    assert_eq!(counts.began.load(Ordering::Relaxed), 0);
    assert!(
        output
            .logs
            .iter()
            .all(|(_, line)| !line.starts_with("quester hazard step=")),
        "admission waits until the Hitpoints stat is present"
    );

    let flow = drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(matches!(flow, ScriptFlow::Continue));
    assert!(script.selected_hazard.handled);
    assert!(script.survival_deferred);
    assert_eq!(counts.began.load(Ordering::Relaxed), 0);
    assert_eq!(
        output
            .logs
            .iter()
            .filter(|(_, line)| line.starts_with("quester hazard step="))
            .count(),
        1,
        "the first Hitpoints frame runs the hazard admission check"
    );
}

#[test]
fn no_selected_step_clears_the_old_hazard_before_the_next_eat_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));

    replace_steps(&mut script, Vec::new());
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_no_stale_hazard_eat(&mut script, &mut snapshot, &mut ledger, &mut output, 22);
    assert_eq!(script.selected_hazard.hazard, None);
}

#[test]
fn run_change_clears_the_old_hazard_before_the_next_eat_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));

    remove_first_step_hazard(&mut script);
    script.run.run = 2;
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_no_stale_hazard_eat(&mut script, &mut snapshot, &mut ledger, &mut output, 22);
    assert_eq!(script.selected_hazard.hazard, None);
}

#[test]
fn death_clears_the_old_hazard_before_the_next_eat_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    snapshot.seed_chat_lines(Vec::new());
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));

    remove_first_step_hazard(&mut script);
    snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: "Oh dear, you are dead!".into(),
        sequence: 1,
    }]);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.deaths, 1);
    assert_no_stale_hazard_eat(&mut script, &mut snapshot, &mut ledger, &mut output, 22);
    assert_eq!(script.selected_hazard.hazard, None);
}

#[test]
fn session_ended_clears_the_old_hazard_before_the_next_eat_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));

    remove_first_step_hazard(&mut script);
    script.interrupt(crate::native::Interrupt::SessionEnded);
    assert_no_stale_hazard_eat(&mut script, &mut snapshot, &mut ledger, &mut output, 21);
    assert_eq!(script.selected_hazard.hazard, None);
}

#[test]
fn quester_eats_inside_a_hazard_step_and_restarts_it() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "selection defers begin"
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(counts.began.load(Ordering::Relaxed), 1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.step.is_none(),
        "the active step is cancelled before the bite"
    );
    assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
    assert!(script.needs_read);
    assert!(script.survival_action.is_some());
    assert_eq!(
        eat_count(&ledger, &output),
        1,
        "the bite op begins on the cancel tick"
    );
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        2,
        "the count drop reads and restarts the same step"
    );
    assert_eq!(
        script.selected_hazard.slot,
        Some(StepSlot {
            in_prelude: false,
            seq_index: script.seq_index,
            step_index: 0
        })
    );
    assert_eq!(eat_count(&ledger, &output), 1);
}
#[test]
fn quester_closes_dialogue_page_before_bite_on_a_hazard_step() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(counts.began.load(Ordering::Relaxed), 1);
    chat_page(&mut snapshot, 2100);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.step.is_none());
    assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
    assert_eq!(close_count(&ledger, &output), 1);
    assert_eq!(
        eat_count(&ledger, &output),
        0,
        "the page closes before the held Eat request"
    );
    assert!(script.survival_action.is_some());
    chat_page(&mut snapshot, -1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(close_count(&ledger, &output), 1);
    assert_eq!(
        eat_count(&ledger, &output),
        1,
        "the held bite starts after the close settles"
    );
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 24,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival_action.is_none());
    assert_eq!(eat_count(&ledger, &output), 1);
}
#[test]
fn quester_logs_an_undeclared_damage_source_once_per_step_identity() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = survival_step_fixture("interact");
    let complete = Arc::new(AtomicBool::new(false));
    let began = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let settled = Arc::new(AtomicBool::new(false));
    let first = compiled_step(
        "unknown-source-1",
        "interact",
        Arc::new(TogglePlan {
            complete: Arc::clone(&complete),
            began: Arc::clone(&began),
            cancelled: Arc::clone(&cancelled),
        }),
        None,
        Arc::new(AtomicPredicate(Arc::clone(&settled))),
    );
    let second_begins = Arc::new(AtomicUsize::new(0));
    let second = compiled_step(
        "unknown-source-2",
        "talk",
        Arc::new(PendingPlan(second_begins)),
        None,
        Arc::new(NeverPredicate),
    );
    replace_steps(&mut script, vec![first, second]);
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        3,
        &mut output,
    );
    assert_eq!(warning_lines(&output).len(), 1);
    assert!(warning_lines(&output)[0].contains("step=unknown-source-1 hit=3"));
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        5,
        &mut output,
    );
    assert_eq!(
        warning_lines(&output).len(),
        1,
        "two hit windows produce one line for this step"
    );

    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 24,
            hp: 1,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.step.is_none(),
        "the low-HP bite cancels the first step"
    );
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 25,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        began.load(Ordering::Relaxed),
        2,
        "survival restarts the same step identity"
    );
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 26,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        7,
        &mut output,
    );
    assert_eq!(
        warning_lines(&output).len(),
        1,
        "a third hit after the same-step restart does not add a line"
    );
    complete.store(true, Ordering::Relaxed);
    settled.store(true, Ordering::Relaxed);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 27,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 28,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        script.selected_hazard.slot,
        Some(StepSlot {
            in_prelude: false,
            seq_index: script.seq_index,
            step_index: 1
        })
    );
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 29,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        9,
        &mut output,
    );
    assert_eq!(
        warning_lines(&output).len(),
        2,
        "the next selected step gets its own warning"
    );
    assert!(warning_lines(&output)[1].contains("step=unknown-source-2 hit=9"));
}

#[test]
fn quester_unattributed_hit_bites_at_half_max_on_its_window() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", None);
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        3,
        &mut output,
    );
    assert_eq!(eat_count(&ledger, &output), 1);
    assert!(script.survival_action.is_some());
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival_action.is_none());
    assert_eq!(
        eat_count(&ledger, &output),
        1,
        "the quiet next edge does not request another bite"
    );
}

#[test]
fn food_park_closes_an_open_dialogue_page_on_the_need_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    chat_page(&mut snapshot, 2100);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(script.park_page_ticks, 1);
    assert_eq!(close_count(&ledger, &output), 1);
    assert!(script.survival_action.is_some());
    chat_page(&mut snapshot, -1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(
        script.survival.report().decision,
        crate::survival::Decision::Parked
    );
    assert!(hazard_bound_lines(&output)
        .iter()
        .any(|line| line.contains("cause=close anchor_tick=22")));
}

#[test]
fn food_park_closes_the_page_on_the_next_tick_when_the_event_is_spent() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    chat_page(&mut snapshot, 2100);
    snapshot.seed_varps(vec![active_prayer_varp(&script)]);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            let spent = begin_clear_owner(script, native, owned_prayer(script));
            native.actions.cancel(spent);
            script
                .tick(native)
                .expect("Quester tick after the real prayer IfButton spent the event")
        },
    );
    assert!(script.survival.parked().is_some());
    assert!(script.survival_deferred);
    assert_eq!(script.park_page_ticks, 1);
    assert_eq!(
        interaction_count(&ledger, &output, |request| matches!(
            request,
            crate::interact::InteractReq::IfButton { .. }
        )),
        1
    );
    assert_eq!(
        live_close_count(&ledger),
        0,
        "no close can begin after the prayer IfButton spends the event"
    );
    assert!(script.survival_action.is_none());
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.park_page_ticks, 2);
    assert_eq!(
        live_close_count(&ledger),
        1,
        "the close begins on the next available event tick"
    );
}
#[test]
fn food_park_revokes_a_retained_prayer_clear_and_closes_the_page() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    let owned = owned_prayer(&script);
    snapshot.seed_varps(Vec::new());
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    chat_page(&mut snapshot, 2100);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            let clear = begin_clear_owner(script, native, owned);
            script.clear_prayers = Some(clear);
            script.prayer_cleanup_owned = owned;
            script.prayer_cleanup_pending = true;
            script
                .tick(native)
                .expect("Need tick revokes the pending ClearPrayers owner")
        },
    );
    assert!(script.clear_prayers.is_none());
    assert!(script.prayer_cleanup_pending);
    assert!(script.survival.parked().is_some());
    assert_eq!(
        live_close_count(&ledger),
        1,
        "the retained prayer owner no longer refuses the close"
    );
    chat_page(&mut snapshot, -1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_none());
    assert!(script.prayer_cleanup_pending);
    assert!(
        script.clear_prayers.is_some(),
        "owned prayer cleanup restarts before the quest step"
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "the deferred quest step does not begin before cleanup"
    );
}
#[test]
fn food_park_retries_a_refused_close() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    let owned = owned_prayer(&script);
    snapshot.seed_varps(Vec::new());
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    let owner = with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            let owner = begin_clear_owner(script, native, owned);
            script.tick(native).expect(
                "hazard-selection tick while a real ClearPrayers owner occupies the ledger",
            );
            owner
        },
    );
    chat_page(&mut snapshot, 2100);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(script.park_page_ticks, 1);
    assert_eq!(
        close_count(&ledger, &output),
        0,
        "the live native owner refuses the close begin"
    );
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            native.actions.cancel(owner);
            script
                .tick(native)
                .expect("the close retries on the next event tick")
        },
    );
    assert_eq!(script.park_page_ticks, 2);
    assert_eq!(
        close_count(&ledger, &output),
        1,
        "the refused close retries after the owner is cancelled"
    );
    assert_eq!(live_close_count(&ledger), 1);
}

#[test]
fn food_park_retries_an_unapplied_close_on_the_same_tick_and_stops_on_tick_24() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 0,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    chat_page(&mut snapshot, 2100);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 1,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.park_page_ticks, 1);
    assert_eq!(close_count(&ledger, &output), 1);
    assert_eq!(live_close_count(&ledger), 1);
    for tick in 2..=8 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert_eq!(close_count(&ledger, &output), 1);
    }
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 9,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.park_page_ticks, 9);
    assert_eq!(
        close_count(&ledger, &output),
        2,
        "the second close starts at t+8"
    );
    for tick in 10..=16 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert_eq!(close_count(&ledger, &output), 2);
    }
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 17,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.park_page_ticks, 17);
    assert_eq!(
        close_count(&ledger, &output),
        3,
        "the third close starts at t+16"
    );
    for tick in 18..=24 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert_eq!(close_count(&ledger, &output), 3);
    }
    let flow = drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 25,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        matches!(flow, ScriptFlow::Blocked(failure) if failure.code.as_ref() == "parked" && failure.message.as_ref() == "survival park: the page did not close within 24 parked ticks")
    );
    assert!(script.parked);
    assert_eq!(script.park_page_ticks, 25);
    assert!(script.survival.parked().is_some());
    assert_eq!(
        close_count(&ledger, &output),
        3,
        "no fourth close request follows the tick-24 stop"
    );
}

#[test]
fn food_park_stops_on_tick_24_under_permanent_refusal() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    let owned = owned_prayer(&script);
    snapshot.seed_varps(Vec::new());
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    let owner = with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            let owner = begin_clear_owner(script, native, owned);
            script
                .tick(native)
                .expect("hazard-selection tick under a real ClearPrayers owner");
            owner
        },
    );
    chat_page(&mut snapshot, 2100);
    for tick in 21..=45 {
        let flow = drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        if tick == 45 {
            assert!(
                matches!(flow, ScriptFlow::Blocked(failure) if failure.code.as_ref() == "parked" && failure.message.as_ref() == "survival park: the page did not close within 24 parked ticks")
            );
        }
    }
    assert!(script.parked);
    assert_eq!(script.park_page_ticks, 25);
    assert!(script.survival.parked().is_some());
    assert_eq!(
        close_count(&ledger, &output),
        0,
        "the real active owner makes every close begin Busy"
    );
    assert_eq!(live_close_count(&ledger), 0);
    drop(owner);
}

#[test]
fn food_park_closes_a_page_that_opens_after_the_park_began() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(script.park_page_ticks, 0);
    for tick in 22..=26 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert_eq!(script.park_page_ticks, 0);
        assert_eq!(close_count(&ledger, &output), 0);
    }
    chat_page(&mut snapshot, 2100);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 27,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(script.park_page_ticks, 1);
    assert_eq!(close_count(&ledger, &output), 1);
    chat_page(&mut snapshot, -1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 28,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(hazard_bound_lines(&output)
        .iter()
        .any(|line| line.contains("cause=close anchor_tick=28")));
}

#[test]
fn food_park_closes_an_open_interface() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp/274bot-upkeep-modal-fixture".into(),
        members: true,
        lowmem: false,
    });
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    main_page(&mut snapshot, &mut client, 600);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(script.park_page_ticks, 1);
    assert_eq!(close_count(&ledger, &output), 1);
    main_page(&mut snapshot, &mut client, -1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(hazard_bound_lines(&output)
        .iter()
        .any(|line| line.contains("cause=close anchor_tick=22")));
    assert!(script.survival.parked().is_some());
}
fn hit_bound_run(unpark_tick: u64) -> (bool, Vec<String>, Quester) {
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 24,
            hp: 4,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        3,
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(
        script.survival.report().need,
        Some(crate::survival::math::SurvivalNeed::Food {
            count: 2,
            ref_heal: 12,
        })
    );
    assert_eq!(
        script.survival.parked(),
        Some(crate::survival::math::SurvivalNeed::Food {
            count: 1,
            ref_heal: 12
        }),
        "the unattributed hit raises the report, not the retained park",
    );
    for (tick, hp) in [(25, 5), (26, 6), (27, 7)] {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert!(script.survival.parked().is_some());
        assert_eq!(
            script.survival.report().need,
            Some(crate::survival::math::SurvivalNeed::Food {
                count: 1,
                ref_heal: 12
            }),
            "the next quiet frame uses the known hazard line again",
        );
    }
    let lines = hazard_bound_lines(&output)
        .iter()
        .map(|line| (*line).to_owned())
        .collect::<Vec<_>>();
    let is_unparked = {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick: unpark_tick,
                hp: 8,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        script.survival.parked().is_none()
    };
    (is_unparked, lines, script)
}

#[test]
fn food_park_recomputes_the_bound_after_the_committed_hit() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (released_at_bound, lines, script) = hit_bound_run(425);
    assert!(
        released_at_bound,
        "Hitpoints 8 releases at the hit-anchored 401-tick upper bound"
    );
    assert_eq!(
        lines.len(),
        2,
        "the edge and committed hit each publish one bound"
    );
    assert!(lines[0].contains("cause=edge anchor_tick=21 hp=7 release_hp=8 bound_ticks=101"));
    assert!(lines[1].contains("cause=hit anchor_tick=24 hp=4 release_hp=8 bound_ticks=401"));
    assert!(script.survival.parked().is_none());
    let (released_before_bound, _, early_script) = hit_bound_run(324);
    assert!(
        released_before_bound,
        "Hitpoints 8 releases before the upper bound too"
    );
    assert!(early_script.survival.parked().is_none());
}

#[test]
fn food_park_before_any_attempt_ends_only_by_regeneration() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, counts) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        0,
        "the deferred begin has not spent an attempt"
    );
    assert_eq!(counts.cancelled.load(Ordering::Relaxed), 0);
    for tick in 22..=120 {
        drive(
            &mut script,
            &mut snapshot,
            &mut ledger,
            TestFrame {
                tick,
                hp: 7,
                hp_max: 40,
                poison: PoisonState::Clear,
                cure: CurePoison::Never,
            },
            &mut output,
        );
        assert!(
            script.survival.parked().is_some(),
            "the step stays parked at tick {tick}"
        );
        assert_eq!(
            counts.began.load(Ordering::Relaxed),
            0,
            "the deferred step begins while parked at tick {tick}"
        );
    }
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 121,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_none());
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 122,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(
        counts.began.load(Ordering::Relaxed),
        1,
        "the deferred step begins after regeneration releases the park"
    );
}

#[test]
fn food_park_release_uses_the_line_of_the_current_hitpoints() {
    if !api::test_support::game_data_available() {
        return;
    }
    let hazard = StepHazard {
        of: HazardStat::Current,
        mul: 25,
        div: 100,
        plus: 4,
        cap: None,
    };
    let (mut script, mut snapshot, _) = fixture("interact", Some(hazard));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 10,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert!(hazard_bound_lines(&output)
        .iter()
        .any(|line| line.contains("release_hp=18 bound_ticks=801")));
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 14,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.survival.parked().is_some(),
        "Hitpoints 14 still needs one point above its current line"
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 17,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.survival.parked().is_some(),
        "Hitpoints 17 remains on the current eat line"
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 24,
            hp: 18,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_none());
}

#[test]
fn food_park_bound_does_not_apply_with_a_live_row() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let world = crate::combat_session::tests::World::new("scorpion");
    let template = world.snapshot.npcs()[0].clone();
    let at = WorldTile {
        x: 3201,
        z: 3200,
        level: 0,
    };
    snapshot.seed_npcs(vec![api::snapshot::NpcView {
        index: 1,
        tile: at,
        network: at,
        target: Some(api::snapshot::ActorTargetView {
            kind: api::snapshot::ActorKind::Player,
            index: 0,
        }),
        in_combat: true,
        ..template
    }]);
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival.parked().is_some());
    assert!(
        hazard_bound_lines(&output).is_empty(),
        "a live combat row prevents a quiet recovery bound"
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.survival.parked().is_some(),
        "the active scorpion line is 11 at Hitpoints 8"
    );
    assert!(hazard_bound_lines(&output).is_empty());
}

#[test]
fn pause_and_resume_keep_a_pending_bite_and_a_hazard_park_safe() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut script, mut snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival_action.is_some());
    assert_eq!(eat_count(&ledger, &output), 1);
    script.interrupt(Interrupt::Pause);
    script.interrupt(Interrupt::Resume);
    assert!(
        script.survival_action.is_none(),
        "Resume drops the pending click handle"
    );
    assert_eq!(script.selected_hazard.hazard, Some(RAILING_HAZARD));
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.survival_action.is_some());
    assert_eq!(
        eat_count(&ledger, &output),
        2,
        "the next poll retries the bite under the retained hazard"
    );

    let (mut parked, mut parked_snapshot, _) = fixture("interact", Some(RAILING_HAZARD));
    let mut parked_ledger = None;
    let mut parked_output = CapturedOutput::default();
    set_inventory(&parked, &mut parked_snapshot, 0, 0, 0);
    drive(
        &mut parked,
        &mut parked_snapshot,
        &mut parked_ledger,
        TestFrame {
            tick: 30,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut parked_output,
    );
    drive(
        &mut parked,
        &mut parked_snapshot,
        &mut parked_ledger,
        TestFrame {
            tick: 31,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut parked_output,
    );
    chat_page(&mut parked_snapshot, 2100);
    drive(
        &mut parked,
        &mut parked_snapshot,
        &mut parked_ledger,
        TestFrame {
            tick: 32,
            hp: 7,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut parked_output,
    );
    let page_ticks = parked.park_page_ticks;
    assert_eq!(page_ticks, 1);
    parked.interrupt(Interrupt::Pause);
    parked.interrupt(Interrupt::Resume);
    assert!(parked.survival.parked().is_some());
    assert_eq!(parked.park_page_ticks, page_ticks);
    assert_eq!(parked.selected_hazard.hazard, Some(RAILING_HAZARD));
    chat_page(&mut parked_snapshot, -1);
    drive(
        &mut parked,
        &mut parked_snapshot,
        &mut parked_ledger,
        TestFrame {
            tick: 33,
            hp: 8,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut parked_output,
    );
    assert!(parked.survival.parked().is_none());
    assert_eq!(parked.selected_hazard.hazard, Some(RAILING_HAZARD));
}

#[test]
fn make_step_restarts_with_the_remaining_quantity_after_a_survival_cancel() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut document = super::super::compile::decode_cook().expect("Cook path document");
    install_make_document(&mut document);
    let (mut script, mut snapshot) = status_fixture(document);
    let product_id = bar_id(&script);
    let mut client = make_client(product_id, "Bronze bar");
    let loc = make_loc(&script);
    snapshot.seed_locs(vec![loc]);
    let mut ledger = None;
    let mut output = CapturedOutput::default();
    set_inventory(&script, &mut snapshot, 0, 1, 0);
    chat_page(&mut snapshot, -1);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 20,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        matches!(ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect), Some(HostEffect::Interaction(crate::interact::InteractReq::Loc { action, .. })) if action == "Smelt")
    );

    client.chat_modal_id = 2100;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
    snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 21,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            accept_latest_interaction(native);
            script.tick(native).expect("accepted Smelt trigger");
        },
    );
    assert!(matches!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .map(|action| &action.effect),
        Some(HostEffect::Interaction(
            crate::interact::InteractReq::IfButton { component_id: 2120 }
        ))
    ));

    snapshot.seed_count_dialog(true);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 22,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            accept_latest_interaction(native);
            script.tick(native).expect("open first count dialog");
        },
    );
    assert_eq!(
        count_answers(&ledger, &output),
        vec![5],
        "the first Make-X request targets five held bars"
    );
    snapshot.seed_count_dialog(false);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 23,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            accept_latest_interaction(native);
            script.tick(native).expect("accept first Make-X count");
        },
    );
    client.chat_modal_id = -1;
    client.bump_gens(ServerProt::IF_CLOSE);
    snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
    snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 24,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.step.is_some(),
        "the real MakeMachine waits for its held target"
    );

    set_inventory(&script, &mut snapshot, 2, 1, 0);
    drive_with_hit(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 25,
            hp: 19,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        3,
        &mut output,
    );
    assert_eq!(
        count_item(&snapshot, product_id),
        2,
        "two produced bars remain held at cancellation"
    );
    assert!(
        script.step.is_none(),
        "the survival bite cancels the active production run"
    );
    assert_eq!(eat_count(&ledger, &output), 1);

    set_inventory(&script, &mut snapshot, 2, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 26,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        matches!(ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect), Some(HostEffect::Interaction(crate::interact::InteractReq::Loc { action, .. })) if action == "Smelt")
    );

    client.chat_modal_id = 2100;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
    snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 27,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            accept_latest_interaction(native);
            script
                .tick(native)
                .expect("accepted restarted Smelt trigger");
        },
    );
    assert!(matches!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .map(|action| &action.effect),
        Some(HostEffect::Interaction(
            crate::interact::InteractReq::IfButton { component_id: 2120 }
        ))
    ));
    snapshot.seed_count_dialog(true);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 28,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            accept_latest_interaction(native);
            script.tick(native).expect("open restarted count dialog");
        },
    );
    assert_eq!(
        count_answers(&ledger, &output),
        vec![5, 3],
        "the restarted Make-X request asks only for the three bars still needed"
    );
    snapshot.seed_count_dialog(false);
    with_runner_tick(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 29,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
        |script, native| {
            accept_latest_interaction(native);
            script.tick(native).expect("accept restarted Make-X count");
        },
    );
    client.chat_modal_id = -1;
    client.bump_gens(ServerProt::IF_CLOSE);
    snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
    snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 30,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(
        script.step.is_some(),
        "the restarted MakeMachine waits for the authored total"
    );
    set_inventory(&script, &mut snapshot, 4, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 31,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert!(script.settling, "the real MakeMachine completed a partial production receipt while the authored total remains unmet");
    set_inventory(&script, &mut snapshot, 5, 0, 0);
    drive(
        &mut script,
        &mut snapshot,
        &mut ledger,
        TestFrame {
            tick: 32,
            hp: 40,
            hp_max: 40,
            poison: PoisonState::Clear,
            cure: CurePoison::Never,
        },
        &mut output,
    );
    assert_eq!(count_item(&snapshot, product_id), 5);
    assert_eq!(
        script.cursor, 1,
        "the Make step settles only when the full target of five is held"
    );
    assert!(!script.settling);
}
