//! Shared Path-stage obligations and risk-selected bank targets.
use super::compile::{CompileContext, CompileError, CompiledProvisioning, CompiledSequence};
use crate::combat::risk::{self, Food, RiskInput, RiskTables, SupplyExposure, UnknownWhy};
use crate::combat::tables::CombatTables;
use crate::combat::{CompiledKit, FoodReserve, Target};
use crate::native_bank::BankItem;
use api::stock::Stock;
use nav_planner::contract::{BoundZoneKey, LegView, PackId, RouteView, ZoneTableView};
use std::sync::Arc;

/// Food-only geometry. Admission keeps its own conservative exposures.
#[derive(Debug)]
pub struct FoodRoute {
    pack: PackId,
    zones: Box<[FoodZoneTime]>,
}

#[derive(Debug)]
struct FoodZoneTime {
    zone: BoundZoneKey,
    walk: FoodTime,
    run: FoodTime,
}

#[derive(Debug, Default, Clone, Copy)]
struct FoodTime {
    ticks: u32,
    first: u32,
    last: u32,
    at_anchor: bool,
}

#[derive(Default)]
struct FoodTrack {
    time: FoodTime,
    clock: u32,
    engaged: Option<u32>,
}

impl FoodTrack {
    fn visit(
        &mut self,
        tile: api::WorldTile,
        delta: u32,
        zone: nav_planner::contract::ZoneView<'_>,
        kind: risk::KindRisk,
    ) -> Option<()> {
        self.clock = self.clock.checked_add(delta)?;
        let (x, z) = zone.spawn();
        let distance = tile.x.abs_diff(x).max(tile.z.abs_diff(z));
        let inside = zone.contains(tile);
        // Npc.ts:670-674 retains a target within maxrange + attackrange.
        // Npc.ts:420-422,904-948 follows at WALK, at most one tile/tick.
        // The expanding reach ball permits every such NPC path. It ignores
        // obstacles and attack pauses, so it cannot remove a possible throw.
        // quest_death.npc:1498,1505,1507 gives 20 + 8, hunt 8. These
        // values come from selected facts, not the zone or fixture name.
        if tile.level != i32::from(zone.plane()) || (!inside && distance > u32::from(kind.r)) {
            self.engaged = None;
        } else if inside && self.engaged.is_none() {
            self.engaged = Some(if delta == 0 {
                self.clock
            } else {
                self.clock.saturating_sub(delta).saturating_add(1)
            });
        }
        self.time.at_anchor = self.engaged.is_some();
        let Some(engaged) = self.engaged else {
            return Some(());
        };
        let reachable = engaged.checked_add(distance.saturating_sub(u32::from(kind.reach)))?;
        let first = if delta == 0 {
            self.clock
        } else {
            self.clock.saturating_sub(delta).saturating_add(1)
        }
        .max(reachable)
        .max(self.time.last.saturating_add(1));
        if first <= self.clock {
            self.time.ticks = self.time.ticks.checked_add(self.clock - first + 1)?;
            if self.time.first == 0 {
                self.time.first = first;
            }
            self.time.last = self.clock;
        }
        Some(())
    }
}

impl FoodRoute {
    /// Borrow the planned route. Retain counters, never a copied tile sequence.
    /// The observed run prefix ends when its energy budget is exhausted.
    pub fn measure(
        route: RouteView<'_>,
        zones: ZoneTableView<'_>,
        tables: &RiskTables,
        run_ticks: u16,
    ) -> Option<Self> {
        if route.pack() != zones.pack() {
            return None;
        }
        let mut times: Vec<FoodZoneTime> = Vec::new();
        let destination = [route.destination()];
        for leg in route
            .legs()
            .chain(std::iter::once(LegView::Walk(&destination)))
        {
            let (tiles, hold) = match leg {
                LegView::Walk(tiles) => (tiles, None),
                LegView::Transport(hop) => (&[][..], Some(hop.effective_takeoff())),
            };
            for tile in tiles.iter().copied().chain(hold) {
                for zone in zones.containing(tile) {
                    if !times.iter().any(|time| time.zone == zone.key()) {
                        times.push(FoodZoneTime {
                            zone: zone.key(),
                            walk: FoodTime::default(),
                            run: FoodTime::default(),
                        });
                    }
                }
            }
        }
        for time in &mut times {
            let zone = zones.zone(time.zone).ok()?;
            let kind = tables.zone_kind(zone.kind());
            let mut walk = FoodTrack::default();
            let mut run = FoodTrack::default();
            let mut remaining = run_ticks;
            for leg in route.legs() {
                match leg {
                    LegView::Walk(tiles) => {
                        let mut second = false;
                        for &tile in tiles {
                            walk.visit(tile, 1, zone, kind)?;
                            if run_ticks != 0 {
                                let delta = if second {
                                    second = false;
                                    0
                                } else if remaining != 0 {
                                    remaining -= 1;
                                    second = true;
                                    1
                                } else {
                                    1
                                };
                                run.visit(tile, delta, zone, kind)?;
                            }
                        }
                    }
                    LegView::Transport(hop) => {
                        let ticks = u32::try_from(hop.ticks()).ok()?;
                        walk.visit(hop.effective_takeoff(), ticks, zone, kind)?;
                        walk.visit(hop.landing(), 0, zone, kind)?;
                        if run_ticks != 0 {
                            run.visit(hop.effective_takeoff(), ticks, zone, kind)?;
                            run.visit(hop.landing(), 0, zone, kind)?;
                        }
                    }
                }
            }
            walk.visit(route.destination(), 0, zone, kind)?;
            if run_ticks != 0 {
                run.visit(route.destination(), 0, zone, kind)?;
            }
            time.walk = walk.time;
            time.run = if run_ticks == 0 { walk.time } else { run.time };
        }
        Some(Self {
            pack: zones.pack(),
            zones: times.into_boxed_slice(),
        })
    }

    /// Food-only occupancy at the observed movement speed. An unvisited zone
    /// has zero exposure; an unusable measurement needs the walking fallback.
    pub fn ticks(&self, zone: BoundZoneKey, run: bool) -> Option<u16> {
        if zone.pack() != self.pack {
            return None;
        }
        let Some(time) = self.zones.iter().find(|time| time.zone == zone) else {
            return Some(0);
        };
        u16::try_from(if run { time.run.ticks } else { time.walk.ticks }).ok()
    }

