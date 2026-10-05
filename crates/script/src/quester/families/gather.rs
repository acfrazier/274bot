//! A bounded quest inventory goal on the Gatherer's one native resource action.
//!
//! Authoring example: `{"skill":"mining","resource":"copper",
//! "until":{"obj":"copper_ore","qty":1}}`. Set exactly one of `resource`
//! (selected Gatherer resource key) or `method` (selected method id).
//! `until.obj` also accepts `{"id":436}` for exact identity. `anchor` locates
//! the initial search area; its radius is fixed for the whole step (default 12,
//! maximum 32). `settle_ms` bounds eligible execution time (default 120000).
//! There is no bank, area loop, map survey, or fallback beyond observed resources.

use super::super::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use super::item_arg::ItemArg;
use crate::gatherer::gather::{GatherEnd, GatherRun, GatherRunArgs, DEFAULT_STALL_TICKS};
use crate::gatherer::select::{
    select_for_quest, FishingSurvey, PlacementClass, SelectionObservation,
};
use crate::gatherer::settings::{admit_quest_method, GathererSettings};
use crate::gatherer::supply::method_ready;
use crate::gatherer::{AreaMode, WorkArea};
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions};
use api::gather_methods::{GatherCatalog, GatherSkill};
use api::snapshot::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub(super) enum Skill {
    Mining,
    Fishing,
}

impl Skill {
    fn catalog(self) -> GatherSkill {
        match self {
            Self::Mining => GatherSkill::Mining,
            Self::Fishing => GatherSkill::Fishing,
        }
    }
    fn stat_name(self) -> &'static str {
        match self {
            Self::Mining => "mining",
            Self::Fishing => "fishing",
        }
    }
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Until {
    pub obj: ItemArg,
    pub qty: super::s2::QuantityDocument,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    pub skill: Skill,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    pub until: Until,
    #[serde(default)]
    pub anchor: Option<super::AnchorArg>,
    #[serde(default = "default_radius")]
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1, max = 32)))]
    pub radius: u16,
    #[serde(default = "default_settle_ms")]
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    pub settle_ms: u64,
}

fn default_radius() -> u16 {
    12
}
fn default_settle_ms() -> u64 {
    120_000
}

pub(super) fn compile(
    args: Args,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.resource.is_some() == args.method.is_some()
        || args.radius == 0
        || args.radius > 32
        || args.settle_ms == 0
        || matches!(&args.until.qty, super::s2::QuantityDocument::Fixed(qty) if *qty < 1)
    {
        return Err(CompileError::code("invalid-args"));
    }
    let catalog = Arc::clone(
        cx.gathering
            .ok_or_else(|| CompileError::code("gathering-unavailable"))?,
    );
    let item_id = args.until.obj.id(cx.selected)?;
    let methods: Vec<_> = catalog
        .methods()
        .iter()
        .enumerate()
        .filter(|(_, method)| {
            method.skill == args.skill.catalog()
                && match (&args.resource, &args.method) {
                    (Some(resource), None) => method
                        .resources
                        .iter()
                        .any(|key| key.0.eq_ignore_ascii_case(resource.trim())),
                    (None, Some(id)) => method.id.0.as_ref() == id,
                    _ => false,
                }
        })
        .filter(|(_, method)| {
            api::gather_methods::known_rows(&method.products)
                .iter()
                .any(|product| product.item == item_id)
        })
        .map(|(index, method)| {
            admit_quest_method("gather", method)
                .map_err(|_| CompileError::code("gather-method-incomplete"))?;
            Ok(index)
        })
        .collect::<Result<_, CompileError>>()?;
    if methods.is_empty() {
        return Err(CompileError::code("unresolved-gather-method"));
    }
    let anchor = super::anchor_tile(args.anchor.as_ref())?;
    Ok(Arc::new(Plan {
        catalog,
        methods: methods.into(),
        settings: Arc::new(GathererSettings {
            target_preference: "Nearest".into(),
            ..GathererSettings::default()
        }),
        skill: args.skill,
        item_id,
        qty: super::s2::compile_quantity(args.until.qty, cx)?,
        anchor,
        radius: args.radius,
        duration: Duration::from_millis(args.settle_ms),
    }))
}

struct Plan {
    catalog: Arc<GatherCatalog>,
    methods: Arc<[usize]>,
    settings: Arc<GathererSettings>,
    skill: Skill,
    item_id: i32,
    qty: super::s2::QuantityPlan,
    anchor: Option<WorldTile>,
    radius: u16,
    duration: Duration,
}

