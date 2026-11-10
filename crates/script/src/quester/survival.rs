//! Shared Path-stage obligations and risk-selected bank targets.
use super::compile::{CompileContext, CompileError, CompiledProvisioning, CompiledSequence};
use crate::combat::risk::{self, Food, RiskInput, RiskTables, SupplyExposure, UnknownWhy};
use crate::combat::tables::CombatTables;
use crate::combat::{CompiledKit, Target};
use crate::native_bank::BankItem;
use api::stock::Stock;
use nav_planner::contract::ZoneTableView;
use std::sync::Arc;

pub struct StageSurvival {
    exposures: Arc<[SupplyExposure]>,
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

const _: () = assert!(std::mem::size_of::<StageSurvival>() == 56);
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
                for name in crossings {
                    let key = zones
                        .resolve(&name)
                        .ok_or_else(|| CompileError::code("unknown-cross-zone"))?;
                    for zone in zones
                        .covered_zones(key)
                        .map_err(|_| CompileError::code("unknown-cross-zone"))?
                    {
                        let kind = risks.zone_kind(zone.kind());
                        // Supply sizing covers a round trip around the full bounding
                        // rectangle at the route model's stop factor, plus its lag.
                        // Actual geometry and single-way ownership are still judged
                        // by route admission before any crossing is entered.
                        let (x0, z0, x1, z1) = zone.bounds();
                        let ticks = x1
                            .checked_sub(x0)
                            .and_then(|x| z1.checked_sub(z0).and_then(|z| x.checked_add(z)))
                            .and_then(|span| span.checked_mul(3))
                            .and_then(|span| span.checked_add(4))
                            .and_then(|ticks| u16::try_from(ticks).ok())
                            .ok_or_else(|| CompileError::code("survival-exposure-overflow"))?;
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
                self.food_need(stage, &candidate, unknown.is_some(), food)?
            };
            if let Some((item, target)) = &food {
                let fact = self.tables.food(item.id).ok_or(UnknownWhy::MissingFacts)?;
                let (value, why) = supply_score(risk::supply_bound(
                    &stage.exposures,
                    &candidate,
                    &self.tables,
                    Food {
                        item: item.id,
                        heal: u8::try_from(fact.heal).map_err(|_| UnknownWhy::Overflow)?,
                        count: stock
                            .held(item.id)
                            .unwrap_or(0)
                            .saturating_add(stock.banked(item.id).unwrap_or(0))
                            .min(*target)
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
        // Protected Path food is not a survival food kind.
        let held_food = stock
            .pack
            .into_iter()
            .flatten()
            .filter(|row| row.count > 0 && self.tables.food(row.def.id).is_some())
            .filter(|row| owner.is_none_or(|owner| !owner.carried_items.contains(&row.def.id)))
            .map(|row| row.def.id)
            .next();
        let mut fallback = None;
        let mut available = None;
        for item in self.tables.selected().items() {
            if owner.is_some_and(|owner| owner.carried_items.contains(&item.id)) {
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
            if stock
                .held(item.id)
                .unwrap_or(0)
                .saturating_add(stock.banked(item.id).unwrap_or(0))
                > 0
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
    ) -> Result<Option<(BankItem, i32)>, UnknownWhy> {
        let Some((id, name, heal, _)) = food else {
            return Ok(None);
        };
        let count = if unknown {
            28
        } else {
            self.food_count(stage, input, id, heal, 28)?.unwrap_or(28)
        };
        Ok(Some((
            BankItem {
                id,
                name: Arc::from(name),
            },
            i32::from(count),
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
                || owner.is_some_and(|owner| owner.carried_items.contains(&row.def.id))
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