    /// One poison clock from first entry to last exit, including gaps and holds.
    pub fn crossing_ticks(
        &self,
        zones: impl IntoIterator<Item = BoundZoneKey>,
        run: bool,
    ) -> Option<u16> {
        let mut first = u32::MAX;
        let mut last = 0;
        for zone in zones {
            if zone.pack() != self.pack {
                return None;
            }
            let Some(time) = self.zones.iter().find(|time| time.zone == zone) else {
                continue;
            };
            let time = if run { &time.run } else { &time.walk };
            if time.ticks != 0 {
                first = first.min(time.first);
                last = last.max(time.last);
            }
        }
        if first == u32::MAX {
            return Some(0);
        }
        u16::try_from(last.checked_sub(first)?.checked_add(1)?).ok()
    }

    fn at_anchor(&self, zone: BoundZoneKey, run: bool) -> bool {
        self.zones.iter().any(|time| {
            time.zone == zone
                && if run {
                    time.run.at_anchor
                } else {
                    time.walk.at_anchor
                }
        })
    }
}

struct FoodZone {
    exposure: usize,
    zone: BoundZoneKey,
    crossing: usize,
    walking_ticks: u16,
}

impl FoodZone {
    fn ticks(&self, route: Option<&FoodRoute>, run: bool) -> u16 {
        route
            .and_then(|route| route.ticks(self.zone, run))
            .unwrap_or(self.walking_ticks)
    }
}

pub struct StageSurvival {
    exposures: Arc<[SupplyExposure]>,
    /// One poison clock for the stage, including concurrent zone attackers.
    duration_ticks: u64,
    food_zones: Arc<[FoodZone]>,
    /// Every combat machine may put on its opening kit during this stage.
    kits: Arc<[Arc<CompiledKit>]>,
    inventory: super::compile::StageInventory,
}

pub struct CompiledSurvival {
    tables: Arc<CombatTables>,
    stages: Box<[StageSurvival]>,
    choices: Box<[Arc<CompiledKit>]>,
    /// Automatic gear never takes a slot claimed anywhere in the Path.
    equipment_slots: u16,
}

pub struct SurvivalNeed {
    pub food: Option<(BankItem, i32)>,
    pub gear: Vec<BankItem>,
    /// Bound using the supplies the account can actually provide, before room caps.
    pub bound: f64,
    pub unknown: Option<UnknownWhy>,
}

const _: () = assert!(std::mem::size_of::<StageSurvival>() == 80);
const _: () = assert!(std::mem::size_of::<CompiledSurvival>() <= 48);
const _: () = assert!(std::mem::size_of::<SurvivalNeed>() <= 88);

impl CompiledSurvival {
    pub(super) fn compile(
        sequences: &[CompiledSequence],
        role: &super::path::PathRoleDocument,
        header: &super::path::QuestHeaderDocument,
        cx: &CompileContext<'_>,
        zones: Option<ZoneTableView<'_>>,
        prelude: &[super::compile::CompiledStep],
    ) -> Result<Option<Arc<Self>>, CompileError> {
        if !sequences.iter().any(|sequence| {
            !sequence.cross.is_empty()
                || sequence
                    .steps
                    .iter()
                    .any(|step| step.plan.combat_request().is_some())
        }) && !prelude
            .iter()
            .any(|step| step.plan.combat_request().is_some())
            && !role
                .sequences
                .iter()
                .flat_map(|sequence| &sequence.steps)
                .chain(&role.prelude)
                .any(|step| {
                    let mut present = false;
                    walk_crossings(&step.args, &mut |_| present = true);
                    present
                })
        {
            return Ok(None);
        }
        let revision = match cx.selected.revision() {
            274 => api::selected::ClientRevision::R274,
            289 => api::selected::ClientRevision::R289,
            377 => api::selected::ClientRevision::R377,
            _ => return Err(CompileError::code("combat-tables-unavailable")),
        };
        let selected = api::game_data::for_revision(revision)
            .map_err(|_| CompileError::code("combat-tables-unavailable"))?;
        let tables = CombatTables::build(selected)
            .map_err(|_| CompileError::code("combat-tables-unavailable"))?;
        let risks = RiskTables::build(&tables);
        let mut choices = Vec::new();
        for name in header.loadouts.keys() {
            let name = name.as_str();
            let qualified = format!("{}/{}", cx.path.0, name);
            let row = cx
                .loadouts
                .resolve(&qualified)
                .ok_or_else(|| CompileError::code("unknown-loadout"))?
                .row();
            // Utility loadouts are not alternative combat equipment.
            if row.unassigned.is_empty() && !row.worn.is_empty() {
                choices.push(super::families::combat::compile_kit(name, cx)?);
            }
        }
        let choices = choices.into_boxed_slice();
        let mut stages = Vec::with_capacity(sequences.len());
        for (sequence, authored) in sequences.iter().zip(&role.sequences) {
            let mut exposures = Vec::new();
            let mut duration_ticks = 0u64;
            let mut food_zones = Vec::new();
            let mut kits = Vec::new();
            let mut crossings = sequence.cross.to_vec();
            for step in authored.steps.iter().chain(&role.prelude) {
                walk_crossings(&step.args, &mut |name| {
                    if !crossings.iter().any(|known| known.as_ref() == name) {
                        crossings.push(Arc::from(name));
                    }
                });
            }
            for step in sequence.steps.iter().chain(prelude) {
                let Some(request) = step.plan.combat_request() else {
                    continue;
                };
                duration_ticks = duration_ticks
                    .checked_add(u64::from(request.budget_ticks))
                    .ok_or_else(|| CompileError::code("survival-exposure-overflow"))?;
                if let Target::Npc { types, .. } = &request.target {
                    for &ident in types.iter() {
                        let kind = risks.npc(ident).copied().unwrap_or_else(unknown_kind);
                        exposures.push(SupplyExposure {
                            ident,
                            kind,
                            ticks: request.budget_ticks,
                        });
                    }
                } else {
                    // A future player or attacker is not a known NPC type.
                    // Preserve that uncertainty; it is not a zero-risk stage.
                    exposures.push(SupplyExposure {
                        ident: -1,
                        kind: unknown_kind(),
                        ticks: request.budget_ticks,
                    });
                }
                for kit in [
                    &request.kits.melee,
                    &request.kits.ranged,
                    &request.kits.mage,
                ]
                .into_iter()
                .flatten()
                {
                    if !kits.iter().any(|known| Arc::ptr_eq(known, kit)) {
                        kits.push(Arc::clone(kit));
                    }
                }
                if let Some(cross) = &request.cross {
                    for name in cross.iter() {
                        if !crossings.contains(name) {
                            crossings.push(Arc::clone(name));
                        }
                    }
                }
            }
            if !crossings.is_empty() {
                let zones = zones.ok_or_else(|| CompileError::code("cross-catalog-unavailable"))?;
                for (crossing, name) in crossings.into_iter().enumerate() {
                    let key = zones
                        .resolve(&name)
                        .ok_or_else(|| CompileError::code("unknown-cross-zone"))?;
                    for zone in zones
                        .covered_zones(key)
                        .map_err(|_| CompileError::code("unknown-cross-zone"))?
                    {
                        let kind = risks.zone_kind(zone.kind());
                        // Keep the original risk::supply_bound exposure. Route-speed
                        // food estimates below cannot weaken admission or this bound.
                        let (x0, z0, x1, z1) = zone.bounds();
                        let ticks = x1
                            .checked_sub(x0)
                            .and_then(|x| z1.checked_sub(z0).and_then(|z| x.checked_add(z)))
                            .and_then(|span| span.checked_mul(3))
                            .and_then(|span| span.checked_add(4))
                            .and_then(|ticks| u16::try_from(ticks).ok())
                            .ok_or_else(|| CompileError::code("survival-exposure-overflow"))?;
                        // Without a planned route, retain the full walking bound;
                        // observing run alone must never shrink this fallback.
                        let walking_ticks = ticks;
                        food_zones.push(FoodZone {
                            exposure: exposures.len(),
                            zone: zone.key(),
                            crossing,
                            walking_ticks,
                        });
                        exposures.push(SupplyExposure {
                            ident: zone.kind().npc_id().unwrap_or(-1),
                            kind,
                            ticks,
                        });
                    }
                }
            }
            stages.push(StageSurvival {
                exposures: exposures.into(),
                duration_ticks,
                food_zones: food_zones.into(),
                kits: kits.into(),
                inventory: super::compile::stage_inventory(
                    authored.steps.iter().chain(&role.prelude),
                    header,
                    cx,
                )?,
            });
        }
        let equipment_slots = stages
            .iter()
            .fold(0, |mask, stage| mask | stage.inventory.equipment_slots);
        Ok(Some(Arc::new(Self {
            tables,
            stages: stages.into_boxed_slice(),
            choices,
            equipment_slots,
        })))
    }