impl StepPlan for Plan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let qty = self
            .qty
            .evaluate_step(cx)
            .ok_or_else(|| ActionError::Unavailable("gather quantity unavailable".into()))?;
        Ok(Box::new(Run {
            catalog: Arc::clone(&self.catalog),
            methods: Arc::clone(&self.methods),
            settings: Arc::clone(&self.settings),
            skill: self.skill,
            item_id: self.item_id,
            qty,
            area: self.anchor.map(|anchor| WorkArea {
                mode: AreaMode::Custom,
                anchor,
                radius: self.radius,
            }),
            radius: self.radius,
            approach: self.anchor,
            deadline: cx.tick.cx.active_now() + self.duration,
            fishing: FishingSurvey::default(),
            gather: None,
            walk: None,
            last_attempt: None,
        }))
    }
    fn settle_timeout(&self) -> Duration {
        self.duration
    }
}

struct Run {
    catalog: Arc<GatherCatalog>,
    methods: Arc<[usize]>,
    settings: Arc<GathererSettings>,
    skill: Skill,
    item_id: i32,
    qty: i32,
    area: Option<WorkArea>,
    radius: u16,
    approach: Option<WorldTile>,
    deadline: Duration,
    fishing: FishingSurvey,
    gather: Option<ActionHandle<GatherRun>>,
    walk: Option<ActionHandle<Walk>>,
    last_attempt: Option<u64>,
}

