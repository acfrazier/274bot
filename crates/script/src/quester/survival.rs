//! Shared Path-stage obligations and risk-selected bank targets.
use super::compile::{CompileContext, CompileError, CompiledSequence};
use crate::bank::BankItem;
use crate::combat::risk::{self, Food, RiskInput, RiskTables, SupplyExposure, UnknownWhy};
use crate::combat::tables::CombatTables;
use crate::combat::{CompiledKit, Target};
use api::stock::Stock;
use nav_planner::contract::ZoneTableView;
use std::sync::Arc;

pub struct StageSurvival {
    exposures: Arc<[SupplyExposure]>,
    /// Every combat machine may put on its opening kit during this stage.
    kits: Arc<[Arc<CompiledKit>]>,
}

pub struct CompiledSurvival {
    tables: Arc<CombatTables>,
    stages: Box<[StageSurvival]>,
    choices: Box<[Arc<CompiledKit>]>,
}

pub struct SurvivalNeed {
    pub food: Option<(BankItem, i32)>,
    pub gear: Vec<BankItem>,
    /// The selected crossing kit must be observed worn before handing off.
    pub wear: Option<Arc<CompiledKit>>,
    /// Bound using the supplies the account can actually provide, before room caps.
    pub bound: f64,
}

const _: () = assert!(std::mem::size_of::<StageSurvival>() == 32);
const _: () = assert!(std::mem::size_of::<CompiledSurvival>() <= 40);
const _: () = assert!(std::mem::size_of::<SurvivalNeed>() <= 80);

impl CompiledSurvival {
    pub(super) fn compile(
        sequences: &[CompiledSequence],
        authored: &[super::path::SequenceDocument],
        loadouts: impl Iterator<Item = impl AsRef<str>>,
        cx: &CompileContext<'_>,
        zones: Option<ZoneTableView<'_>>,
    ) -> Result<Option<Arc<Self>>, CompileError> {
        if !sequences.iter().any(|sequence| {
            !sequence.cross.is_empty()
                || sequence.steps.iter().any(|step| step.plan.combat_request().is_some())
        }) && !authored.iter().flat_map(|sequence| &sequence.steps).any(|step| {
            let mut present = false;
            walk_crossings(&step.args, &mut |_| present = true);
            present
        }) {
            return Ok(None);
        }
        let selected = api::game_data::for_revision(cx.selected.revision())
            .map_err(|_| CompileError::code("combat-tables-unavailable"))?;
        let tables = CombatTables::build(selected)
            .map_err(|_| CompileError::code("combat-tables-unavailable"))?;
        let risks = RiskTables::build(&tables);
        let mut choices = Vec::new();
        for name in loadouts {
            let name = name.as_ref();
            let qualified = format!("{}/{}", cx.path.0, name);
            let row = cx.loadouts.resolve(&qualified)
                .ok_or_else(|| CompileError::code("unknown-loadout"))?.row();
            // Utility loadouts are not alternative combat equipment.
            if row.unassigned.is_empty() && !row.worn.is_empty() {
                choices.push(super::families::combat::compile_kit(name, cx)?);
            }
        }
        let choices = choices.into_boxed_slice();
        let mut stages = Vec::with_capacity(sequences.len());
        for (sequence, authored) in sequences.iter().zip(authored) {
            let mut exposures = Vec::new();
            let mut kits = Vec::new();
            let mut crossings = sequence.cross.to_vec();
            for step in &authored.steps {
                walk_crossings(&step.args, &mut |name| {
                    if !crossings.iter().any(|known| known.as_ref() == name) {
                        crossings.push(Arc::from(name));
                    }
                });
            }
            for step in &sequence.steps {
                let Some(request) = step.plan.combat_request() else { continue };
                if let Target::Npc { types, .. } = &request.target {
                    for &ident in types.iter() {
                        let kind = risks.npc(ident).copied().ok_or_else(|| {
                            CompileError::code("survival-npc-facts-unavailable")
                        })?;
                        exposures.push(SupplyExposure { ident, kind, ticks: request.budget_ticks });
                    }
                }
                if let Some(kit) = request.kits.get(request.style) {
                    kits.push(Arc::clone(kit));
                }
                if let Some(cross) = &request.cross {
                    for name in cross.iter() {
                        if !crossings.contains(name) { crossings.push(Arc::clone(name)); }
                    }
                }
            }
            if !crossings.is_empty() {
                let zones = zones.ok_or_else(|| CompileError::code("cross-catalog-unavailable"))?;
                for name in crossings {
                    let key = zones.resolve(&name)
                        .ok_or_else(|| CompileError::code("unknown-cross-zone"))?;
                    for zone in zones.covered_zones(key)
                        .map_err(|_| CompileError::code("unknown-cross-zone"))? {
                        let kind = risks.zone_kind(zone.kind());
                        // Supply sizing covers a round trip around the full bounding
                        // rectangle at the route model's stop factor, plus its lag.
                        // Actual geometry and single-way ownership are still judged
                        // by route admission before any crossing is entered.
                        let (x0, z0, x1, z1) = zone.bounds();
                        let ticks = x1.checked_sub(x0)
                            .and_then(|x| z1.checked_sub(z0).and_then(|z| x.checked_add(z)))
                            .and_then(|span| span.checked_mul(3))
                            .and_then(|span| span.checked_add(4))
                            .and_then(|ticks| u16::try_from(ticks).ok())
                            .ok_or_else(|| CompileError::code("survival-exposure-overflow"))?;
                        exposures.push(SupplyExposure {
                            ident: zone.kind().npc_id().unwrap_or(-1), kind, ticks,
                        });
                    }
                }
            }
            stages.push(StageSurvival { exposures: exposures.into(), kits: kits.into() });
        }
        Ok(Some(Arc::new(Self { tables, stages: stages.into_boxed_slice(), choices })))
    }