    pub fn tables(&self) -> &CombatTables {
        &self.tables
    }

    pub(super) fn equipment_slots(&self) -> u16 {
        self.equipment_slots
    }

    pub fn stage_has_risk(&self, stage: usize) -> bool {
        self.stages
            .get(stage)
            .is_some_and(|stage| !stage.exposures.is_empty())
    }

    pub(super) fn stage_has_crossing(&self, stage: usize) -> bool {
        self.stages
            .get(stage)
            .is_some_and(|stage| !stage.food_zones.is_empty())
    }

    pub(crate) fn stage_inventory(&self, stage: usize) -> Option<&super::compile::StageInventory> {
        self.stages.get(stage).map(|stage| &stage.inventory)
    }

    /// Name the priced NPCs only when preparation emits a stage result.
    pub(super) fn stage_targets(&self, stage: usize) -> String {
        let Some(stage) = self.stages.get(stage) else {
            return "unknown exposure".to_owned();
        };
        let mut text = String::new();
        for (index, exposure) in stage.exposures.iter().enumerate() {
            if stage.exposures[..index]
                .iter()
                .any(|row| row.ident == exposure.ident)
            {
                continue;
            }
            if !text.is_empty() {
                text.push_str(", ");
            }
            if let Some(npc) = self.tables.npc(exposure.ident) {
                text.push_str(npc.display.as_deref().unwrap_or(&npc.config));
            } else {
                use std::fmt::Write;
                let _ = write!(&mut text, "exposure {}", exposure.ident);
            }
        }
        text
    }