impl StepRun for Run {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let walk_pending = if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => true,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    super::walk_step_evidence(receipt)?;
                    self.walk = None;
                    false
                }
            }
        } else {
            false
        };
        if cx.tick.cx.active_now() >= self.deadline {
            return Poll::Ready(Err(ActionError::Failed("gather deadline exhausted".into())));
        }
        if let Some(inventory) = cx.tick.cx.snapshot().inventory() {
            let count = inventory
                .value
                .iter()
                .filter(|item| item.def.id == self.item_id)
                .fold(0i32, |count, item| count.saturating_add(item.count.max(0)));
            if count >= self.qty {
                self.gather = None;
                self.walk = None;
                return Poll::Ready(Ok(StepOutcome {
                    progress: None,
                    evidence: cx.tick.cx.evidence(),
                    receipt: None,
                }));
            }
        }
        if walk_pending {
            return Poll::Pending;
        }
        if let Some(handle) = &self.gather {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(result)) => {
                    self.gather = None;
                    match result.end {
                        GatherEnd::Full => {
                            return Poll::Ready(Err(ActionError::Failed(
                                "gather inventory full before goal".into(),
                            )))
                        }
                        // The shared run tends stray modals. Both callers then
                        // revalidate and select again after a server refusal.
                        GatherEnd::Refused => {}
                        GatherEnd::Hazard => {
                            return Poll::Ready(Err(ActionError::Failed(
                                "gather resource became hazardous".into(),
                            )))
                        }
                        GatherEnd::Depleted | GatherEnd::TargetGone | GatherEnd::Idle => {}
                    }
                }
            }
        }
        let snapshot = cx.tick.cx.snapshot();
        let (Some(here), Some(world), Some(stats), Some(locs), Some(npcs)) = (
            snapshot.here(),
            snapshot.world(),
            snapshot.stats(),
            snapshot.locs(),
            snapshot.npcs(),
        ) else {
            return Poll::Pending;
        };
        let Some(stat) = stats
            .value
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(self.skill.stat_name()))
        else {
            return Poll::Pending;
        };
        if let Some(anchor) = self.approach.take() {
            if !super::reach::within(here.value, anchor, i32::from(self.radius)) {
                self.walk = Some(cx.tick.actions.begin::<Walk>(
                    super::reach::walk_request(anchor, self.radius, None, cx.required_after),
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
        }
        let area = *self.area.get_or_insert(WorkArea {
            mode: AreaMode::Start,
            anchor: here.value,
            radius: self.radius,
        });
        let mut ready = false;
        let mut unobserved = false;
        let mut refusal = None;
        for &index in self.methods.iter() {
            match method_ready(snapshot, &self.catalog.methods()[index], true) {
                Ok(true) => {
                    ready = true;
                    break;
                }
                Ok(false) => unobserved = true,
                Err(error) => {
                    refusal.get_or_insert(error);
                }
            }
        }
        if !ready {
            return if unobserved {
                Poll::Pending
            } else {
                Poll::Ready(Err(ActionError::Unavailable(
                    refusal
                        .expect("compiled gather methods are nonempty")
                        .into(),
                )))
            };
        }
        let selected = select_for_quest(
            &self.catalog,
            &self.methods,
            &self.settings,
            area,
            SelectionObservation {
                world: world.value,
                locs: locs.value,
                npcs: npcs.value,
                here: here.value,
                now: cx.tick.cx.evidence().tick,
                skill_stat: stat.index,
                fishing: &mut self.fishing,
            },
            snapshot,
        );
        // This is an authored local resource step, not Gatherer's site/Auto
        // survey. Never widen the area or circle unloaded placements.
        let Some(target) = selected
            .target
            .filter(|target| target.class == PlacementClass::Live)
        else {
            return Poll::Pending;
        };
        if let Some(request) = target.approach(snapshot, cx.required_after) {
            self.walk = Some(cx.tick.actions.begin::<Walk>(request, &mut cx.tick.cx)?);
            return Poll::Pending;
        }
        if self.last_attempt == Some(cx.tick.cx.evidence().tick) {
            return Poll::Pending;
        }
        self.last_attempt = Some(cx.tick.cx.evidence().tick);
        self.gather = Some(cx.tick.actions.begin::<GatherRun>(
            GatherRunArgs {
                target: target.plan,
                catalog: Arc::clone(&self.catalog),
                stall_ticks: DEFAULT_STALL_TICKS,
                quest_owned: true,
            },
            &mut cx.tick.cx,
        )?);
        Poll::Pending
    }

    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.gather = None;
        self.walk = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::families::tests::{def, local_player, with_tick};
    use api::snapshot::{
        GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, StatView,
        WorldStateView,
    };

    fn plan(method: &str, qty: i32) -> Plan {
        let catalog = api::selected::FamilyPreparation::run(|worker| {
            let selected =
                api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            api::gather_methods::prepare(&selected, worker)
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let index = catalog
            .methods()
            .iter()
            .position(|row| row.id.0.as_ref() == method)
            .unwrap();
        Plan {
            catalog,
            methods: Arc::from([index]),
            settings: Arc::new(GathererSettings {
                target_preference: "Nearest".into(),
                ..Default::default()
            }),
            skill: Skill::Mining,
            item_id: 436,
            qty: super::super::s2::QuantityPlan::Fixed(qty),
            anchor: None,
            radius: 12,
            duration: Duration::from_secs(30),
        }
    }

    fn held(id: i32, count: i32, slot: i32) -> ItemView {
        ItemView {
            def: def(id, "fixture"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count,
            actions: vec![],
            component_id: 3214,
        }
    }

    fn snapshot(plan: &Plan, level: i32) -> GameSnapshot {
        let method = &plan.catalog.methods()[plan.methods[0]];
        let spot = api::gather_methods::known_rows(&method.spots)
            .iter()
            .find(|spot| {
                api::gather_methods::known_rows(&method.targets)
                    .iter()
                    .any(|target| {
                        target.entity == spot.entity
                            && matches!(target.respawn, api::selected::Knowledge::Known(_))
                            && target.class == api::gather_methods::TargetClass::Resource
                    })
            })
            .unwrap();
        let api::selected::EntityId::Loc(id) = spot.entity else {
            panic!("mining loc");
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![held(1265, 1, 0)], 28);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_stats(vec![StatView {
            index: 14,
            name: "mining".into(),
            effective: level,
            base: level,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_local_player(local_player(WorldTile {
            x: spot.origin.x + 1,
            ..spot.origin
        }));
        snapshot.seed_world(WorldStateView {
            map_base_x: spot.origin.x - 50,
            map_base_z: spot.origin.z - 50,
            level: spot.origin.level,
            members: true,
            ..Default::default()
        });
        snapshot.seed_scene(api::snapshot::SceneView {
            available: true,
            base_x: spot.origin.x - 50,
            base_z: spot.origin.z - 50,
            level: spot.origin.level,
            width: 104,
            height: 104,
            collision_flags: vec![0; 104 * 104],
        });
        snapshot.seed_npcs(vec![]);
        snapshot.seed_locs(vec![LocView {
            id,
            name: Some("Rocks".into()),
            actions: vec![Some("Mine".into())],
            tile: spot.origin,
            distance: 1,
            typecode: 0,
            info: 0,
            description: None,
            layer: LocLayer::Ground,
            shape: 10,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: true,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
        snapshot
    }

    fn with_step<R>(
        tick: &mut crate::native::NativeTick<'_>,
        f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
    ) -> R {
        let quests = api::quest_facts::QuestCatalog::empty();
        let bank = crate::quester::bank_memo::BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        let required_after = tick.cx.evidence();
        f(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            bank: &bank,
            banks: &banks,
            choices: &choices,
        })
    }

    #[test]
    fn gather_uses_native_resource_action_and_finishes_only_at_exact_goal() {
        let plan = plan("mining.copper", 2);
        let mut snapshot = snapshot(&plan, 1);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick(&snapshot, &mut ledger, 2, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(ledger.as_ref().is_some_and(|ledger| ledger.outbox.iter().any(|interaction| {
            matches!(&interaction.effect, crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { action, .. }) if action == "Mine")
        })));
        snapshot.seed_inventory(
            vec![held(1265, 1, 0), held(438, 99, 1), held(436, 1, 2)],
            28,
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 3, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        snapshot.seed_inventory(vec![held(1265, 1, 0), held(436, 2, 1)], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| with_step(tick, |cx| run
                .poll(cx))),
            Poll::Ready(Ok(_))
        ));
    }

    #[test]
    fn ordinary_gatherer_defers_supply_refusal_to_server_for_revalidation() {
        let plan = plan("mining.copper", 1);
        let mut snapshot = snapshot(&plan, 1);
        let loc = snapshot.locs()[0].clone();
        snapshot.seed_inventory(vec![], 28);
        let target = crate::gatherer::select::TargetPlan {
            entity: api::selected::EntityId::Loc(loc.id),
            tile: loc.tile,
            op: "Mine".into(),
            alias: "mining.copper".into(),
            products: [436, 0, 0, 0, 0, 0, 0, 0],
            products_len: 1,
            skill_stat: 14,
            method_index: plan.methods[0] as u16,
            npc_index: -1,
        };
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<GatherRun>(
                    GatherRunArgs {
                        target,
                        catalog: Arc::clone(&plan.catalog),
                        stall_ticks: DEFAULT_STALL_TICKS,
                        quest_owned: false,
                    },
                    &mut tick.cx,
                )
                .expect("ordinary Gatherer waits for server refusal")
        });
        assert!(ledger.as_ref().unwrap().outbox.iter().any(|op| matches!(
            &op.effect, crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { action, .. })
                if action == "Mine"
        )));
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            sequence: 1,
            type_: 0,
            text: "You can't use this tool.".into(),
            username: None,
        }]);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(crate::gatherer::gather::GatherResult {
                end: GatherEnd::Refused,
                ..
            }))
        ));
    }

    #[test]
    fn gather_observed_fishing_npc_owns_approach_without_a_walk() {
        let mut plan = plan("fishing.saltfish.op1", 1);
        plan.skill = Skill::Fishing;
        plan.item_id = 317;
        let method = &plan.catalog.methods()[plan.methods[0]];
        let spot = api::gather_methods::known_rows(&method.spots)
            .iter()
            .find(|spot| matches!(spot.entity, api::selected::EntityId::Npc(_)))
            .unwrap();
        let api::selected::EntityId::Npc(id) = spot.entity else {
            unreachable!()
        };
        let tile = spot.origin;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![held(303, 1, 0)], 28);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_stats(vec![StatView {
            index: 10,
            name: "fishing".into(),
            effective: 1,
            base: 1,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_local_player(local_player(WorldTile {
            x: tile.x + 5,
            ..tile
        }));
        snapshot.seed_world(WorldStateView {
            map_base_x: tile.x - 50,
            map_base_z: tile.z - 50,
            level: tile.level,
            members: true,
            ..Default::default()
        });
        snapshot.seed_locs(vec![]);
        snapshot.seed_npcs(vec![api::snapshot::NpcView {
            index: 7,
            r#type: Some(id as usize),
            name: Some("Fishing spot".into()),
            actions: vec![Some("Net".into())],
            tile,
            distance: 5,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: 0,
            z: 0,
            yaw: 0,
        }]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let outbox = &ledger.as_ref().unwrap().outbox;
        assert!(!outbox
            .iter()
            .any(|op| matches!(op.effect, crate::native::HostEffect::Walk(_))));
        assert!(outbox.iter().any(|op| matches!(
            &op.effect, crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc { action, index, .. })
                if action == "Net" && *index == Some(7)
        )));
    }

    // content/scripts/levelup/scripts/levelup.rs2 opens the skill chat interface.
    // Retain the reviewer's copper qty=2 probe through continue and resumed yield.
    #[test]
    fn review_probe_level_up_modal_mid_goal() {
        let plan = plan("mining.copper", 2);
        let mut snapshot = snapshot(&plan, 1);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_inventory(vec![held(1265, 1, 0), held(436, 1, 1)], 28);
        snapshot.seed_chat_modal(
            233,
            vec!["Congratulations, you just advanced a Mining level.".into()],
        );
        snapshot.seed_chat_options(vec![], 233);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(ledger.as_ref().unwrap().outbox.iter().any(|op| matches!(
            op.effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog { .. })
        )));
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(ledger.as_ref().unwrap().outbox.iter().any(|op| matches!(
            &op.effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { action, .. })
                if action == "Mine"
        )));
        snapshot.seed_inventory(vec![held(1265, 1, 0), held(436, 2, 1)], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
    }

    #[test]
    fn gather_takes_a_usable_method_even_when_another_method_is_level_gated() {
        let mut plan = plan("mining.copper", 1);
        let snapshot = snapshot(&plan, 1);
        let iron = plan
            .catalog
            .methods()
            .iter()
            .position(|method| method.id.0.as_ref() == "mining.iron")
            .unwrap();
        plan.methods = Arc::from([iron, plan.methods[0]]);
        // The fixture observes copper only, not the first (unusable) method.
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(ledger.as_ref().unwrap().outbox.iter().any(|op| matches!(
            &op.effect,
            crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { action, .. })
                if action == "Mine"
        )));
    }

    #[test]
    fn gather_arrives_at_a_loc_footprint_not_its_origin() {
        let plan = plan("mining.copper", 1);
        let mut snapshot = snapshot(&plan, 1);
        let mut loc = snapshot.locs()[0].clone();
        loc.width = 3;
        loc.footprint_width = 3;
        snapshot.seed_local_player(local_player(WorldTile {
            x: loc.tile.x + 3,
            ..loc.tile
        }));
        snapshot.seed_locs(vec![loc]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let outbox = &ledger.as_ref().unwrap().outbox;
        assert!(!outbox
            .iter()
            .any(|op| matches!(op.effect, crate::native::HostEffect::Walk(_))));
        assert!(outbox.iter().any(|op| matches!(
            &op.effect, crate::native::HostEffect::Interaction(crate::shim::InteractReq::Loc { action, .. })
                if action == "Mine"
        )));
    }

    #[test]
    fn gather_level_gate_emits_nothing_and_missing_evidence_has_a_fixed_deadline() {
        let iron = plan("mining.iron", 1);
        let snapshot = snapshot(&iron, 1);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| iron.begin(cx).unwrap())
        });
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 2, |tick| with_step(tick, |cx| run
                .poll(cx))),
            Poll::Ready(Err(ActionError::Unavailable(_)))
        ));
        assert!(ledger.is_none());
        let copper = plan("mining.copper", 1);
        let missing = GameSnapshot::new();
        let mut run = with_tick(&missing, &mut ledger, 1, |tick| {
            with_step(tick, |cx| copper.begin(cx).unwrap())
        });
        assert!(
            with_tick(&missing, &mut ledger, 10, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(matches!(
            with_tick(&missing, &mut ledger, 52, |tick| with_step(tick, |cx| run
                .poll(cx))),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    }

    #[test]
    fn late_inventory_growth_cannot_extend_the_gather_deadline() {
        let plan = plan("mining.copper", 1);
        let mut snapshot = snapshot(&plan, 1);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_inventory(vec![held(1265, 1, 0), held(436, 1, 1)], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.cx.active_now = Duration::from_secs(40);
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
    }

    #[test]
    fn manual_approach_cancellation_wins_over_an_inventory_goal() {
        let mut plan = plan("mining.copper", 1);
        let mut snapshot = snapshot(&plan, 1);
        let tile = snapshot.local_player().unwrap().player.actor.tile;
        plan.anchor = Some(WorldTile {
            x: tile.x + 40,
            ..tile
        });
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        super::super::tests::post_user_input_walk_receipt(&mut ledger, 3);
        snapshot.seed_inventory(vec![held(1265, 1, 0), held(436, 1, 1)], 28);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::UserInput))
        ));
    }
}
