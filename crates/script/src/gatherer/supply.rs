//! Fixed-capacity supply planning for Gatherer bank trips.
//!
//! Plans are built only at a loaded-bank boundary. They contain exact final
//! inventory counts, so rune decisions stay latched across the whole withdrawal
//! batch instead of changing as each rune arrives.

use super::card::Prepared;
use super::settings::{GathererSettings, Skill};
use crate::native::ConfigError;
use api::game_data::{SelectedGameData, TeleportSpell};
use api::gather_methods::{known_rows, GatherCatalog};
use api::selected::SkillMinimum;
use api::snapshot::{ItemView, StatView};
use std::sync::Arc;

const COINS_ID: i32 = 995;
const MAX_RESERVE_RUNES: usize = 3;
const MAX_PLAN_ITEMS: usize = 8;
const MAX_PROTECTED_IDS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyItemFact {
    pub id: i32,
    pub name: Arc<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuneCost {
    pub id: i32,
    pub name: Arc<str>,
    pub per_cast: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveSpell {
    pub name: Arc<str>,
    runes: [Option<RuneCost>; MAX_RESERVE_RUNES],
    rune_len: u8,
}

impl ReserveSpell {
    pub fn runes(&self) -> impl Iterator<Item = &RuneCost> {
        self.runes[..usize::from(self.rune_len)]
            .iter()
            .filter_map(Option::as_ref)
    }
}

/// Per-account facts resolved from the selected cache and gathering methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSupply {
    /// The selected object name and id used by Eat and food refills.
    pub food: Option<SupplyItemFact>,
    /// The selected method's single consumed bait item, when it consumes bait.
    pub bait: Option<SupplyItemFact>,
    /// Selected object name for the fixed coin id 995; absent if the cache has
    /// no coin name and no coin target was configured.
    pub coin_name: Option<Arc<str>>,
    /// Selected spell and its selected-cache rune costs; absent for `Off`.
    pub reserve: Option<ReserveSpell>,
}

impl PreparedSupply {
    pub fn prepare(
        settings: &mut GathererSettings,
        selected: &SelectedGameData,
        catalog: &GatherCatalog,
        methods: &[usize],
    ) -> Result<Self, ConfigError> {
        let food = resolve_food(settings, selected)?;
        let bait = resolve_bait(settings, selected, catalog, methods)?;
        let coin_name = selected
            .item_by_id(COINS_ID)
            .and_then(|item| item.name.as_deref())
            .filter(|name| !name.trim().is_empty())
            .map(Arc::from);
        if settings.coin_target > 0 && coin_name.is_none() {
            return Err(ConfigError::new(
                "coinTarget",
                "item-unavailable",
                "the selected cache has no named coins item",
            ));
        }
        let reserve = resolve_reserve(settings, selected)?;
        Ok(Self {
            food,
            bait,
            coin_name,
            reserve,
        })
    }
}

fn resolve_food(
    settings: &mut GathererSettings,
    selected: &SelectedGameData,
) -> Result<Option<SupplyItemFact>, ConfigError> {
    let requested = settings.food.trim();
    if requested.is_empty() {
        settings.food.clear();
        return Ok(None);
    }
    let hit = selected
        .search_named_items(requested, usize::MAX)
        .into_iter()
        .filter(|item| item.name.eq_ignore_ascii_case(requested))
        .min_by_key(|item| item.id)
        .ok_or_else(|| {
            ConfigError::new(
                "food",
                "unknown-item",
                format!("food {requested:?} is not a selected object name"),
            )
        })?;
    settings.food.clone_from(&hit.name);
    Ok(Some(SupplyItemFact {
        id: hit.id,
        name: Arc::from(hit.name),
    }))
}

fn resolve_bait(
    settings: &GathererSettings,
    selected: &SelectedGameData,
    catalog: &GatherCatalog,
    methods: &[usize],
) -> Result<Option<SupplyItemFact>, ConfigError> {
    let mut bait_id = None;
    for &index in methods {
        let method = catalog.methods().get(index).ok_or_else(|| {
            ConfigError::new(
                settings.resources_field(),
                "method-index",
                "selected gathering method is unavailable",
            )
        })?;
        for amount in known_rows(&method.consumes) {
            if let Some(existing) = bait_id {
                if existing != amount.item {
                    return Err(ConfigError::new(
                        settings.resources_field(),
                        "unsupported-supplies",
                        "one Gatherer method cannot provision multiple consumed items",
                    ));
                }
            } else {
                bait_id = Some(amount.item);
            }
        }
    }
    bait_id
        .map(|id| selected_item(selected, id, settings.resources_field(), "consumed item"))
        .transpose()
}

fn selected_item(
    selected: &SelectedGameData,
    id: i32,
    field: &str,
    description: &str,
) -> Result<SupplyItemFact, ConfigError> {
    let name = selected
        .item_by_id(id)
        .and_then(|item| item.name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            ConfigError::new(
                field,
                "item-unavailable",
                format!("selected {description} id {id} has no object name"),
            )
        })?;
    Ok(SupplyItemFact {
        id,
        name: Arc::from(name),
    })
}

fn resolve_reserve(
    settings: &mut GathererSettings,
    selected: &SelectedGameData,
) -> Result<Option<ReserveSpell>, ConfigError> {
    let requested = settings.reserve_teleport.trim();
    if requested.eq_ignore_ascii_case("off") {
        settings.reserve_teleport = "Off".into();
        return Ok(None);
    }
    let spell = selected
        .teleports()
        .iter()
        .find(|spell| spell.available() && spell.name.eq_ignore_ascii_case(requested))
        .ok_or_else(|| {
            ConfigError::new(
                "reserveTeleport",
                "unknown-spell",
                format!("{requested:?} is not an available selected teleport spell"),
            )
        })?;
    settings.reserve_teleport.clone_from(&spell.name);
    Ok(Some(reserve_spell(spell)?))
}