    pub fn plan(
        &self,
        stage: usize,
        stock: Stock<'_>,
        input: RiskInput,
        owner: Option<&CompiledProvisioning>,
        food_route: Option<&FoodRoute>,
    ) -> Result<SurvivalNeed, UnknownWhy> {
        let stage = self.stages.get(stage).ok_or(UnknownWhy::MissingFacts)?;
        let mut required = Vec::new();
        for kit in stage.kits.iter() {
            self.add_gear(kit, &mut required, stock, owner)?;
        }
        let mut best: Option<SurvivalNeed> = None;
        let food_unknown = self.food_unknown(stock, owner);
        let mut food_candidate = None;
        // Prefer an already worn kit when it is as safe as a banked guide kit.
        for choice in std::iter::once(None).chain(self.choices.iter().map(Some)) {
            if choice.is_some_and(|kit| {
                kit.worn.iter().any(|&(_, id)| {
                    self.gear_allowed(id, stock, owner)
                        && stock.total(id).is_some_and(|count| count <= 0)
                })
            }) {
                continue;
            }
            let mut candidate = input;
            candidate.style_def = 0;
            if let Some(kit) = choice {
                candidate.worn_defence = if let Some(owner) = owner {
                    self.owned_defence(kit, stock, owner)?
                } else {
                    self.defence(kit)?
                };
            }
            for kit in stage.kits.iter() {
                let defence = self.defence(kit)?;
                for (value, forced) in candidate.worn_defence.iter_mut().zip(defence) {
                    *value = (*value).min(forced);
                }
            }
            let no_food = Food {
                item: 0,
                heal: 0,
                count: 0,
            };
            let (mut bound, mut unknown) = supply_score(risk::supply_bound(
                &stage.exposures,
                &candidate,
                &self.tables,
                no_food,
            ));
            unknown = unknown.or(food_unknown);
            if unknown.is_some() {
                bound = 1.0;
            }
            let food = if bound <= risk::consts::DEATH_BOUND {
                None
            } else {
                let food = *food_candidate
                    .get_or_insert_with(|| self.food_candidate(stock, unknown.is_some(), owner));
                self.food_need(
                    stage,
                    &candidate,
                    unknown.is_some(),
                    food,
                    owner,
                    food_route,
                )?
            };
            if let Some((item, target)) = &food {
                let fact = self.tables.food(item.id).ok_or(UnknownWhy::MissingFacts)?;
                let reserved = FoodReserve::reserved(food_reserves(owner), item.id);
                let (value, why) = supply_score(risk::supply_bound(
                    &stage.exposures,
                    &candidate,
                    &self.tables,
                    Food {
                        item: item.id,
                        heal: u8::try_from(fact.heal).map_err(|_| UnknownWhy::Overflow)?,
                        count: FoodReserve::spendable(
                            food_reserves(owner),
                            item.id,
                            stock
                                .held(item.id)
                                .unwrap_or(0)
                                .saturating_add(stock.banked(item.id).unwrap_or(0)),
                        )
                        .min(target.saturating_sub(reserved))
                        .clamp(0, 28) as u8,
                    },
                ));
                unknown = unknown.or(why);
                bound = if unknown.is_some() { 1.0 } else { value };
            }
            let mut gear = required.clone();
            if let Some(kit) = choice {
                self.add_gear(kit, &mut gear, stock, owner)?;
            }
            let need = SurvivalNeed {
                food,
                gear,
                bound,
                unknown,
            };
            // An unread bank cannot invalidate observed food or force a
            // better-kit trip when the carried supplies already meet the bound.
            if choice.is_none()
                && need.bound <= risk::consts::DEATH_BOUND
                && need.food.as_ref().is_none_or(|(item, target)| {
                    stock.held(item.id).is_some_and(|held| held >= *target)
                })
                && need
                    .gear
                    .iter()
                    .all(|item| stock.carried(item.id).is_some_and(|count| count > 0))
            {
                return Ok(need);
            }
            if best.as_ref().is_none_or(|best| {
                let admitted = need.bound <= risk::consts::DEATH_BOUND;
                let best_admitted = best.bound <= risk::consts::DEATH_BOUND;
                if admitted && best_admitted {
                    need.food.as_ref().map_or(0, |(_, qty)| *qty)
                        < best.food.as_ref().map_or(0, |(_, qty)| *qty)
                } else {
                    need.bound < best.bound
                }
            }) {
                best = Some(need);
            }
        }
        best.ok_or(UnknownWhy::MissingFacts)
    }

    fn food_candidate(
        &self,
        stock: Stock<'_>,
        unknown: bool,
        owner: Option<&CompiledProvisioning>,
    ) -> Option<(i32, &str, i32, i32)> {
        // Retain one-kind sizing until the shared fight solver replaces it.
        // Only the authored must-have quantity is reserved.
        let held_food = stock
            .pack
            .into_iter()
            .flatten()
            .filter(|row| row.count > 0 && self.tables.food(row.def.id).is_some())
            .filter(|row| {
                FoodReserve::spendable(
                    food_reserves(owner),
                    row.def.id,
                    stock.held(row.def.id).unwrap_or(0),
                ) > 0
            })
            .map(|row| row.def.id)
            .next();
        let mut fallback = None;
        let mut available = None;
        for item in self.tables.selected().items() {
            if !item.stackable && FoodReserve::reserved(food_reserves(owner), item.id) >= 28 {
                continue;
            }
            if !unknown && held_food.is_some_and(|id| id != item.id) {
                continue;
            }
            let Some(fact) = self.tables.food(item.id).filter(|fact| {
                fact.heal > 0
                    && fact.eat_delay_arg == Some(2)
                    && fact.message_delay.is_none_or(|delay| delay <= 0)
            }) else {
                continue;
            };
            let Some(name) = item.name.as_deref() else {
                continue;
            };
            let candidate = (
                item.id,
                name,
                fact.heal,
                Stock::slots_for(28, item.stackable),
            );
            if fallback.is_none_or(|previous| food_candidate_better(candidate, previous)) {
                fallback = Some(candidate);
            }
            if FoodReserve::spendable(
                food_reserves(owner),
                item.id,
                stock
                    .held(item.id)
                    .unwrap_or(0)
                    .saturating_add(stock.banked(item.id).unwrap_or(0)),
            ) > 0
                && available.is_none_or(|previous| food_candidate_better(candidate, previous))
            {
                available = Some(candidate);
            }
        }
        available.or(fallback)
    }

    fn food_need(
        &self,
        stage: &StageSurvival,
        input: &RiskInput,
        unknown: bool,
        food: Option<(i32, &str, i32, i32)>,
        owner: Option<&CompiledProvisioning>,
        food_route: Option<&FoodRoute>,
    ) -> Result<Option<(BankItem, i32)>, UnknownWhy> {
        let Some((id, name, heal, _)) = food else {
            return Ok(None);
        };
        let reserved = FoodReserve::reserved(food_reserves(owner), id);
        let maximum = if self
            .tables
            .selected()
            .item_by_id(id)
            .ok_or(UnknownWhy::MissingFacts)?
            .stackable
        {
            28
        } else {
            28 - reserved.min(28) as u8
        };
        if maximum == 0 {
            return Ok(None);
        }
        let count = if unknown {
            estimated_food_count(
                stage.food_exposures(food_route, input.run()),
                &self.tables,
                input,
                stage.food_duration(food_route, input.run()),
                heal,
                owner.map(|owner| owner.path.0.as_ref()),
            )
            .min(maximum)
        } else {
            self.food_count(stage, input, id, heal, maximum)?
                .unwrap_or(maximum)
        };
        Ok(Some((
            BankItem {
                id,
                name: Arc::from(name),
            },
            reserved.saturating_add(i32::from(count)),
        )))
    }