    pub fn tables(&self) -> &CombatTables { &self.tables }

    pub fn stage_has_risk(&self, stage: usize) -> bool {
        self.stages.get(stage).is_some_and(|stage| !stage.exposures.is_empty())
    }

    pub fn plan(&self, stage: usize, stock: Stock<'_>, input: RiskInput)
        -> Result<SurvivalNeed, UnknownWhy>
    {
        let stage = self.stages.get(stage).ok_or(UnknownWhy::MissingFacts)?;
        let mut required = Vec::new();
        for kit in stage.kits.iter() {
            self.add_gear(kit, &mut required)?;
        }
        let mut best: Option<SurvivalNeed> = None;
        // Prefer an already worn kit when it is as safe as a banked guide kit.
        for choice in std::iter::once(None).chain(self.choices.iter().map(Some)) {
            if choice.is_some_and(|kit| kit.worn.iter().any(|&(_, id)| {
                stock.total(id).is_some_and(|count| count <= 0)
            })) {
                continue;
            }
            let mut candidate = input;
            candidate.style_def = 0;
            if let Some(kit) = choice {
                candidate.worn_defence = self.defence(kit)?;
            }
            for kit in stage.kits.iter() {
                let defence = self.defence(kit)?;
                for (value, forced) in candidate.worn_defence.iter_mut().zip(defence) {
                    *value = (*value).min(forced);
                }
            }
            let no_food = Food { item: 0, heal: 0, count: 0 };
            let mut bound = risk::supply_bound(
                &stage.exposures, &candidate, &self.tables, no_food,
            )?.bound;
            let food = if bound <= risk::consts::DEATH_BOUND {
                None
            } else {
                self.food_need(stage, stock, &candidate)?
            };
            if let Some((item, target)) = &food {
                let fact = self.tables.food(item.id).ok_or(UnknownWhy::MissingFacts)?;
                bound = risk::supply_bound(&stage.exposures, &candidate, &self.tables, Food {
                    item: item.id,
                    heal: u8::try_from(fact.heal).map_err(|_| UnknownWhy::Overflow)?,
                    count: stock.total(item.id).unwrap_or(0).min(*target).clamp(0, 28) as u8,
                })?.bound;
            }
            let mut gear = required.clone();
            if let Some(kit) = choice { self.add_gear(kit, &mut gear)?; }
            let need = SurvivalNeed { food, gear, wear: choice.cloned(), bound };
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

    fn food_need(&self, stage: &StageSurvival, stock: Stock<'_>, input: &RiskInput)
        -> Result<Option<(BankItem, i32)>, UnknownWhy>
    {
        // The guard's homogeneous picker is exact. A different carried food
        // would change its decisions, so do not credit a hypothetical mixture.
        let held_food = stock.pack.into_iter().flatten()
            .filter(|row| row.count > 0 && self.tables.food(row.def.id).is_some())
            .map(|row| row.def.id).next();
        let mixed = stock.pack.into_iter().flatten().any(|row| {
            row.count > 0 && self.tables.food(row.def.id).is_some()
                && Some(row.def.id) != held_food
        });
        if mixed { return Err(UnknownWhy::MissingFacts); }
        let mut fallback = None;
        let mut best: Option<(BankItem, i32, f64)> = None;
        for item in self.tables.selected().items() {
            if held_food.is_some_and(|id| id != item.id) { continue; }
            let Some(fact) = self.tables.food(item.id).filter(|fact| {
                fact.heal > 0 && fact.eat_delay_arg == Some(2)
                    && fact.message_delay.is_none_or(|delay| delay <= 0)
            }) else { continue };
            let Some(name) = item.name.as_deref() else { continue };
            if fallback.is_none_or(|(_, heal)| fact.heal > heal) {
                fallback = Some((item.id, fact.heal));
            }
            let total = stock.total(item.id).unwrap_or(0).clamp(0, 28) as u8;
            if total == 0 { continue; }
            let target = self.food_count(stage, input, item.id, fact.heal, 28)?.unwrap_or(28);
            let bound = risk::supply_bound(&stage.exposures, input, &self.tables, Food {
                item: item.id, heal: fact.heal as u8, count: total.min(target),
            })?.bound;
            if best.as_ref().is_none_or(|(_, count, previous)| {
                if bound <= risk::consts::DEATH_BOUND && *previous <= risk::consts::DEATH_BOUND {
                    i32::from(target) < *count
                } else { bound < *previous }
            }) {
                best = Some((BankItem { id: item.id, name: Arc::from(name) }, i32::from(target), bound));
            }
        }
        if let Some((item, count, _)) = best { return Ok(Some((item, count))); }
        let Some((id, heal)) = fallback else { return Ok(None) };
        let count = self.food_count(stage, input, id, heal, 28)?.unwrap_or(28);
        let name = self.tables.selected().item_by_id(id).and_then(|item| item.name.as_deref())
            .ok_or(UnknownWhy::MissingFacts)?;
        Ok(Some((BankItem { id, name: Arc::from(name) }, i32::from(count))))
    }

    fn food_count(
        &self, stage: &StageSurvival, input: &RiskInput, item: i32, heal: i32, maximum: u8,
    ) -> Result<Option<u8>, UnknownWhy> {
        let heal = u8::try_from(heal).map_err(|_| UnknownWhy::Overflow)?;
        let admits = |count| risk::supply_bound(
            &stage.exposures, input, &self.tables, Food { item, heal, count },
        ).map(|bound| bound.admits(risk::consts::DEATH_BOUND));
        if !admits(maximum)? { return Ok(None); }
        // Extra identical bites cannot alter the picker before the smaller
        // stock is exhausted. The admitted held target is therefore monotone.
        let (mut lo, mut hi) = (1, maximum);
        while lo < hi {
            let middle = lo + (hi - lo) / 2;
            if admits(middle)? { hi = middle; } else { lo = middle + 1; }
        }
        Ok(Some(lo))
    }

    fn defence(&self, kit: &CompiledKit) -> Result<[i16; 4], UnknownWhy> {
        let mut defence = [0i16; 4];
        for &(_, id) in kit.worn.iter() {
            let bonus = self.tables.selected().item_combat_bonus(id)
                .ok_or(UnknownWhy::MissingFacts)?;
            for (sum, value) in defence.iter_mut().zip([
                bonus.stabdefence, bonus.slashdefence, bonus.crushdefence, bonus.rangedefence,
            ]) {
                *sum = sum.checked_add(value).ok_or(UnknownWhy::Overflow)?;
            }
        }
        Ok(defence)
    }

    fn add_gear(&self, kit: &CompiledKit, gear: &mut Vec<BankItem>) -> Result<(), UnknownWhy> {
        for &(_, id) in kit.worn.iter() {
            if gear.iter().any(|known| known.id == id) { continue; }
            let name = self.tables.selected().item_by_id(id)
                .and_then(|item| item.name.as_deref()).ok_or(UnknownWhy::MissingFacts)?;
            gear.push(BankItem { id, name: Arc::from(name) });
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
                            if let Some(name) = row.as_str() { visit(name); }
                        }
                    }
                } else {
                    walk_crossings(value, visit);
                }
            }
        }
        serde_json::Value::Array(rows) => {
            for row in rows { walk_crossings(row, visit); }
        }
        _ => {}
    }
}