fn reserve_spell(spell: &TeleportSpell) -> Result<ReserveSpell, ConfigError> {
    let mut runes: [Option<RuneCost>; MAX_RESERVE_RUNES] = std::array::from_fn(|_| None);
    let mut rune_len = 0_usize;
    for rune in &spell.runes {
        let per_cast = (rune.count > 0).then_some(rune.count);
        if rune.id < 0 || rune.name.trim().is_empty() || per_cast.is_none() {
            return Err(ConfigError::new(
                "reserveTeleport",
                "invalid-spell-facts",
                format!("{} has an invalid selected rune cost", spell.name),
            ));
        }
        let per_cast = per_cast.expect("checked above");
        if let Some(existing) = runes[..rune_len]
            .iter_mut()
            .filter_map(Option::as_mut)
            .find(|existing| existing.id == rune.id)
        {
            existing.per_cast = existing.per_cast.checked_add(per_cast).ok_or_else(|| {
                ConfigError::new(
                    "reserveTeleport",
                    "invalid-spell-facts",
                    format!("{} has an overflowing selected rune cost", spell.name),
                )
            })?;
            continue;
        }
        if rune_len == MAX_RESERVE_RUNES {
            return Err(ConfigError::new(
                "reserveTeleport",
                "unsupported-spell",
                format!("{} requires more than three rune types", spell.name),
            ));
        }
        runes[rune_len] = Some(RuneCost {
            id: rune.id,
            name: Arc::from(rune.name.as_str()),
            per_cast,
        });
        rune_len += 1;
    }
    if rune_len == 0 {
        return Err(ConfigError::new(
            "reserveTeleport",
            "invalid-spell-facts",
            format!("{} has no selected rune costs", spell.name),
        ));
    }
    Ok(ReserveSpell {
        name: Arc::from(spell.name.as_str()),
        runes,
        rune_len: rune_len as u8,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolChoice {
    pub id: i32,
    pub worn: bool,
}

/// Return the best-first supported tool that is already held or worn.
/// Wield eligibility is deliberately separate: an unwieldable higher-tier axe
/// in the pack is still the selected and protected tool.
pub fn best_tool(
    prepared: &Prepared,
    stats: &[StatView],
    inventory: &[ItemView],
    equipment: &[ItemView],
) -> Option<ToolChoice> {
    match prepared.settings.skill_kind() {
        Skill::Woodcutting => best_static_tool(
            api::gather_tools::AXES,
            &prepared.selected,
            stats,
            inventory,
            equipment,
        ),
        Skill::Mining => best_static_tool(
            api::gather_tools::PICKAXES,
            &prepared.selected,
            stats,
            inventory,
            equipment,
        ),
        Skill::Fishing => {
            for &index in prepared.methods.iter() {
                let Some(method) = prepared.catalog.methods().get(index) else {
                    continue;
                };
                for tool in known_rows(&method.tools) {
                    if tool.use_gate.is_some_and(|gate| !meets_gate(stats, gate)) {
                        continue;
                    }
                    let worn = equipment
                        .iter()
                        .any(|row| row.def.id == tool.item && row.count > 0);
                    let held = inventory
                        .iter()
                        .any(|row| row.def.id == tool.item && row.count > 0);
                    if worn || held {
                        return Some(ToolChoice {
                            id: tool.item,
                            worn,
                        });
                    }
                }
            }
            None
        }
    }
}

fn best_static_tool(
    candidates: &[api::gather_tools::GatherTool],
    selected: &SelectedGameData,
    stats: &[StatView],
    inventory: &[ItemView],
    equipment: &[ItemView],
) -> Option<ToolChoice> {
    for candidate in candidates {
        if candidate.use_level.is_some_and(|level| {
            !candidate.use_skill.is_some_and(|skill| {
                stats
                    .iter()
                    .any(|stat| stat.name.eq_ignore_ascii_case(skill) && stat.base >= level)
            })
        }) {
            continue;
        }
        let Some(item) = selected.item_by_alias(candidate.alias) else {
            continue;
        };
        let worn = equipment
            .iter()
            .any(|row| row.def.id == item.id && row.count > 0);
        let held = inventory
            .iter()
            .any(|row| row.def.id == item.id && row.count > 0);
        if worn || held {
            return Some(ToolChoice { id: item.id, worn });
        }
    }
    None
}

/// Test whether a held tool may be worn; this never decides which tool wins.
pub fn can_wield(
    prepared: &Prepared,
    tool_id: i32,
    inventory: &[ItemView],
    stats: &[StatView],
) -> bool {
    if !inventory
        .iter()
        .any(|row| row.def.id == tool_id && row.count > 0)
    {
        return false;
    }
    match prepared.settings.skill_kind() {
        Skill::Woodcutting => {
            static_wield_gate(api::gather_tools::AXES, &prepared.selected, tool_id, stats)
        }
        Skill::Mining => static_wield_gate(
            api::gather_tools::PICKAXES,
            &prepared.selected,
            tool_id,
            stats,
        ),
        Skill::Fishing => false,
    }
}

fn static_wield_gate(
    candidates: &[api::gather_tools::GatherTool],
    selected: &SelectedGameData,
    tool_id: i32,
    stats: &[StatView],
) -> bool {
    let Some(candidate) = candidates.iter().find(|candidate| {
        selected
            .item_by_alias(candidate.alias)
            .is_some_and(|item| item.id == tool_id)
    }) else {
        return false;
    };
    stats.iter().any(|stat| {
        stat.name.eq_ignore_ascii_case("attack") && stat.base >= candidate.wield_attack.unwrap_or(1)
    })
}

fn meets_gate(stats: &[StatView], gate: SkillMinimum) -> bool {
    stats
        .iter()
        .any(|stat| stat.index == i32::from(gate.skill) && stat.base >= i32::from(gate.level))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectedIds {
    ids: [i32; MAX_PROTECTED_IDS],
    len: u8,
}

impl ProtectedIds {
    fn new() -> Self {
        Self {
            ids: [-1; MAX_PROTECTED_IDS],
            len: 0,
        }
    }

    fn push(&mut self, id: i32) {
        if id < 0 || self.as_slice().contains(&id) {
            return;
        }
        assert!(usize::from(self.len) < MAX_PROTECTED_IDS);
        self.ids[usize::from(self.len)] = id;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[i32] {
        &self.ids[..usize::from(self.len)]
    }
}

/// Rebuild the small per-boundary keep set from the current selected tool and
/// prepared supply facts. Alternate tools are not protected as carried gear.
pub fn protected_ids(prepared: &Prepared, tool_id: i32) -> ProtectedIds {
    let mut ids = ProtectedIds::new();
    ids.push(tool_id);
    if let Some(bait) = &prepared.supply.bait {
        ids.push(bait.id);
    }
    if let Some(food) = &prepared.supply.food {
        ids.push(food.id);
    }
    ids.push(COINS_ID);
    if prepared.settings.reserve_casts > 0 {
        if let Some(reserve) = &prepared.supply.reserve {
            for rune in reserve.runes() {
                ids.push(rune.id);
            }
        }
    }
    ids
}

/// Return every observed inventory item ID with a positive count except
/// applicable gathering tools and prepared supplies. Incidental non-products
/// are banked too, without special-casing individual drops.
pub fn bank_deposit_ids(prepared: &Prepared, tool_id: i32, inventory: &[ItemView]) -> Arc<[i32]> {
    let protected = protected_ids(prepared, tool_id);
    let mut ids = Vec::with_capacity(inventory.len());
    for item in inventory {
        let id = item.def.id;
        let gathering_tool = prepared.methods.iter().any(|&index| {
            known_rows(&prepared.catalog.methods()[index].tools)
                .iter()
                .any(|tool| tool.item == id)
        });
        if id >= 0
            && item.count > 0
            && !protected.as_slice().contains(&id)
            && !gathering_tool
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    Arc::from(ids)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyItem {
    pub id: i32,
    pub name: Arc<str>,
    /// Exact final inventory count to reach, not an amount to add.
    pub target: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyPlan {
    items: [Option<SupplyItem>; MAX_PLAN_ITEMS],
    len: u8,
    missing: Option<Arc<str>>,
}

impl Default for SupplyPlan {
    fn default() -> Self {
        Self {
            items: std::array::from_fn(|_| None),
            len: 0,
            missing: None,
        }
    }
}

impl SupplyPlan {
    /// Return the first gating item that has no usable stock in inventory or
    /// the loaded bank. Executable withdrawals remain in the plan.
    pub fn missing(&self) -> Option<&Arc<str>> {
        self.missing.as_ref()
    }

    fn note_missing(&mut self, missing: Arc<str>) {
        if self.missing.is_none() {
            self.missing = Some(missing);
        }
    }

    /// Coins are deliberately absent from trip admission: they top up only
    /// during a trip already required for another reason.
    pub fn due(
        prepared: &Prepared,
        stats: &[StatView],
        inventory: &[ItemView],
        equipment: &[ItemView],
    ) -> bool {
        best_tool(prepared, stats, inventory, equipment).is_none()
            || supplies_due(&prepared.supply, &prepared.settings, inventory)
    }

    /// Build the entire immutable withdrawal batch from one loaded-bank view.
    /// `None` means the bank observation is not yet available and is never
    /// interpreted as an empty bank.
    pub fn from_loaded_bank(
        prepared: &Prepared,
        stats: &[StatView],
        inventory: &[ItemView],
        equipment: &[ItemView],
        bank: Option<&[ItemView]>,
    ) -> SupplyPlanResult {
        let Some(bank) = bank else {
            return SupplyPlanResult::Pending;
        };
        let mut plan = Self::default();
        if best_tool(prepared, stats, inventory, equipment).is_none() {
            if let Some(tool) = best_tool(prepared, stats, bank, &[]) {
                if let Some(name) = tool_name(prepared, tool.id) {
                    let fact = SupplyItemFact { id: tool.id, name };
                    top_up(&mut plan, &fact, 1, 1, inventory, bank);
                } else {
                    plan.note_missing(Arc::from("gathering tool"));
                }
            } else {
                plan.note_missing(preferred_tool_name(prepared, stats));
            }
        }
        if let Some(bait) = &prepared.supply.bait {
            top_up(
                &mut plan,
                bait,
                prepared.settings.bait_target,
                1,
                inventory,
                bank,
            );
        }
        if let Some(food) = &prepared.supply.food {
            top_up(
                &mut plan,
                food,
                prepared.settings.food_target,
                if prepared.settings.food_target > 0 {
                    1
                } else {
                    0
                },
                inventory,
                bank,
            );
        }
        if prepared.settings.coin_target > 0 {
            if let Some(name) = prepared.supply.coin_name.as_ref() {
                let coins = SupplyItemFact {
                    id: COINS_ID,
                    name: Arc::clone(name),
                };
                top_up(
                    &mut plan,
                    &coins,
                    prepared.settings.coin_target,
                    0,
                    inventory,
                    bank,
                );
            }
        }
        if prepared.settings.reserve_casts > 0 {
            if let Some(reserve) = &prepared.supply.reserve {
                if !can_afford_cast(reserve, inventory) {
                    for rune in reserve.runes() {
                        let Some(target) =
                            rune.per_cast.checked_mul(prepared.settings.reserve_casts)
                        else {
                            plan.note_missing(Arc::clone(&rune.name));
                            continue;
                        };
                        let fact = SupplyItemFact {
                            id: rune.id,
                            name: Arc::clone(&rune.name),
                        };
                        top_up(&mut plan, &fact, target, rune.per_cast, inventory, bank);
                    }
                }
            }
        }
        if plan.len == 0 {
            if let Some(missing) = &plan.missing {
                return SupplyPlanResult::Missing(Arc::clone(missing));
            }
        }
        SupplyPlanResult::Ready(plan)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SupplyItem> {
        (0..usize::from(self.len))
            .map(|index| self.items[index].as_ref().expect("dense supply plan"))
    }

    pub fn to_withdrawals(&self) -> Arc<[crate::bank::Withdrawal]> {
        self.iter()
            .map(|item| crate::bank::Withdrawal {
                id: item.id,
                name: Arc::clone(&item.name),
                target: item.target,
            })
            .collect()
    }

    fn push(&mut self, item: SupplyItem) {
        if let Some(existing) = self.items[..usize::from(self.len)]
            .iter_mut()
            .filter_map(Option::as_mut)
            .find(|existing| existing.id == item.id)
        {
            existing.target = existing.target.max(item.target);
            return;
        }
        assert!(usize::from(self.len) < MAX_PLAN_ITEMS);
        self.items[usize::from(self.len)] = Some(item);
        self.len += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupplyPlanResult {
    Pending,
    Ready(SupplyPlan),
    Missing(Arc<str>),
}

fn supplies_due(
    supply: &PreparedSupply,
    settings: &GathererSettings,
    inventory: &[ItemView],
) -> bool {
    supply
        .bait
        .as_ref()
        .is_some_and(|bait| item_count(inventory, bait.id) == 0)
        || (settings.food_target > 0
            && supply
                .food
                .as_ref()
                .is_some_and(|food| item_count(inventory, food.id) == 0))
        || (settings.reserve_casts > 0
            && supply
                .reserve
                .as_ref()
                .is_some_and(|reserve| !can_afford_cast(reserve, inventory)))
}

fn can_afford_cast(reserve: &ReserveSpell, inventory: &[ItemView]) -> bool {
    let mut has_cost = false;
    for rune in reserve.runes() {
        has_cost = true;
        if item_count(inventory, rune.id) < rune.per_cast {
            return false;
        }
    }
    has_cost
}

fn top_up(
    plan: &mut SupplyPlan,
    item: &SupplyItemFact,
    target: i32,
    minimum_usable: i32,
    inventory: &[ItemView],
    bank: &[ItemView],
) {
    let current = item_count(inventory, item.id);
    if current >= target && current >= minimum_usable {
        return;
    }
    let available = current.saturating_add(item_count(bank, item.id));
    let target = target.min(available);
    if target > current {
        plan.push(SupplyItem {
            id: item.id,
            name: Arc::clone(&item.name),
            target,
        });
    }
    if available < minimum_usable {
        plan.note_missing(Arc::clone(&item.name));
    }
}

fn item_count(items: &[ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id)
        .fold(0_i32, |total, item| total.saturating_add(item.count.max(0)))
}

fn tool_name(prepared: &Prepared, id: i32) -> Option<Arc<str>> {
    prepared
        .selected
        .item_by_id(id)
        .and_then(|item| item.name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .map(Arc::from)
        .or_else(|| {
            static_tool_name(&prepared.selected, prepared.settings.skill_kind(), id).map(Arc::from)
        })
}

fn preferred_tool_name(prepared: &Prepared, stats: &[StatView]) -> Arc<str> {
    match prepared.settings.skill_kind() {
        Skill::Woodcutting => preferred_static_tool(api::gather_tools::AXES, stats),
        Skill::Mining => preferred_static_tool(api::gather_tools::PICKAXES, stats),
        Skill::Fishing => prepared
            .methods
            .iter()
            .filter_map(|&index| prepared.catalog.methods().get(index))
            .flat_map(|method| known_rows(&method.tools))
            .find(|tool| tool.use_gate.is_none_or(|gate| meets_gate(stats, gate)))
            .and_then(|tool| tool_name(prepared, tool.item))
            .unwrap_or_else(|| Arc::from("fishing tool")),
    }
}

fn preferred_static_tool(
    candidates: &[api::gather_tools::GatherTool],
    stats: &[StatView],
) -> Arc<str> {
    candidates
        .iter()
        .find(|candidate| {
            candidate.use_level.is_none_or(|level| {
                candidate.use_skill.is_some_and(|skill| {
                    stats
                        .iter()
                        .any(|stat| stat.name.eq_ignore_ascii_case(skill) && stat.base >= level)
                })
            })
        })
        .map(|candidate| Arc::from(candidate.name))
        .unwrap_or_else(|| Arc::from("gathering tool"))
}

fn static_tool_name(selected: &SelectedGameData, skill: Skill, id: i32) -> Option<&'static str> {
    let rows = match skill {
        Skill::Woodcutting => api::gather_tools::AXES,
        Skill::Mining => api::gather_tools::PICKAXES,
        Skill::Fishing => return None,
    };
    rows.iter()
        .find(|candidate| {
            selected
                .item_by_alias(candidate.alias)
                .is_some_and(|item| item.id == id)
        })
        .map(|candidate| candidate.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::obj_names::ItemDefView;
    use api::snapshot::{ItemActionFamily, ItemContainer};
    use serde_json::json;

    fn prepare(settings: crate::native::SettingsBag) -> Arc<Prepared> {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(settings),
                selected,
                Arc::default(),
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        Arc::clone(config.get::<Arc<Prepared>>().unwrap())
    }

    fn item(id: i32, name: &str, count: i32, container: ItemContainer) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some(name.into()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: -1,
        }
    }

    fn stats(prepared: &Prepared) -> Vec<StatView> {
        let mut stats = vec![
            StatView {
                index: 0,
                name: "attack".into(),
                effective: 1,
                base: 1,
                xp: 0,
                used: true,
            },
            StatView {
                index: 8,
                name: "woodcutting".into(),
                effective: 99,
                base: 99,
                xp: 0,
                used: true,
            },
            StatView {
                index: 10,
                name: "fishing".into(),
                effective: 99,
                base: 99,
                xp: 0,
                used: true,
            },
            StatView {
                index: 14,
                name: "mining".into(),
                effective: 99,
                base: 99,
                xp: 0,
                used: true,
            },
        ];
        for &index in prepared.methods.iter() {
            let Some(method) = prepared.catalog.methods().get(index) else {
                continue;
            };
            for tool in known_rows(&method.tools) {
                for gate in [tool.use_gate, tool.wield_gate].into_iter().flatten() {
                    let index = i32::from(gate.skill);
                    if let Some(stat) = stats.iter_mut().find(|stat| stat.index == index) {
                        stat.base = stat.base.max(i32::from(gate.level));
                    } else {
                        stats.push(StatView {
                            index,
                            name: String::new(),
                            effective: i32::from(gate.level),
                            base: i32::from(gate.level),
                            xp: 0,
                            used: true,
                        });
                    }
                }
            }
        }
        stats
    }

    fn known_tool(prepared: &Prepared) -> (i32, String) {
        let method = prepared.catalog.methods().get(prepared.methods[0]).unwrap();
        let id = known_rows(&method.tools).first().unwrap().item;
        let name = prepared
            .selected
            .item_by_id(id)
            .and_then(|item| item.name.clone())
            .unwrap();
        (id, name)
    }

    #[test]
    fn partial_supplies_do_not_admit_a_trip_and_loaded_bank_plan_targets_final_counts() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.freshfish.op1"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Raw trout"));
        settings.insert("foodTarget".into(), json!(5));
        settings.insert("coinTarget".into(), json!(500));
        let prepared = prepare(settings);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let food = prepared.supply.food.as_ref().unwrap();
        let coins = prepared.supply.coin_name.as_deref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = vec![
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
            item(food.id, &food.name, 2, ItemContainer::Inventory),
            item(COINS_ID, coins, 100, ItemContainer::Inventory),
        ];
        let stats = stats(&prepared);
        let equipment = [];
        assert!(!SupplyPlan::due(&prepared, &stats, &held, &equipment));

        let bank = vec![
            item(bait.id, &bait.name, 7, ItemContainer::Bank),
            item(food.id, &food.name, 3, ItemContainer::Bank),
            item(COINS_ID, coins, 400, ItemContainer::Bank),
        ];
        assert_eq!(
            SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, None),
            SupplyPlanResult::Pending
        );
        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&bank)) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("expected a complete bank plan, got {other:?}"),
            };
        assert_eq!(
            plan.iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(bait.id, 10), (food.id, 5), (COINS_ID, 500)]
        );
        assert_eq!(
            plan.to_withdrawals()
                .iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(bait.id, 10), (food.id, 5), (COINS_ID, 500)]
        );

        let mut empty_bait = held.clone();
        empty_bait[1].count = 0;
        assert!(SupplyPlan::due(&prepared, &stats, &empty_bait, &equipment));
        let bank_without_bait = &bank[1..];
        let partial = match SupplyPlan::from_loaded_bank(
            &prepared,
            &stats,
            &held,
            &equipment,
            Some(bank_without_bait),
        ) {
            SupplyPlanResult::Ready(plan) => plan,
            other => panic!("above-zero bait must not gate the loaded-bank plan: {other:?}"),
        };
        assert_eq!(partial.missing(), None);
        assert_eq!(
            partial
                .iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(food.id, 5), (COINS_ID, 500)]
        );
    }

    #[test]
    fn current_and_partial_bank_stock_limits_the_supply_target() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.freshfish.op1"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Raw trout"));
        settings.insert("foodTarget".into(), json!(5));
        let prepared = prepare(settings);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let food = prepared.supply.food.as_ref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = vec![
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
        ];
        let bank = [item(bait.id, &bait.name, 2, ItemContainer::Bank)];
        let stats = stats(&prepared);
        let equipment = [];

        assert!(SupplyPlan::due(&prepared, &stats, &held, &equipment));
        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&bank)) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("partial usable bait stock must remain available: {other:?}"),
            };
        assert!(plan
            .missing()
            .is_some_and(|missing| missing.as_ref() == food.name.as_ref()));
        assert_eq!(
            plan.iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(bait.id, 5)]
        );
    }

    #[test]
    fn missing_gating_supply_keeps_tool_withdrawal_and_does_not_gate_on_empty_coins() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.freshfish.op1"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Raw trout"));
        settings.insert("foodTarget".into(), json!(5));
        settings.insert("coinTarget".into(), json!(500));
        let prepared = prepare(settings);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = [];
        let bank = [item(tool_id, &tool_name, 1, ItemContainer::Bank)];
        let stats = stats(&prepared);
        let equipment = [];
        assert!(SupplyPlan::due(&prepared, &stats, &held, &equipment));

        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&bank)) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("available tool withdrawal must precede a missing gate: {other:?}"),
            };
        assert!(plan
            .missing()
            .is_some_and(|missing| missing.as_ref() == bait.name.as_ref()));
        assert_eq!(
            plan.to_withdrawals()
                .iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(tool_id, 1)]
        );
    }

    #[test]
    fn zero_optional_coin_target_neither_admits_nor_gates() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("coinTarget".into(), json!(0));
        let prepared = prepare(settings);
        let held = [item(1351, "Bronze axe", 1, ItemContainer::Inventory)];
        let stats = stats(&prepared);
        let equipment = [];

        assert!(!SupplyPlan::due(&prepared, &stats, &held, &equipment));
        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&[])) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("zero-target optional coins must not gate: {other:?}"),
            };
        assert_eq!(plan.missing(), None);
        assert_eq!(plan.iter().count(), 0);
    }

    #[test]
    fn coins_top_up_only_when_an_unrelated_supply_trip_is_already_due() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("coinTarget".into(), json!(500));
        let prepared = prepare(settings);
        let held = vec![
            item(1351, "Bronze axe", 1, ItemContainer::Inventory),
            item(
                COINS_ID,
                prepared.supply.coin_name.as_deref().unwrap(),
                100,
                ItemContainer::Inventory,
            ),
        ];
        let stats = stats(&prepared);
        let equipment = [];
        assert!(!SupplyPlan::due(&prepared, &stats, &held, &equipment));
        let bank = [item(
            COINS_ID,
            prepared.supply.coin_name.as_deref().unwrap(),
            400,
            ItemContainer::Bank,
        )];
        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&bank)) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("expected coin top-up plan, got {other:?}"),
            };
        assert_eq!(
            plan.iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(COINS_ID, 500)]
        );
    }

    #[test]
    fn reserve_latches_all_runes_only_below_one_cast_and_protects_the_batch() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let spell = selected
            .teleports()
            .iter()
            .find(|spell| {
                spell.available()
                    && !spell.runes.is_empty()
                    && spell
                        .runes
                        .iter()
                        .all(|rune| rune.id >= 0 && !rune.name.trim().is_empty() && rune.count > 0)
            })
            .unwrap()
            .clone();
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("reserveTeleport".into(), json!(spell.name));
        settings.insert("reserveCasts".into(), json!(5));
        let prepared = prepare(settings);
        let reserve = prepared.supply.reserve.as_ref().unwrap();
        let rune_rows: Vec<_> = reserve.runes().cloned().collect();
        let mut held = vec![item(1359, "Rune axe", 1, ItemContainer::Inventory)];
        held.extend(
            rune_rows
                .iter()
                .map(|rune| item(rune.id, &rune.name, rune.per_cast, ItemContainer::Inventory)),
        );
        let bank: Vec<_> = rune_rows
            .iter()
            .map(|rune| item(rune.id, &rune.name, 10_000, ItemContainer::Bank))
            .collect();
        let stats = stats(&prepared);
        let equipment = [];
        assert!(!SupplyPlan::due(&prepared, &stats, &held, &equipment));
        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&bank)) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("expected ready rune plan, got {other:?}"),
            };
        assert_eq!(plan.iter().count(), 0);

        let first_rune = rune_rows[0].id;
        held.iter_mut()
            .find(|item| item.def.id == first_rune)
            .unwrap()
            .count -= 1;
        assert!(SupplyPlan::due(&prepared, &stats, &held, &equipment));
        let plan =
            match SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&bank)) {
                SupplyPlanResult::Ready(plan) => plan,
                other => panic!("expected complete rune refill batch, got {other:?}"),
            };
        assert_eq!(plan.iter().count(), rune_rows.len());
        for rune in &rune_rows {
            assert_eq!(
                plan.iter().find(|item| item.id == rune.id).unwrap().target,
                rune.per_cast * 5
            );
        }

        let protected = protected_ids(&prepared, 1359);
        assert!(protected.as_slice().contains(&1359));
        assert!(protected.as_slice().contains(&COINS_ID));
        assert!(rune_rows
            .iter()
            .all(|rune| protected.as_slice().contains(&rune.id)));
        assert!(protected.as_slice().len() <= MAX_PROTECTED_IDS);
    }

    #[test]
    fn bank_deposit_ids_include_incidental_items_and_protect_current_supplies() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let spell = selected
            .teleports()
            .iter()
            .find(|spell| {
                spell.available()
                    && !spell.runes.is_empty()
                    && spell
                        .runes
                        .iter()
                        .all(|rune| rune.id >= 0 && !rune.name.trim().is_empty() && rune.count > 0)
            })
            .unwrap()
            .clone();
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.freshfish.op1"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Raw trout"));
        settings.insert("foodTarget".into(), json!(5));
        settings.insert("coinTarget".into(), json!(500));
        settings.insert("reserveTeleport".into(), json!(spell.name));
        settings.insert("reserveCasts".into(), json!(5));
        let prepared = prepare(settings);
        let (tool_id, tool_name) = known_tool(&prepared);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let food = prepared.supply.food.as_ref().unwrap();
        let coin_name = prepared.supply.coin_name.as_deref().unwrap();
        let reserve = prepared.supply.reserve.as_ref().unwrap();
        let protected = protected_ids(&prepared, tool_id);
        let product_id = prepared
            .products
            .iter()
            .copied()
            .find(|id| !protected.as_slice().contains(id))
            .expect("the selected fishing method has an unprotected fish product");
        let product_name = prepared
            .selected
            .item_by_id(product_id)
            .and_then(|item| item.name.as_deref())
            .unwrap();
        assert!(!prepared.products.contains(&405));

        let mut inventory = vec![
            item(product_id, product_name, 1, ItemContainer::Inventory),
            item(405, "Fishing casket", 1, ItemContainer::Inventory),
            item(product_id, product_name, 2, ItemContainer::Inventory),
            item(405, "Fishing casket", 1, ItemContainer::Inventory),
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
            item(food.id, &food.name, 2, ItemContainer::Inventory),
            item(COINS_ID, coin_name, 100, ItemContainer::Inventory),
        ];
        inventory.extend(
            reserve
                .runes()
                .map(|rune| item(rune.id, &rune.name, rune.per_cast, ItemContainer::Inventory)),
        );
        inventory.push(item(0, "Dwarf remains", 1, ItemContainer::Inventory));
        inventory.push(item(406, "Empty row", 0, ItemContainer::Inventory));
        inventory.push(item(-1, "Unknown item", 1, ItemContainer::Inventory));

        let deposit_ids = bank_deposit_ids(&prepared, tool_id, &inventory);
        assert_eq!(deposit_ids.as_ref(), &[product_id, 405, 0]);
        for id in [tool_id, bait.id, food.id, COINS_ID] {
            assert!(protected.as_slice().contains(&id));
            assert!(!deposit_ids.contains(&id));
        }
        for rune in reserve.runes() {
            assert!(protected.as_slice().contains(&rune.id));
            assert!(!deposit_ids.contains(&rune.id));
        }
    }

    #[test]
    fn bank_deposit_keeps_alternate_content_defined_gathering_tools() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let tools = known_rows(&prepared.catalog.methods()[prepared.methods[0]].tools);
        assert!(
            tools.len() > 1,
            "woodcutting has multiple applicable tool tiers"
        );
        let mut inventory: Vec<_> = tools
            .iter()
            .map(|tool| item(tool.item, "Gathering tool", 1, ItemContainer::Inventory))
            .collect();
        inventory.push(item(405, "Casket", 1, ItemContainer::Inventory));
        assert_eq!(
            bank_deposit_ids(&prepared, tools[0].item, &inventory).as_ref(),
            &[405]
        );
    }

    #[test]
    fn reserve_gate_requires_one_complete_cast_from_combined_stock() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let spell = selected
            .teleports()
            .iter()
            .find(|spell| {
                spell.available()
                    && spell.runes.iter().any(|rune| rune.count > 1)
                    && spell
                        .runes
                        .iter()
                        .all(|rune| rune.id >= 0 && !rune.name.trim().is_empty() && rune.count > 0)
            })
            .unwrap()
            .clone();
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("reserveTeleport".into(), json!(spell.name));
        settings.insert("reserveCasts".into(), json!(5));
        let prepared = prepare(settings);
        let reserve = prepared.supply.reserve.as_ref().unwrap();
        let rune = reserve.runes().find(|rune| rune.per_cast > 1).unwrap();
        let mut held = vec![item(1351, "Bronze axe", 1, ItemContainer::Inventory)];
        held.extend(reserve.runes().map(|other| {
            let count = if other.id == rune.id {
                0
            } else {
                other.per_cast
            };
            item(other.id, &other.name, count, ItemContainer::Inventory)
        }));
        let stats = stats(&prepared);
        let equipment = [];
        let partial_bank = [item(
            rune.id,
            &rune.name,
            rune.per_cast - 1,
            ItemContainer::Bank,
        )];

        assert!(SupplyPlan::due(&prepared, &stats, &held, &equipment));
        let partial = match SupplyPlan::from_loaded_bank(
            &prepared,
            &stats,
            &held,
            &equipment,
            Some(&partial_bank),
        ) {
            SupplyPlanResult::Ready(plan) => plan,
            other => panic!("fractional rune stock should be withdrawn before failure: {other:?}"),
        };
        assert!(partial
            .missing()
            .is_some_and(|missing| missing.as_ref() == rune.name.as_ref()));
        assert_eq!(
            partial
                .iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(rune.id, rune.per_cast - 1)]
        );
        assert!(matches!(
            SupplyPlan::from_loaded_bank(&prepared, &stats, &held, &equipment, Some(&[])),
            SupplyPlanResult::Missing(missing) if missing.as_ref() == rune.name.as_ref()
        ));
        held.iter_mut()
            .find(|item| item.def.id == rune.id)
            .unwrap()
            .count = rune.per_cast - 1;

        let exactly_one_cast = [item(rune.id, &rune.name, 1, ItemContainer::Bank)];
        let usable = match SupplyPlan::from_loaded_bank(
            &prepared,
            &stats,
            &held,
            &equipment,
            Some(&exactly_one_cast),
        ) {
            SupplyPlanResult::Ready(plan) => plan,
            other => panic!("one full cast of combined runes is usable: {other:?}"),
        };
        assert_eq!(usable.missing(), None);
        assert_eq!(
            usable
                .iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(rune.id, rune.per_cast)]
        );
    }

    #[test]
    fn axe_and_pickaxe_handles_are_not_usable_gathering_tools() {
        for (skill, handle_id, handle_name) in [
            ("Woodcutting", 492, "Axe handle"),
            ("Mining", 466, "Pickaxe handle"),
        ] {
            let mut settings = crate::native::SettingsBag::new();
            settings.insert("skill".into(), json!(skill));
            let prepared = prepare(settings);
            let stats = stats(&prepared);
            let handle = [item(handle_id, handle_name, 1, ItemContainer::Inventory)];
            let (tool_id, tool_name) = known_tool(&prepared);

            assert_eq!(best_tool(&prepared, &stats, &handle, &[]), None);
            assert!(SupplyPlan::due(&prepared, &stats, &handle, &[]));
            let bank = [item(tool_id, &tool_name, 1, ItemContainer::Bank)];
            let plan =
                match SupplyPlan::from_loaded_bank(&prepared, &stats, &handle, &[], Some(&bank)) {
                    SupplyPlanResult::Ready(plan) => plan,
                    other => panic!("loaded real tool should replace {handle_name}: {other:?}"),
                };
            assert_eq!(plan.missing(), None);
            assert_eq!(
                plan.iter()
                    .map(|item| (item.id, item.target))
                    .collect::<Vec<_>>(),
                vec![(tool_id, 1)]
            );
        }
    }

    #[test]
    fn best_tool_is_selected_before_wieldability_and_bank_choice_is_best_first() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let mut stats = stats(&prepared);
        let attack = stats.iter_mut().find(|stat| stat.index == 0).unwrap();
        attack.base = 1;
        attack.effective = 1;
        let held = vec![
            item(1351, "Bronze axe", 1, ItemContainer::Inventory),
            item(1359, "Rune axe", 1, ItemContainer::Inventory),
        ];
        let choice = best_tool(&prepared, &stats, &held, &[]).unwrap();
        assert_eq!(choice.id, 1359);
        assert!(!choice.worn);
        assert!(!can_wield(&prepared, choice.id, &held, &stats));
        let protected = protected_ids(&prepared, choice.id);
        assert!(protected.as_slice().contains(&1359));
        assert!(!protected.as_slice().contains(&1351));

        let bank = vec![
            item(1351, "Bronze axe", 1, ItemContainer::Bank),
            item(1359, "Rune axe", 1, ItemContainer::Bank),
        ];
        let plan = match SupplyPlan::from_loaded_bank(&prepared, &stats, &[], &[], Some(&bank)) {
            SupplyPlanResult::Ready(plan) => plan,
            other => panic!("expected bank tool plan, got {other:?}"),
        };
        assert_eq!(
            plan.iter()
                .map(|item| (item.id, item.target))
                .collect::<Vec<_>>(),
            vec![(1359, 1)]
        );
    }

    #[test]
    fn every_content_defined_fishing_tool_is_inventory_ready_without_wielding() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        let mut prepared = prepare(settings);
        let catalog = Arc::clone(&prepared.catalog);
        let prepared = Arc::get_mut(&mut prepared).expect("test owns the prepared config");
        let mut tested_tools = std::collections::BTreeSet::new();
        for (index, method) in catalog
            .methods()
            .iter()
            .enumerate()
            .filter(|(_, method)| method.skill == api::gather_methods::GatherSkill::Fishing)
        {
            // Exercise tool/supply semantics even for content methods whose
            // unrelated quest/form facts keep the whole method unselectable.
            prepared.methods = Arc::from([index]);
            prepared.settings.fishing_method = method.id.0.to_string();
            prepared.supply = PreparedSupply::prepare(
                &mut prepared.settings,
                &prepared.selected,
                &catalog,
                &prepared.methods,
            )
            .unwrap();
            let stats = stats(prepared);
            for tool in known_rows(&method.tools) {
                let name = prepared
                    .selected
                    .item_by_id(tool.item)
                    .unwrap()
                    .name
                    .as_deref()
                    .unwrap();
                let mut held = vec![item(tool.item, name, 1, ItemContainer::Inventory)];
                if let Some(bait) = &prepared.supply.bait {
                    held.push(item(bait.id, &bait.name, 1, ItemContainer::Inventory));
                }
                let choice = best_tool(prepared, &stats, &held, &[]).unwrap();
                assert_eq!(choice.id, tool.item, "{}", method.id.0);
                assert!(!choice.worn);
                assert!(!can_wield(prepared, choice.id, &held, &stats));
                assert!(!SupplyPlan::due(prepared, &stats, &held, &[]));
                assert!(protected_ids(prepared, choice.id)
                    .as_slice()
                    .contains(&tool.item));
                assert!(matches!(
                    &SupplyPlan::from_loaded_bank(prepared, &stats, &held, &[], Some(&[])),
                    SupplyPlanResult::Ready(plan) if plan.iter().next().is_none()
                ));

                // A lost tool must still use the same bank stock machinery.
                held.remove(0);
                assert!(SupplyPlan::due(prepared, &stats, &held, &[]));
                let bank = [item(tool.item, name, 1, ItemContainer::Bank)];
                let plan =
                    match SupplyPlan::from_loaded_bank(prepared, &stats, &held, &[], Some(&bank)) {
                        SupplyPlanResult::Ready(plan) => plan,
                        other => panic!(
                            "{} tool stock must admit withdrawal: {other:?}",
                            method.id.0
                        ),
                    };
                assert_eq!(plan.missing(), None);
                assert_eq!(plan.to_withdrawals()[0].id, tool.item);
                assert_eq!(plan.to_withdrawals()[0].target, 1);
                tested_tools.insert(tool.item);
            }
        }
        for alias in [
            "net",
            "big_net",
            "lobster_pot",
            "harpoon",
            "fishing_rod",
            "fly_fishing_rod",
            "oily_fishing_rod",
            "tbwt_karambwan_vessel_loaded_with_karambwanji",
        ] {
            let id = prepared.selected.item_by_alias(alias).unwrap().id;
            assert!(
                tested_tools.contains(&id),
                "missing fishing tool coverage: {alias}"
            );
        }
    }
}