    fn food_unknown(
        &self,
        stock: Stock<'_>,
        owner: Option<&CompiledProvisioning>,
    ) -> Option<UnknownWhy> {
        let mut held = None;
        let mut mixed = false;
        for row in stock.pack.into_iter().flatten() {
            if row.count <= 0
                || FoodReserve::spendable(
                    food_reserves(owner),
                    row.def.id,
                    stock.held(row.def.id).unwrap_or(0),
                ) == 0
            {
                continue;
            }
            let Some(fact) = self.tables.food(row.def.id) else {
                continue;
            };
            if fact.eat_delay_arg != Some(2) || fact.message_delay.is_some_and(|delay| delay > 0) {
                return Some(UnknownWhy::UnsupportedFood);
            }
            mixed |= held.is_some_and(|id| id != row.def.id);
            held = Some(row.def.id);
        }
        mixed.then_some(UnknownWhy::MixedFood)
    }

    fn food_count(
        &self,
        stage: &StageSurvival,
        input: &RiskInput,
        item: i32,
        heal: i32,
        maximum: u8,
    ) -> Result<Option<u8>, UnknownWhy> {
        let heal = u8::try_from(heal).map_err(|_| UnknownWhy::Overflow)?;
        let admits = |count| {
            risk::supply_bound(
                &stage.exposures,
                input,
                &self.tables,
                Food { item, heal, count },
            )
            .map(|bound| bound.admits(risk::consts::DEATH_BOUND))
        };
        if !admits(maximum)? {
            return Ok(None);
        }
        // Extra identical bites cannot alter the picker before the smaller
        // stock is exhausted. The admitted held target is therefore monotone.
        let (mut lo, mut hi) = (1, maximum);
        while lo < hi {
            let middle = lo + (hi - lo) / 2;
            if admits(middle)? {
                hi = middle;
            } else {
                lo = middle + 1;
            }
        }
        Ok(Some(lo))
    }

    fn defence(&self, kit: &CompiledKit) -> Result<[i16; 4], UnknownWhy> {
        self.defence_ids(kit.worn.iter().map(|&(_, id)| id))
    }

    fn defence_ids(&self, ids: impl Iterator<Item = i32>) -> Result<[i16; 4], UnknownWhy> {
        let mut defence = [0i16; 4];
        for id in ids {
            let bonus = self
                .tables
                .selected()
                .item_combat_bonus(id)
                .ok_or(UnknownWhy::MissingFacts)?;
            for (sum, value) in defence.iter_mut().zip([
                bonus.stabdefence,
                bonus.slashdefence,
                bonus.crushdefence,
                bonus.rangedefence,
            ]) {
                *sum = sum.checked_add(value).ok_or(UnknownWhy::Overflow)?;
            }
        }
        Ok(defence)
    }

    fn wear_mask(&self, id: i32) -> Option<u16> {
        let item = self.tables.selected().item_by_id(id)?;
        let mut mask = 0u16;
        for slot in [
            item.wear_position,
            item.wear_position_2,
            item.wear_position_3,
        ] {
            if slot >= 0 {
                mask |= 1u16.checked_shl(u32::try_from(slot).ok()?)?;
            }
        }
        (mask != 0).then_some(mask)
    }

    fn gear_allowed(
        &self,
        id: i32,
        stock: Stock<'_>,
        owner: Option<&CompiledProvisioning>,
    ) -> bool {
        owner.is_none_or(|owner| {
            super::compile::inventory::gear_allowed(
                self.tables.selected(),
                id,
                stock,
                &owner.managed_items,
                &owner.carried_items,
                self.equipment_slots,
            )
        })
    }

    fn owned_defence(
        &self,
        kit: &CompiledKit,
        stock: Stock<'_>,
        owner: &CompiledProvisioning,
    ) -> Result<[i16; 4], UnknownWhy> {
        let selected = self.tables.selected();
        let mut worn = [-1; u16::BITS as usize];
        for row in stock.worn.ok_or(UnknownWhy::MissingFacts)? {
            if row.count <= 0 {
                continue;
            }
            let slot = selected
                .item_by_id(row.def.id)
                .and_then(|item| usize::try_from(item.wear_position).ok())
                .filter(|&slot| slot < worn.len())
                .ok_or(UnknownWhy::MissingFacts)?;
            worn[slot] = row.def.id;
        }
        for &(_, id) in kit.worn.iter() {
            if !self.gear_allowed(id, stock, Some(owner)) {
                continue;
            }
            let mask = self.wear_mask(id).ok_or(UnknownWhy::MissingFacts)?;
            for existing in &mut worn {
                if *existing >= 0
                    && self
                        .wear_mask(*existing)
                        .is_none_or(|existing| existing & mask != 0)
                {
                    *existing = -1;
                }
            }
            let slot = selected
                .item_by_id(id)
                .and_then(|item| usize::try_from(item.wear_position).ok())
                .filter(|&slot| slot < worn.len())
                .ok_or(UnknownWhy::MissingFacts)?;
            worn[slot] = id;
        }
        self.defence_ids(worn.into_iter().filter(|&id| id >= 0))
    }

    fn add_gear(
        &self,
        kit: &CompiledKit,
        gear: &mut Vec<BankItem>,
        stock: Stock<'_>,
        owner: Option<&CompiledProvisioning>,
    ) -> Result<(), UnknownWhy> {
        for &(_, id) in kit.worn.iter() {
            if !self.gear_allowed(id, stock, owner) {
                continue;
            }
            if let Some(index) = gear.iter().position(|known| known.id == id) {
                let item = gear.remove(index);
                gear.push(item);
                continue;
            }
            let name = self
                .tables
                .selected()
                .item_by_id(id)
                .and_then(|item| item.name.as_deref())
                .ok_or(UnknownWhy::MissingFacts)?;
            gear.push(BankItem {
                id,
                name: Arc::from(name),
            });
        }
        Ok(())
    }
}

impl StageSurvival {
    fn food_exposures<'a>(
        &'a self,
        route: Option<&'a FoodRoute>,
        run: bool,
    ) -> impl Iterator<Item = SupplyExposure> + 'a {
        self.exposures
            .iter()
            .enumerate()
            .map(move |(index, exposure)| {
                let mut exposure = *exposure;
                if let Some(zone) = self.food_zones.iter().find(|zone| zone.exposure == index) {
                    exposure.ticks = zone.ticks(route, run);
                    if route.is_some_and(|route| route.at_anchor(zone.zone, run)) {
                        // Declared combat work is stage time at the anchor.
                        // death_troll_thrower.rs2:8-11 keeps applayer2 between throws.
                        exposure.ticks = exposure
                            .ticks
                            .saturating_add(u16::try_from(self.duration_ticks).unwrap_or(u16::MAX));
                    }
                }
                exposure
            })
    }

    fn food_duration(&self, route: Option<&FoodRoute>, run: bool) -> u64 {
        let mut duration = self.duration_ticks;
        let mut crossing = None;
        let mut longest = 0;
        let mut crossing_route = None;
        for zone in self.food_zones.iter() {
            if crossing != Some(zone.crossing) {
                duration += u64::from(longest);
                crossing = Some(zone.crossing);
                let span = route.and_then(|route| {
                    route.crossing_ticks(
                        self.food_zones
                            .iter()
                            .filter(|member| member.crossing == zone.crossing)
                            .map(|member| member.zone),
                        run,
                    )
                });
                longest = span.unwrap_or(0);
                crossing_route = span.and(route);
            }
            longest = longest.max(zone.ticks(crossing_route, run));
        }
        duration + u64::from(longest)
    }
}

fn walk_crossings(value: &serde_json::Value, visit: &mut impl FnMut(&str)) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                if key == "cross" {
                    if let Some(rows) = value.as_array() {
                        for row in rows {
                            if let Some(name) = row.as_str() {
                                visit(name);
                            }
                        }
                    }
                } else {
                    walk_crossings(value, visit);
                }
            }
        }
        serde_json::Value::Array(rows) => {
            for row in rows {
                walk_crossings(row, visit);
            }
        }
        _ => {}
    }
}

fn food_reserves(owner: Option<&CompiledProvisioning>) -> &[FoodReserve] {
    owner
        .and_then(|owner| owner.food_reserves.as_deref())
        .unwrap_or(&[])
}

pub(super) fn food_candidate_better(
    candidate: (i32, &str, i32, i32),
    previous: (i32, &str, i32, i32),
) -> bool {
    candidate.2 > previous.2
        || (candidate.2 == previous.2
            && (candidate.3 < previous.3
                || (candidate.3 == previous.3 && candidate.0 < previous.0)))
}

fn supply_score(result: Result<risk::odds::DeathBound, UnknownWhy>) -> (f64, Option<UnknownWhy>) {
    match result {
        Ok(bound) => (bound.bound + f64::from(bound.visits) * f64::EPSILON, None),
        Err(why) => (1.0, Some(why)),
    }
}

/// A policy reserve, not an admission bound. Four ordinary bites replace the
/// old full-pack guess. Underground Pass has repeatable, unbounded failures
/// (quest-guides.md, "Damage from a step"), so retain two such reserves there.
const UNKNOWN_FOOD_DEFAULT: u8 = 4;
const UNKNOWN_FOOD_OVERRIDES: &[(&str, u8)] = &[("upass", 8)];

fn fallback_food_count(quest: Option<&str>) -> u8 {
    UNKNOWN_FOOD_OVERRIDES
        .iter()
        .find(|(id, _)| Some(*id) == quest)
        .map_or(UNKNOWN_FOOD_DEFAULT, |(_, count)| *count)
}

fn estimated_food_count(
    exposures: impl IntoIterator<Item = SupplyExposure>,
    tables: &CombatTables,
    input: &RiskInput,
    duration_ticks: u64,
    heal: i32,
    quest: Option<&str>,
) -> u8 {
    let Some((twice_damage, max_hit, poison)) = expected_damage(exposures, tables) else {
        return fallback_food_count(quest);
    };
    let poison = match input.poison {
        api::condition::PoisonState::Clear | api::condition::PoisonState::Immune { .. } => poison,
        api::condition::PoisonState::Poisoned { per_tick, .. }
        | api::condition::PoisonState::Onset {
            q: Some(per_tick), ..
        } => poison.max(u64::from(per_tick)),
        _ => poison.max(u64::from(tables.unknown_poison_pulse())),
    };
    // Content skill_combat/scripts/poison.rs2:72-75 arms the next pulse
    // with settimer(poison, 30); concurrent attackers share that one clock.
    let twice_damage = twice_damage + 2 * poison * duration_ticks.div_ceil(30);
    damage_food_count(twice_damage, max_hit, input.hp_max, heal)
        .unwrap_or_else(|| fallback_food_count(quest))
}

/// Uniform randominc(max_hit) has mean max_hit / 2. Charge every attack as a
/// hit, with no defence or prayer credit.
fn expected_damage(
    exposures: impl IntoIterator<Item = SupplyExposure>,
    tables: &CombatTables,
) -> Option<(u64, u64, u64)> {
    let (mut twice_damage, mut max_hit, mut poison) = (0u64, 0, 0);
    for exposure in exposures {
        if exposure.ticks == 0 {
            continue;
        }
        let kind = exposure.kind;
        let hit = if kind.unknown_for().is_none() {
            u64::from(kind.max_hit)
        } else {
            let row = tables.npc(exposure.ident)?;
            // death_troll_thrower.rs2:20-32 calls npc_ranged_maxhit and
            // randominc, without an accuracy roll or a poison operation.
            if kind.unknown != risk::UnknownKind::Bespoke
                || !matches!(
                    row.config.as_str(),
                    "death_troll_thrower1"
                        | "death_troll_thrower2"
                        | "death_troll_thrower3"
                        | "death_troll_thrower4"
                        | "death_troll_thrower5"
                )
            {
                return None;
            }
            let ranged = u64::try_from(row.ranged?).ok()?;
            let bonus = u64::try_from(row.rangebonus?.checked_add(64)?).ok()?;
            // npc/npc_combat_ranged.rs2:7-13 and combat.rs2:7-12.
            u64::from(u8::try_from(((ranged + 9) * bonus + 320) / 640).ok()?)
        };
        let rate = u64::from(kind.rate);
        if rate == 0 || exposure.ticks == 0 {
            return None;
        }
        twice_damage += if kind.hazard() || kind.unknown_for().is_none() && !kind.shared_attack() {
            2 * hit
                * if kind.hazard() {
                    1
                } else {
                    u64::from(exposure.ticks).div_ceil(rate)
                }
        } else {
            u64::from(exposure.ticks).div_ceil(rate) * hit
        };
        max_hit = max_hit.max(hit);
        poison = poison.max(u64::from(kind.poison));
    }
    Some((twice_damage, max_hit, poison))
}

fn damage_food_count(twice_damage: u64, max_hit: u64, hp_max: u8, heal: i32) -> Option<u8> {
    let heal = u64::try_from(heal).ok()?.min(u64::from(hp_max));
    if heal == 0 {
        return None;
    }
    // 50% above the mean plus one full opening hit covers the maximum of
    // <=4 uniform throws: .75*n*m + m >= n*m. For the verified throwers
    // (m=7), its distance above the mean is >=3.05 standard deviations for
    // every n>=5: (.25*n*7+7)/sqrt(n*7*9/12), minimized near n=4.
    // The opening hit also covers a due projectile before a bite heals
    // (supplies.rs:7-10). This is a reserve, not a death-probability bound.
    let damage = twice_damage
        .checked_mul(3)?
        .div_ceil(4)
        .checked_add(max_hit)?;
    Some(damage.div_ceil(heal).clamp(1, 28) as u8)
}

fn unknown_kind() -> risk::KindRisk {
    risk::KindRisk::new(
        0,
        1,
        0,
        0,
        risk::Style::Melee,
        risk::UnknownKind::MissingFacts,
        0,
        0,
        false,
        false,
        false,
        false,
    )
}

#[cfg(test)]
mod food_sizing_tests {
    use super::*;

    #[test]
    fn energy_budget_uses_the_engine_drain_and_no_recovery_credit() {
        assert_eq!(RiskInput::available_run_ticks(0, 0), 0);
        assert_eq!(RiskInput::available_run_ticks(1, 8), 1);
        assert_eq!(RiskInput::available_run_ticks(100, 64), 10000 / 134);
        assert_eq!(RiskInput::available_run_ticks(50, 8), 5000 / 76);
        assert_eq!(RiskInput::available_run_ticks(100, -5), 10000 / 67);
    }

    #[test]
    fn engaged_attacker_follows_beyond_the_hunt_zone_at_walk_speed() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (lease, name) = risk::provisioning_cross_fixture();
        let zones = lease.zones().unwrap();
        let zone = zones.zone(zones.resolve(&name).unwrap()).unwrap();
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let mut kind = RiskTables::build(&tables).zone_kind(zone.kind());
        kind.r = 28;
        kind.reach = 8;
        let (x, z) = zone.spawn();
        let start = api::WorldTile {
            x,
            z,
            level: i32::from(zone.plane()),
        };
        assert!(zone.contains(start));
        let mut walk = FoodTrack::default();
        let mut run = FoodTrack::default();
        walk.visit(start, 1, zone, kind).unwrap();
        run.visit(start, 1, zone, kind).unwrap();
        let mut in_zone = 1;
        for step in 1..=28 {
            let tile = api::WorldTile {
                x: x + step,
                ..start
            };
            in_zone += u32::from(zone.contains(tile));
            walk.visit(tile, 1, zone, kind).unwrap();
            run.visit(tile, u32::from(step % 2 == 1), zone, kind)
                .unwrap();
        }
        assert_eq!(walk.time.ticks, 29);
        assert!(
            walk.time.ticks > in_zone,
            "zone-only counting loses pursuit"
        );
        assert!(
            run.time.ticks < walk.time.ticks,
            "a walker cannot follow a runner at run speed"
        );
        assert!(walk.time.at_anchor);
        walk.visit(api::WorldTile { x: x + 29, ..start }, 1, zone, kind)
            .unwrap();
        assert_eq!(
            walk.time.ticks, 29,
            "the spawn pursuit radius ends engagement"
        );
        assert!(!walk.time.at_anchor);
    }

    #[test]
    fn food_geometry_keeps_the_per_bot_layout_budget() {
        let provisioner = std::mem::size_of::<super::super::provision::Provisioner>();
        let quester = std::mem::size_of::<super::super::runner::Quester>();
        eprintln!(
            "food geometry layouts Provisioner={provisioner} Quester={quester} StageSurvival={} Ledger={}",
            std::mem::size_of::<StageSurvival>(),
            std::mem::size_of::<crate::native::ledger::Ledger>(),
        );
        assert!(provisioner <= 1184);
        assert!(
            quester + 64 <= 4000,
            "the route cache must retain 64 bytes of Quester headroom"
        );
    }

    #[test]
    fn route_food_geometry_uses_observed_speed_not_a_zone_name() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (lease, name) = risk::provisioning_cross_fixture();
        let zones = lease.zones().unwrap();
        let key = zones.resolve(&name).unwrap();
        let zone = zones.zone(key).unwrap();
        let (x0, _, x1, _) = zone.bounds();
        let walking = u16::try_from(x1 - x0 + 1).unwrap();
        let mut measured = FoodRoute {
            pack: zones.pack(),
            zones: vec![FoodZoneTime {
                zone: key,
                walk: FoodTime {
                    ticks: u32::from(walking),
                    first: 1,
                    last: u32::from(walking) + 10,
                    at_anchor: false,
                },
                run: FoodTime {
                    ticks: u32::from(walking.div_ceil(2)),
                    first: 1,
                    last: u32::from(walking.div_ceil(2)) + 5,
                    at_anchor: false,
                },
            }]
            .into_boxed_slice(),
        };
        assert_eq!(measured.ticks(key, false), Some(walking));
        assert_eq!(measured.ticks(key, true), Some(walking.div_ceil(2)));
        assert_eq!(
            measured.crossing_ticks([key], false),
            Some(walking + 10),
            "poison includes gaps between zone visits"
        );
        assert_eq!(
            measured.crossing_ticks([key], true),
            Some(walking.div_ceil(2) + 5)
        );
        let unvisited = FoodRoute {
            pack: zones.pack(),
            zones: Box::new([]),
        };
        assert_eq!(unvisited.ticks(key, false), Some(0));
        assert_eq!(unvisited.ticks(key, true), Some(0));
        let (foreign, foreign_name) = risk::provisioning_cross_fixture();
        assert_eq!(
            measured.ticks(
                foreign.zones().unwrap().resolve(&foreign_name).unwrap(),
                true
            ),
            None
        );
        let fallback = FoodZone {
            exposure: 0,
            zone: key,
            crossing: 0,
            walking_ticks: 100,
        };
        assert_eq!(fallback.ticks(None, false), 100);
        assert_eq!(
            fallback.ticks(None, true),
            100,
            "unmeasured run must not reduce the walking fallback"
        );
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let exposure = SupplyExposure {
            ident: zone.kind().npc_id().unwrap(),
            kind: RiskTables::build(&tables).zone_kind(zone.kind()),
            ticks: 100,
        };
        let mut stage = StageSurvival {
            exposures: Arc::from([exposure]),
            duration_ticks: 0,
            food_zones: Arc::from([fallback]),
            kits: Arc::from([]),
            inventory: super::super::compile::StageInventory {
                items: Arc::from([]),
                equipment_slots: 0,
            },
        };
        let input = RiskInput::default();
        let food = Food {
            item: 379,
            heal: 12,
            count: 22,
        };
        let original =
            risk::supply_bound(&stage.exposures, &input, &tables, food).map(|bound| bound.bound);
        assert_eq!(
            stage
                .food_exposures(Some(&measured), false)
                .next()
                .unwrap()
                .ticks,
            walking
        );
        assert_eq!(
            stage
                .food_exposures(Some(&measured), true)
                .next()
                .unwrap()
                .ticks,
            walking.div_ceil(2)
        );
        assert_eq!(stage.food_exposures(None, true).next().unwrap().ticks, 100);
        assert_eq!(
            stage
                .food_exposures(Some(&unvisited), true)
                .next()
                .unwrap()
                .ticks,
            0,
            "an assessed route outside the hunt zones has no crossing exposure"
        );
        assert_eq!(stage.food_duration(Some(&unvisited), true), 0);
        assert_eq!(
            stage.food_duration(Some(&measured), false),
            u64::from(walking + 10)
        );
        assert_eq!(
            stage.food_duration(Some(&measured), true),
            u64::from(walking.div_ceil(2) + 5)
        );
        assert_eq!(stage.exposures[0].ticks, 100);
        assert_eq!(
            risk::supply_bound(&stage.exposures, &input, &tables, food).map(|bound| bound.bound),
            original,
            "food-only exposure cannot replace supply_bound's retained exposure",
        );
        stage.duration_ticks = 30;
        measured.zones[0].walk.at_anchor = true;
        assert_eq!(
            stage
                .food_exposures(Some(&measured), false)
                .next()
                .unwrap()
                .ticks,
            walking + 30,
            "an engaged attacker remains a food exposure during anchor work",
        );
        assert_eq!(
            stage.food_duration(Some(&measured), false),
            u64::from(walking + 40)
        );
    }

    #[test]
    fn thrower_headroom_covers_four_maxima_and_three_sigmas_afterward() {
        for n in 1..=4 {
            assert!(0.75 * f64::from(n) * 7.0 + 7.0 >= f64::from(n) * 7.0);
        }
        // The expression is increasing for n>4, so its minimum over n>=5 is n=5.
        let n: f64 = 5.0;
        let sigmas = (0.25 * n * 7.0 + 7.0) / (n * 7.0 * 9.0 / 12.0).sqrt();
        assert!(sigmas >= 3.05);
    }

    #[test]
    fn damage_budget_rounds_up_and_keeps_the_opening_hit_margin() {
        // Twenty-five throws, each uniform on 0..=7, need 139 HP of healing.
        assert_eq!(damage_food_count(175, 7, 40, 12), Some(12));
        assert_eq!(damage_food_count(175, 7, 40, 20), Some(7));
        // A small HP cap cannot use the food's full advertised heal.
        assert_eq!(damage_food_count(16, 1, 4, 12), Some(4));
        assert_eq!(damage_food_count(100_000, 70, 99, 12), Some(28));
        assert_eq!(damage_food_count(20, 7, 40, 0), None);
    }

    #[test]
    fn unknown_stage_uses_the_default_or_the_quest_override() {
        assert_eq!(fallback_food_count(None), 4);
        assert_eq!(fallback_food_count(Some("imp")), 4);
        assert_eq!(fallback_food_count(Some("upass")), 8);
    }

    #[test]
    fn estimated_supply_includes_poison_without_claiming_admission() {
        if !api::test_support::game_data_available() {
            return;
        }
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let kind = risk::KindRisk::new(
            2,
            4,
            1,
            1,
            risk::Style::Melee,
            risk::UnknownKind::Known,
            3,
            1000,
            true,
            false,
            false,
            false,
        );
        let exposures = [SupplyExposure {
            ident: -1,
            kind,
            ticks: 31,
        }];
        let mut input = RiskInput::default();
        input.hp_max = 40;
        input.poison = api::condition::PoisonState::Clear;
        // Eight attacks average 8 damage. Two poison pulses add 6.
        assert_eq!(
            estimated_food_count(exposures, &tables, &input, 31, 12, None),
            2
        );
        input.poison = api::condition::PoisonState::Poisoned {
            per_tick: 6,
            last_mark: 0,
            hit_since_mark: false,
        };
        assert_eq!(
            estimated_food_count(exposures, &tables, &input, 31, 12, None),
            3
        );
        let unknown = [SupplyExposure {
            kind: unknown_kind(),
            ..exposures[0]
        }];
        assert_eq!(
            estimated_food_count(unknown, &tables, &input, 31, 12, None),
            4
        );
        assert_eq!(
            estimated_food_count(unknown, &tables, &input, 31, 12, Some("upass")),
            8
        );
    }

    #[test]
    fn concurrent_throwers_share_one_poison_clock() {
        if !api::test_support::game_data_available() {
            return;
        }
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let risks = RiskTables::build(&tables);
        // The plateau has six placements across five NPC types.
        let exposures = [1105, 1101, 1102, 1103, 1104, 1105].map(|ident| SupplyExposure {
            ident,
            kind: *risks.npc(ident).unwrap(),
            ticks: 30,
        });
        let mut input = RiskInput::default();
        input.hp_max = 40;
        input.poison = api::condition::PoisonState::Poisoned {
            per_tick: 11,
            last_mark: 0,
            hit_since_mark: false,
        };
        assert_eq!(expected_damage(exposures, &tables), Some((210, 7, 0)));
        assert_eq!(
            estimated_food_count(exposures, &tables, &input, 30, 12, None),
            16
        );
        // Six sequential exposures do allow six poison pulses.
        assert_eq!(
            estimated_food_count(exposures, &tables, &input, 180, 12, None),
            22
        );
    }
}
