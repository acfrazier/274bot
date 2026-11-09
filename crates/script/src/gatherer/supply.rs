//! Gatherer supply admission and immutable acquisition needs.
//!
//! The acquisition core owns bank planning and wear. This module prepares
//! selected supply facts, lists needs, and decides when a trip is due.

use super::card::Prepared;
use super::settings::{GathererSettings, Skill};
use crate::acquire::{unbankable_index, Need, NeedKind, Presence, SourceSet, UNBANKABLE_ALIASES};
use crate::native::{ActionError, ConfigError};
use api::game_data::{SelectedGameData, TeleportSpell};
use api::gather_methods::{known_rows, GatherCatalog, GatherMethod};
use api::selected::{Knowledge, SkillMinimum, Truth};
use api::snapshot::{ItemView, StatView};
use api::stock::Stock;
use std::sync::Arc;

const COINS_ID: i32 = 995;
const MAX_RESERVE_RUNES: usize = 3;
const MAX_PROTECTED_IDS: usize = 10 + UNBANKABLE_ALIASES.len();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupplyItemFact {
    pub id: i32,
    pub name: Arc<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickKit {
    pub knife: SupplyItemFact,
    pub logs: SupplyItemFact,
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
    /// Fixed knife and normal-log kit, resolved without method applicability.
    pub tick_kit: Option<TickKit>,
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
        let tick_kit = resolve_tick_kit(settings, selected)?;
        Ok(Self {
            food,
            bait,
            coin_name,
            reserve,
            tick_kit,
        })
    }
}

fn resolve_tick_kit(
    settings: &GathererSettings,
    selected: &SelectedGameData,
) -> Result<Option<TickKit>, ConfigError> {
    if settings.tick_method != "Knife" {
        return Ok(None);
    }
    let resolve = |alias| {
        selected
            .resolve_item_name(alias)
            .and_then(|item| {
                item.name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .map(|name| SupplyItemFact {
                        id: item.id,
                        name: Arc::from(name),
                    })
            })
            .ok_or_else(|| {
                ConfigError::new(
                    "tickMethod",
                    "item-unavailable",
                    format!("the selected cache has no named {alias} item"),
                )
            })
    };
    Ok(Some(TickKit {
        knife: resolve("knife")?,
        logs: resolve("logs")?,
    }))
}

fn resolve_food(
    settings: &mut GathererSettings,
    selected: &SelectedGameData,
) -> Result<Option<SupplyItemFact>, ConfigError> {
    let requested = settings.food.as_str();
    if requested.is_empty() {
        return Ok(None);
    }
    let item = selected
        .resolve_item_name(requested)
        .filter(|item| {
            item.name
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty())
        })
        .ok_or_else(|| {
            ConfigError::new(
                "food",
                "unknown-item",
                format!("food {requested:?} is not a selected item alias or name"),
            )
        })?;
    let name = item.name.as_deref().expect("resolved named item");
    settings.food = name.to_string();
    Ok(Some(SupplyItemFact {
        id: item.id,
        name: Arc::from(name),
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

/// Admission shares supply planning's tool and effective-stat rules. Quest Paths
/// own their selected quest-state gates; ordinary Gatherer revalidates on refusal.
/// A required item counts worn (`Stock::has`); consumed bait comes from the
/// pack (`Stock::holds`).
pub(crate) fn method_ready(
    snapshot: api::snapshot::SnapshotView<'_>,
    method: &GatherMethod,
    quest_owned: bool,
) -> Result<bool, &'static str> {
    let (Some(stats), Some(inventory), Some(equipment), Some(world)) = (
        snapshot.stats(),
        snapshot.inventory(),
        snapshot.equipment(),
        snapshot.world(),
    ) else {
        return Ok(false);
    };
    let stock = snapshot.stock();
    let Some(requirements) = super::settings::method_requirements(method, quest_owned) else {
        return Err("gather requirements are incomplete");
    };
    for requirement in requirements.iter() {
        match requirement.kind {
            api::selected::RequirementKind::Skill(minimum) => {
                if !stats
                    .value
                    .iter()
                    .any(|stat| stat.index == i32::from(minimum.skill))
                {
                    return Ok(false);
                }
                if !meets_gate(stats.value, minimum) {
                    return Err("gather level too low");
                }
            }
            api::selected::RequirementKind::MembersWorld if !world.value.members => {
                return Err("gather requires a members world");
            }
            api::selected::RequirementKind::MembersWorld => {}
            api::selected::RequirementKind::Item(item)
                if stock.has(item.item, amount(item.count)) != Truth::True =>
            {
                return Err("gather required item missing");
            }
            api::selected::RequirementKind::Item(_) => {}
            _ => return Err("gather requirement is not observed"),
        }
    }
    let (Knowledge::Known(tools), Knowledge::Known(consumes)) = (&method.tools, &method.consumes)
    else {
        return Err("gather supplies are incomplete");
    };
    if !tools.is_empty()
        && best_method_tool(method, stats.value, |id| {
            carried_tool(inventory.value, equipment.value, id)
        })
        .is_none()
    {
        return Err("gather usable tool missing");
    }
    if consumes
        .iter()
        .any(|item| stock.holds(item.item, amount(item.count)) != Truth::True)
    {
        return Err("gather bait missing");
    }
    Ok(true)
}

/// A selected item amount as a stock quantity; an amount no `i32` stack can
/// reach is never satisfied.
fn amount(count: u32) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

pub(crate) fn method_tool_candidates<'a>(
    method: &'a GatherMethod,
    stats: &'a [StatView],
) -> impl Iterator<Item = &'a api::gather_methods::ToolUse> + 'a {
    known_rows(&method.tools)
        .iter()
        .filter(move |tool| tool.use_gate.is_none_or(|gate| meets_gate(stats, gate)))
}

/// Whether a positive row of `id` is in `rows`.
fn present(rows: &[ItemView], id: i32) -> bool {
    rows.iter().any(|row| row.def.id == id && row.count > 0)
}

/// `Some(worn)` when the tool `id` is held or worn.
fn carried_tool(inventory: &[ItemView], equipment: &[ItemView], id: i32) -> Option<bool> {
    let worn = present(equipment, id);
    (worn || present(inventory, id)).then_some(worn)
}

/// The best-first usable tool `owned` reports present: `owned(id)` is
/// `Some(worn)` for a tool that is there, `None` otherwise.
fn best_method_tool(
    method: &GatherMethod,
    stats: &[StatView],
    owned: impl Fn(i32) -> Option<bool>,
) -> Option<ToolChoice> {
    method_tool_candidates(method, stats).find_map(|tool| {
        owned(tool.item).map(|worn| ToolChoice {
            id: tool.item,
            worn,
        })
    })
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
    best_tool_where(prepared, stats, |id| carried_tool(inventory, equipment, id))
}

/// [`best_tool`] over any item source: `owned(id)` is `Some(worn)` when the
/// tool is there. The bank planners pass a banked count, never worn.
fn best_tool_where(
    prepared: &Prepared,
    stats: &[StatView],
    owned: impl Fn(i32) -> Option<bool>,
) -> Option<ToolChoice> {
    match prepared.settings.skill_kind() {
        Skill::Woodcutting => {
            best_static_tool(api::gather_tools::AXES, &prepared.selected, stats, &owned)
        }
        Skill::Mining => best_static_tool(
            api::gather_tools::PICKAXES,
            &prepared.selected,
            stats,
            &owned,
        ),
        Skill::Fishing => prepared.methods.iter().find_map(|&index| {
            best_method_tool(prepared.catalog.methods().get(index)?, stats, &owned)
        }),
    }
}

fn best_static_tool(
    candidates: &[api::gather_tools::GatherTool],
    selected: &SelectedGameData,
    stats: &[StatView],
    owned: impl Fn(i32) -> Option<bool>,
) -> Option<ToolChoice> {
    for candidate in candidates {
        if candidate.use_level.is_some_and(|level| {
            !candidate
                .use_skill
                .is_some_and(|skill| meets_named_gate(stats, skill, level))
        }) {
            continue;
        }
        let Some(item) = selected.item_by_alias(candidate.alias) else {
            continue;
        };
        if let Some(worn) = owned(item.id) {
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

/// Content's `stat()` gate uses the current effective level, including boosts/drains.
pub(crate) fn meets_gate(stats: &[StatView], gate: SkillMinimum) -> bool {
    stats
        .iter()
        .any(|stat| stat.index == i32::from(gate.skill) && stat.effective >= i32::from(gate.level))
}

fn meets_named_gate(stats: &[StatView], skill: &str, level: i32) -> bool {
    stats
        .iter()
        .any(|stat| stat.name.eq_ignore_ascii_case(skill) && stat.effective >= level)
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

    /// Bank keeps also include content-refused items actually held this trip.
    pub(super) fn keep_unbankable(&mut self, selected: &SelectedGameData, inventory: &[ItemView]) {
        for item in inventory {
            if item.count > 0 && unbankable_index(selected, item).is_some() {
                self.push(item.def.id);
            }
        }
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
    if let Some(kit) = &prepared.supply.tick_kit {
        ids.push(kit.knife.id);
        ids.push(kit.logs.id);
    }
    ids
}

/// Return every observed positive-count inventory item ID except gathering
/// tools, prepared supplies, held antipoison doses and content-defined
/// unbankable items. Bankable incidental non-products are deposited too. The
/// deposit moves only these ids (`DepositProducts` is `only: Some(products)`,
/// `native_bank.rs` `deposit_spec`), so a held antipoison or super antipoison
/// dose stays in the pack for the next poison.
pub fn bank_deposit_ids(prepared: &Prepared, tool_id: i32, inventory: &[ItemView]) -> Arc<[i32]> {
    let mut protected = protected_ids(prepared, tool_id);
    protected.keep_unbankable(&prepared.selected, inventory);
    let tables = crate::combat::guard::shared_tables_ref();
    let mut ids = Vec::with_capacity(inventory.len());
    for item in inventory {
        let id = item.def.id;
        if id < 0 || item.count <= 0 || protected.as_slice().contains(&id) || ids.contains(&id) {
            continue;
        }
        if tables.is_some_and(|tables| crate::survival::cure::is_antipoison_dose(tables, id)) {
            continue;
        }
        let gathering_tool = prepared.methods.iter().any(|&index| {
            known_rows(&prepared.catalog.methods()[index].tools)
                .iter()
                .any(|tool| tool.item == id)
        });
        if !gathering_tool {
            ids.push(id);
        }
    }
    Arc::from(ids)
}

/// Gatherer supply admission and need production over the acquisition core.
pub struct SupplyPlan;

impl SupplyPlan {
    /// Return whether posted pack and worn pages require an acquisition trip.
    ///
    /// Coins never admit a trip. Bank knowledge does not admit a trip. An
    /// unworn selected tool admits a trip only when its own wield gate passes.
    pub fn due(prepared: &Prepared, stats: &[StatView], stock: Stock<'_>) -> bool {
        let (Some(inventory), Some(equipment)) = (stock.pack, stock.worn) else {
            return false;
        };
        let selected_tool = best_tool(prepared, stats, inventory, equipment);
        selected_tool.is_none()
            || selected_tool
                .is_some_and(|tool| !tool.worn && can_wield(prepared, tool.id, inventory, stats))
            || supplies_due(&prepared.supply, &prepared.settings, stock)
    }

    /// The optional technique kit cannot reach one knife and one log from the
    /// pack and the bank. It never becomes a gating missing item or a trip by
    /// itself. An unknown bank count reads as none.
    pub fn tick_kit_missing(prepared: &Prepared, stock: Stock<'_>) -> bool {
        prepared
            .supply
            .tick_kit
            .as_ref()
            .is_some_and(|kit| !tick_kit_reachable(kit, stock))
    }

    /// List immutable acquisition needs for the current selected supply facts.
    ///
    /// This method does not plan withdrawals, scan bank verdicts, or perform
    /// bank actions. The caller passes the full request to the acquisition core.
    pub fn build(
        prepared: &Prepared,
        stats: &[StatView],
        stock: Stock<'_>,
    ) -> Result<Arc<[crate::acquire::Need]>, crate::native::ActionError> {
        let mut needs = Vec::with_capacity(4 + MAX_RESERVE_RUNES);
        if let Some(tool) = tool_need(prepared, stats, stock)? {
            needs.push(tool);
        }
        if let Some(bait) = &prepared.supply.bait {
            needs.push(Need {
                item: bait.id,
                name: Arc::clone(&bait.name),
                target: prepared.settings.bait_target,
                minimum: 1,
                presence: Presence::Held,
                kind: NeedKind::Required,
                sources: purchase_sources(prepared, SourceSet::PACK | SourceSet::BANK),
                alternatives: Arc::<[i32]>::from([]),
            });
        }
        if prepared.settings.food_target > 0 {
            if let Some(food) = &prepared.supply.food {
                needs.push(Need {
                    item: food.id,
                    name: Arc::clone(&food.name),
                    target: prepared.settings.food_target,
                    minimum: 1,
                    presence: Presence::Held,
                    kind: NeedKind::Required,
                    sources: SourceSet::PACK | SourceSet::BANK,
                    alternatives: Arc::<[i32]>::from([]),
                });
            }
        }
        if prepared.settings.coin_target > 0 {
            if let Some(name) = prepared.supply.coin_name.as_ref() {
                needs.push(Need {
                    item: COINS_ID,
                    name: Arc::clone(name),
                    target: prepared.settings.coin_target,
                    minimum: 0,
                    presence: Presence::Held,
                    kind: NeedKind::Optional,
                    sources: SourceSet::PACK | SourceSet::BANK,
                    alternatives: Arc::<[i32]>::from([]),
                });
            }
        }
        if prepared.settings.reserve_casts > 0 {
            if let Some(reserve) = &prepared.supply.reserve {
                if !can_afford_cast(reserve, stock) {
                    for rune in reserve.runes() {
                        let target = rune
                            .per_cast
                            .checked_mul(prepared.settings.reserve_casts)
                            .ok_or_else(|| {
                                ActionError::Unavailable(Arc::from(format!(
                                    "reserve rune target overflows for {}",
                                    rune.name
                                )))
                            })?;
                        needs.push(Need {
                            item: rune.id,
                            name: Arc::clone(&rune.name),
                            target,
                            minimum: rune.per_cast,
                            presence: Presence::Held,
                            kind: NeedKind::Required,
                            sources: SourceSet::PACK | SourceSet::BANK,
                            alternatives: Arc::<[i32]>::from([]),
                        });
                    }
                }
            }
        }
        if let Some(kit) = &prepared.supply.tick_kit {
            // The technique kit is optional: it is withdrawn only when the held
            // and banked counts can complete it, so a missing half never blocks
            // the run and never leaves a lone Knife or lone Logs in the pack.
            if tick_kit_reachable(kit, stock) {
                for (item, target) in [(&kit.knife, 1), (&kit.logs, 3)] {
                    needs.push(Need {
                        item: item.id,
                        name: Arc::clone(&item.name),
                        target,
                        minimum: 1,
                        presence: Presence::Held,
                        kind: NeedKind::Optional,
                        sources: SourceSet::PACK | SourceSet::BANK,
                        alternatives: Arc::<[i32]>::from([]),
                    });
                }
            }
        }
        Ok(Arc::from(needs))
    }
}

fn tool_need(
    prepared: &Prepared,
    stats: &[StatView],
    stock: Stock<'_>,
) -> Result<Option<Need>, ActionError> {
    let alternatives = usable_tool_ids(prepared, stats);
    let Some(&first) = alternatives.first() else {
        return Ok(None);
    };
    let fallback = alternatives
        .iter()
        .copied()
        .find(|&id| wield_gate_met(prepared, id, stats))
        .unwrap_or(first);
    let carried = match (stock.pack, stock.worn) {
        (Some(inventory), Some(equipment)) => best_tool(prepared, stats, inventory, equipment),
        _ => None,
    };
    let selected = carried.or_else(|| {
        best_tool_where(prepared, stats, |id| {
            stock
                .banked(id)
                .is_some_and(|count| count > 0)
                .then_some(false)
        })
    });
    let item = selected.map_or(fallback, |tool| tool.id);
    let name = prepared
        .selected
        .item_by_id(item)
        .and_then(|item| item.name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .map(Arc::from)
        .ok_or_else(|| {
            ActionError::Unavailable(Arc::from(format!(
                "selected cache has no name for gathering item {item}"
            )))
        })?;
    Ok(Some(Need {
        item,
        name,
        target: 1,
        minimum: 1,
        presence: if wield_gate_met(prepared, item, stats) {
            Presence::Wielded
        } else {
            Presence::Carried
        },
        kind: NeedKind::Required,
        sources: purchase_sources(
            prepared,
            SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
        ),
        alternatives: Arc::from(alternatives),
    }))
}

fn purchase_sources(prepared: &Prepared, carried: SourceSet) -> SourceSet {
    if prepared.settings.buy_tools {
        carried | SourceSet::SHOP
    } else {
        carried
    }
}

fn usable_tool_ids(prepared: &Prepared, stats: &[StatView]) -> Vec<i32> {
    let mut ids = Vec::new();
    match prepared.settings.skill_kind() {
        Skill::Woodcutting => {
            append_static_tool_ids(&mut ids, api::gather_tools::AXES, &prepared.selected, stats)
        }
        Skill::Mining => append_static_tool_ids(
            &mut ids,
            api::gather_tools::PICKAXES,
            &prepared.selected,
            stats,
        ),
        Skill::Fishing => {
            for &index in prepared.methods.iter() {
                let Some(method) = prepared.catalog.methods().get(index) else {
                    continue;
                };
                for tool in method_tool_candidates(method, stats) {
                    if prepared.selected.item_by_id(tool.item).is_some() {
                        push_tool_id(&mut ids, tool.item);
                    }
                }
            }
        }
    }
    ids
}

fn append_static_tool_ids(
    ids: &mut Vec<i32>,
    candidates: &[api::gather_tools::GatherTool],
    selected: &SelectedGameData,
    stats: &[StatView],
) {
    for candidate in candidates {
        if candidate.use_level.is_some_and(|level| {
            !candidate
                .use_skill
                .is_some_and(|skill| meets_named_gate(stats, skill, level))
        }) {
            continue;
        }
        if let Some(item) = selected.item_by_alias(candidate.alias) {
            push_tool_id(ids, item.id);
        }
    }
}

fn push_tool_id(ids: &mut Vec<i32>, id: i32) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

fn supplies_due(supply: &PreparedSupply, settings: &GathererSettings, stock: Stock<'_>) -> bool {
    let lacks = |id| stock.holds(id, 1) == Truth::False;
    supply.bait.as_ref().is_some_and(|bait| lacks(bait.id))
        || (settings.food_target > 0 && supply.food.as_ref().is_some_and(|food| lacks(food.id)))
        || (settings.reserve_casts > 0
            && supply
                .reserve
                .as_ref()
                .is_some_and(|reserve| !can_afford_cast(reserve, stock)))
        || supply
            .tick_kit
            .as_ref()
            .is_some_and(|kit| tick_kit_due(kit, stock))
}

/// One knife and one log are reachable from the pack and a known bank.
fn tick_kit_reachable(kit: &TickKit, stock: Stock<'_>) -> bool {
    let held = |id| stock.held(id).unwrap_or(0).max(0);
    let banked = |id| {
        stock
            .bank
            .filter(|bank| bank.known())
            .and_then(|bank| bank.count(id))
            .unwrap_or(0)
            .max(0)
    };
    held(kit.knife.id).saturating_add(banked(kit.knife.id)) >= 1
        && held(kit.logs.id).saturating_add(banked(kit.logs.id)) >= 1
}

fn tick_kit_due(kit: &TickKit, stock: Stock<'_>) -> bool {
    let (Some(knife), Some(logs)) = (stock.held(kit.knife.id), stock.held(kit.logs.id)) else {
        return false;
    };
    if knife >= 1 && logs >= 1 {
        return false;
    }
    stock.bank.filter(|bank| bank.known()).is_none_or(|bank| {
        (knife >= 1 || bank.count(kit.knife.id).is_some_and(|count| count >= 1))
            && (logs >= 1 || bank.count(kit.logs.id).is_some_and(|count| count >= 1))
    })
}

/// Runes are consumed from the pack; worn runes never pay a cast.
fn can_afford_cast(reserve: &ReserveSpell, stock: Stock<'_>) -> bool {
    let mut has_cost = false;
    for rune in reserve.runes() {
        has_cost = true;
        if stock.holds(rune.id, rune.per_cast) != Truth::True {
            return false;
        }
    }
    has_cost
}

fn wield_gate_met(prepared: &Prepared, tool_id: i32, stats: &[StatView]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acquire::{
        self, AcquirePlan, AcquireRequest, AcquireVerdict, BankChoice, BankStage, CloseBank,
        CoinPolicy, VendorChoice, WorldFacts,
    };
    use api::bank_memory::{BankMemory, ObservedAt, Origin};
    use api::obj_names::ItemDefView;
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::{ItemActionFamily, ItemContainer};
    use serde_json::json;

    const FISHING_ROD: i32 = 307;

    fn prepare(mut settings: crate::native::SettingsBag) -> Arc<Prepared> {
        // These pre-G1 cases exercise bank-only acquisition by design.
        settings.entry("buyTools").or_insert(json!(false));
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(settings),
                selected,
                Arc::default(),
                None,
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

    /// Posted pack and worn pages with no bank memory.
    fn pages<'a>(held: &'a [ItemView], worn: &'a [ItemView]) -> Stock<'a> {
        Stock {
            pack: Some(held),
            capacity: Some(28),
            worn: Some(worn),
            bank: None,
        }
    }

    fn with_bank<'a>(
        held: &'a [ItemView],
        worn: &'a [ItemView],
        bank: &'a BankMemory,
    ) -> Stock<'a> {
        Stock {
            bank: Some(bank),
            ..pages(held, worn)
        }
    }

    fn due(prepared: &Prepared, stats: &[StatView], held: &[ItemView], worn: &[ItemView]) -> bool {
        SupplyPlan::due(prepared, stats, pages(held, worn))
    }

    fn stamp() -> EvidenceStamp {
        EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        }
    }

    fn request(needs: Arc<[Need]>) -> AcquireRequest {
        AcquireRequest {
            needs,
            deposit: None,
            bank: BankChoice {
                bank: None,
                explicit: None,
                required: false,
                preferences: Default::default(),
            },
            stage: BankStage::Planned,
            partial_ok: false,
            close: CloseBank::LeaveOpen,
            vendor: VendorChoice::None,
            coins: CoinPolicy::None,
            walk: crate::native::WalkOptions::default(),
            required_after: stamp(),
        }
    }

    fn core_plan_needs(needs: Arc<[Need]>, stock: Stock<'_>) -> AcquireVerdict {
        let request = request(needs);
        acquire::plan::plan(&request, &stock, WorldFacts::default())
            .unwrap_or_else(|_| panic!("the acquisition core rejected the test request"))
    }

    fn core_plan(prepared: &Prepared, stats: &[StatView], stock: Stock<'_>) -> AcquireVerdict {
        let needs = SupplyPlan::build(prepared, stats, stock)
            .unwrap_or_else(|_| panic!("the supply builder rejected the selected facts"));
        core_plan_needs(needs, stock)
    }

    fn planned(verdict: AcquireVerdict) -> AcquirePlan {
        match verdict {
            AcquireVerdict::Plan(plan) => plan,
            AcquireVerdict::Satisfied => panic!("expected acquisition work"),
            AcquireVerdict::Blocked(block) => {
                panic!("expected acquisition work, got {}", block.message)
            }
        }
    }

    fn targets(plan: &AcquirePlan) -> Vec<(i32, i32)> {
        plan.withdrawals[..plan.withdrawal_count]
            .iter()
            .flatten()
            .map(|item| (item.id, item.target))
            .collect()
    }

    fn tool_need(needs: &[Need]) -> &Need {
        needs
            .iter()
            .find(|need| need.sources.contains(SourceSet::WORN))
            .expect("a usable gathering tool need")
    }

    /// A memory holding rows the way the host fills it.
    fn memory(rows: &[(i32, i32)], origin: Origin) -> BankMemory {
        let mut memory = BankMemory::default();
        match origin {
            Origin::Unknown => {}
            Origin::Hint => memory.load_hint(rows.to_vec(), 1).unwrap(),
            Origin::Session => {
                let bank: Vec<ItemView> = rows
                    .iter()
                    .map(|&(id, count)| item(id, "banked", count, ItemContainer::Bank))
                    .collect();
                memory.observe(
                    &bank,
                    64,
                    ObservedAt {
                        unix_secs: 1,
                        tick: 1,
                        bank_session: 1,
                    },
                );
            }
        }
        memory
    }

    /// Draynor's sardine/herring bait method.
    fn bait_fishing(bait_target: i32) -> Arc<Prepared> {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.saltfish.op3"));
        settings.insert("baitTarget".into(), json!(bait_target));
        prepare(settings)
    }

    #[test]
    fn food_setting_resolves_alias_or_case_insensitive_name() {
        if !api::test_support::game_data_available() {
            return;
        }
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let (item_id, alias, name) = selected
            .items()
            .iter()
            .find_map(|item| {
                let alias = item.alias.as_deref()?;
                let name = item.name.as_deref()?;
                (!alias.eq_ignore_ascii_case(name))
                    .then(|| (item.id, alias.to_string(), name.to_string()))
            })
            .unwrap();
        let mut settings = GathererSettings {
            food: alias,
            ..GathererSettings::default()
        };

        let fact = resolve_food(&mut settings, &selected).unwrap().unwrap();

        assert_eq!(fact.id, item_id);
        assert_eq!(fact.name.as_ref(), name);
        assert_eq!(settings.food, name);

        let mut display_settings = GathererSettings {
            food: name.to_ascii_uppercase(),
            ..GathererSettings::default()
        };
        let by_display = resolve_food(&mut display_settings, &selected)
            .unwrap()
            .unwrap();
        assert_eq!(by_display.id, item_id);
        assert_eq!(by_display.name.as_ref(), name);
        assert_eq!(display_settings.food, name);
    }

    #[test]
    fn tool_use_uses_effective_level_and_wield_uses_base_level() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Mining"));
        let prepared = prepare(settings);
        let held = [item(1275, "Rune pickaxe", 1, ItemContainer::Inventory)];
        let mut mining_stats = stats(&prepared);
        let mining = mining_stats
            .iter_mut()
            .find(|stat| stat.index == 14)
            .unwrap();
        mining.base = 40;
        mining.effective = 41;
        assert_eq!(
            best_tool(&prepared, &mining_stats, &held, &[]).unwrap().id,
            1275
        );

        let mining = mining_stats
            .iter_mut()
            .find(|stat| stat.index == 14)
            .unwrap();
        mining.base = 41;
        mining.effective = 40;
        assert!(best_tool(&prepared, &mining_stats, &held, &[]).is_none());
        assert!(!meets_gate(
            &mining_stats,
            SkillMinimum {
                skill: 14,
                level: 41
            }
        ));

        let prepared = prepare(crate::native::SettingsBag::new());
        let axe = [item(1359, "Rune axe", 1, ItemContainer::Inventory)];
        let mut attack_stats = stats(&prepared);
        {
            let attack = attack_stats
                .iter_mut()
                .find(|stat| stat.index == 0)
                .unwrap();
            attack.base = 39;
            attack.effective = 99;
        }
        assert!(!can_wield(&prepared, 1359, &axe, &attack_stats));
        {
            let attack = attack_stats
                .iter_mut()
                .find(|stat| stat.index == 0)
                .unwrap();
            attack.base = 40;
            attack.effective = 1;
        }
        assert!(can_wield(&prepared, 1359, &axe, &attack_stats));
    }

    #[test]
    fn shop_tool_tiers_exclude_items_beyond_the_mining_use_level() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Mining"));
        settings.insert("buyTools".into(), json!(true));
        let prepared = prepare(settings);
        let mut stats = stats(&prepared);
        let mining = stats.iter_mut().find(|stat| stat.index == 14).unwrap();
        mining.base = 1;
        mining.effective = 1;
        let needs = SupplyPlan::build(&prepared, &stats, pages(&[], &[])).unwrap();
        let tool = tool_need(&needs);
        let rune = prepared.selected.item_by_alias("rune_pickaxe").unwrap().id;
        let steel = prepared.selected.item_by_alias("steel_pickaxe").unwrap().id;
        let iron = prepared.selected.item_by_alias("iron_pickaxe").unwrap().id;
        let bronze = prepared
            .selected
            .item_by_alias("bronze_pickaxe")
            .unwrap()
            .id;

        assert_eq!(tool.item, iron);
        assert_eq!(tool.presence, Presence::Wielded);
        assert!(!tool.alternatives.contains(&rune));
        assert!(!tool.alternatives.contains(&steel));
        assert!(tool.alternatives.contains(&iron));
        assert!(tool.alternatives.contains(&bronze));
    }

    #[test]
    fn the_best_held_usable_tier_stays_selected_when_it_cannot_be_worn() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let mut stats = stats(&prepared);
        let attack = stats.iter_mut().find(|stat| stat.index == 0).unwrap();
        attack.base = 1;
        attack.effective = 1;
        let held = [
            item(1351, "Bronze axe", 1, ItemContainer::Inventory),
            item(1359, "Rune axe", 1, ItemContainer::Inventory),
        ];

        let choice = best_tool(&prepared, &stats, &held, &[]).unwrap();
        assert_eq!(choice.id, 1359);
        assert!(!choice.worn);
        assert!(!can_wield(&prepared, choice.id, &held, &stats));
        assert!(!due(&prepared, &stats, &held, &[]));

        let needs = SupplyPlan::build(&prepared, &stats, pages(&held, &[])).unwrap();
        let tool = tool_need(&needs);
        assert_eq!(tool.item, 1359);
        assert_eq!(tool.presence, Presence::Carried);
        assert_eq!(tool.kind, NeedKind::Required);
        assert_eq!(tool.target, 1);
        assert_eq!(tool.minimum, 1);
        assert!(tool.alternatives.contains(&1359));
        assert!(matches!(
            core_plan_needs(needs, pages(&held, &[])),
            AcquireVerdict::Satisfied
        ));
    }

    #[test]
    fn a_held_wieldable_tool_is_due_and_the_core_owns_wear() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let stats = stats(&prepared);
        let axe = [item(1351, "Bronze axe", 1, ItemContainer::Inventory)];
        assert!(due(&prepared, &stats, &axe, &[]));

        let needs = SupplyPlan::build(&prepared, &stats, pages(&axe, &[])).unwrap();
        let tool = tool_need(&needs);
        assert_eq!(tool.item, 1351);
        assert_eq!(tool.presence, Presence::Wielded);
        let plan = planned(core_plan_needs(needs, pages(&axe, &[])));
        assert_eq!(plan.wear_count, 1);
        assert_eq!(plan.wears[0], Some(1351));
        assert_eq!(plan.withdrawal_count, 0);

        let worn = [item(1351, "Bronze axe", 1, ItemContainer::Equipment)];
        assert!(!due(&prepared, &stats, &[], &worn));
        assert!(matches!(
            core_plan(&prepared, &stats, pages(&[], &worn)),
            AcquireVerdict::Satisfied
        ));
    }

    #[test]
    fn best_banked_tool_uses_selected_name_and_core_withdraws_that_tier() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let mut stats = stats(&prepared);
        let attack = stats.iter_mut().find(|stat| stat.index == 0).unwrap();
        attack.base = 1;
        attack.effective = 1;
        let bank = memory(&[(1351, 1), (1359, 1)], Origin::Session);
        let stock = with_bank(&[], &[], &bank);

        let needs = SupplyPlan::build(&prepared, &stats, stock).unwrap();
        let tool = tool_need(&needs);
        assert_eq!(tool.item, 1359);
        let expected_name = prepared
            .selected
            .item_by_id(1359)
            .and_then(|item| item.name.as_deref())
            .unwrap();
        assert_eq!(tool.name.as_ref(), expected_name);
        assert_eq!(tool.presence, Presence::Carried);
        let rune_rank = tool.alternatives.iter().position(|&id| id == 1359).unwrap();
        let bronze_rank = tool.alternatives.iter().position(|&id| id == 1351).unwrap();
        assert!(rune_rank < bronze_rank);

        let plan = planned(core_plan_needs(needs, stock));
        assert_eq!(targets(&plan), vec![(1359, 1)]);
    }

    #[test]
    fn absent_tool_uses_the_best_usable_wieldable_tier_as_primary_need() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let mut stats = stats(&prepared);
        stats.iter_mut().find(|stat| stat.index == 0).unwrap().base = 1;
        let needs = SupplyPlan::build(&prepared, &stats, pages(&[], &[])).unwrap();
        let tool = tool_need(&needs);
        let best = tool
            .alternatives
            .iter()
            .copied()
            .find(|&id| wield_gate_met(&prepared, id, &stats))
            .expect("at least one usable tier meets the wield gate");
        let expected_name = prepared
            .selected
            .item_by_id(best)
            .and_then(|item| item.name.as_deref())
            .unwrap();

        assert_eq!(tool.item, best);
        assert_eq!(tool.name.as_ref(), expected_name);
        assert!(
            tool.alternatives.contains(&1359),
            "Rune remains a usable tier"
        );
        assert_eq!(tool.presence, Presence::Wielded);
    }

    #[test]
    fn skill_use_gate_removes_higher_pickaxe_tiers_before_purchase_selection() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Mining"));
        let prepared = prepare(settings);
        let mut stats = stats(&prepared);
        stats.iter_mut().find(|stat| stat.index == 0).unwrap().base = 1;
        let mining = stats.iter_mut().find(|stat| stat.index == 14).unwrap();
        mining.base = 1;
        mining.effective = 1;
        let needs = SupplyPlan::build(&prepared, &stats, pages(&[], &[])).unwrap();
        let tool = tool_need(&needs);
        let iron = prepared.selected.item_by_alias("iron_pickaxe").unwrap().id;
        let rune = prepared.selected.item_by_alias("rune_pickaxe").unwrap().id;

        assert_eq!(tool.item, iron);
        assert!(!tool.alternatives.contains(&rune));
        assert!(tool.alternatives.contains(&iron));
    }

    #[test]
    fn a_better_worn_tool_wins_over_a_lower_packed_tier() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let mut stats = stats(&prepared);
        stats.iter_mut().find(|stat| stat.index == 0).unwrap().base = 40;
        let pack = [item(1351, "Bronze axe", 1, ItemContainer::Inventory)];
        let worn = [item(1359, "Rune axe", 1, ItemContainer::Equipment)];

        let choice = best_tool(&prepared, &stats, &pack, &worn).unwrap();
        assert_eq!(choice.id, 1359);
        assert!(choice.worn);
        assert!(!due(&prepared, &stats, &pack, &worn));

        let needs = SupplyPlan::build(&prepared, &stats, pages(&pack, &worn)).unwrap();
        let tool = tool_need(&needs);
        assert_eq!(tool.item, 1359);
        assert_eq!(tool.presence, Presence::Wielded);
        assert!(matches!(
            core_plan_needs(needs, pages(&pack, &worn)),
            AcquireVerdict::Satisfied
        ));
    }

    #[test]
    fn unposted_pack_or_worn_pages_do_not_admit_a_trip() {
        let prepared = prepare(crate::native::SettingsBag::new());
        let stats = stats(&prepared);
        assert!(!SupplyPlan::due(&prepared, &stats, Stock::default()));
        assert!(!SupplyPlan::due(
            &prepared,
            &stats,
            Stock {
                pack: Some(&[]),
                capacity: Some(28),
                worn: None,
                bank: None,
            }
        ));
    }

    #[test]
    fn fishing_tool_needs_stay_carried_for_every_content_defined_tool() {
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
            // Exercise tool semantics even when unrelated method facts prevent selection.
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
                    held.push(item(
                        bait.id,
                        &bait.name,
                        prepared.settings.bait_target,
                        ItemContainer::Inventory,
                    ));
                }
                let stock = pages(&held, &[]);
                let choice = best_tool(prepared, &stats, &held, &[]).unwrap();
                assert_eq!(choice.id, tool.item, "{}", method.id.0);
                assert!(!choice.worn);
                assert!(!can_wield(prepared, choice.id, &held, &stats));
                assert!(!due(prepared, &stats, &held, &[]));

                let needs = SupplyPlan::build(prepared, &stats, stock).unwrap();
                let tool_need = tool_need(&needs);
                assert_eq!(tool_need.item, tool.item, "{}", method.id.0);
                assert_eq!(tool_need.presence, Presence::Carried, "{}", method.id.0);
                assert!(tool_need.sources.contains(SourceSet::PACK));
                assert!(tool_need.sources.contains(SourceSet::WORN));
                assert!(tool_need.sources.contains(SourceSet::BANK));
                assert!(matches!(
                    core_plan_needs(needs, stock),
                    AcquireVerdict::Satisfied
                ));

                held.remove(0);
                let bank = memory(&[(tool.item, 1)], Origin::Session);
                let stock = with_bank(&held, &[], &bank);
                assert!(due(prepared, &stats, &held, &[]));
                let plan = planned(core_plan(prepared, &stats, stock));
                assert_eq!(targets(&plan), vec![(tool.item, 1)], "{}", method.id.0);
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

    #[test]
    fn need_builder_preserves_tool_bait_food_and_optional_coin_contracts() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.saltfish.op3"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Lobster"));
        settings.insert("foodTarget".into(), json!(5));
        settings.insert("coinTarget".into(), json!(500));
        let prepared = prepare(settings);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let food = prepared.supply.food.as_ref().unwrap();
        let coin_name = prepared.supply.coin_name.as_deref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = vec![
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
            item(food.id, &food.name, 2, ItemContainer::Inventory),
            item(COINS_ID, coin_name, 100, ItemContainer::Inventory),
        ];
        let stats = stats(&prepared);
        let stock = pages(&held, &[]);
        assert!(!SupplyPlan::due(&prepared, &stats, stock));

        let needs = SupplyPlan::build(&prepared, &stats, stock).unwrap();
        assert_eq!(needs.len(), 4);
        let tool = tool_need(&needs);
        assert_eq!((tool.target, tool.minimum), (1, 1));
        assert_eq!(tool.kind, NeedKind::Required);
        assert_eq!(tool.presence, Presence::Carried);
        for source in [SourceSet::PACK, SourceSet::WORN, SourceSet::BANK] {
            assert!(tool.sources.contains(source));
        }
        assert!(!tool.sources.contains(SourceSet::SHOP));

        let bait_need = needs.iter().find(|need| need.item == bait.id).unwrap();
        assert_eq!((bait_need.target, bait_need.minimum), (10, 1));
        assert_eq!(bait_need.kind, NeedKind::Required);
        assert_eq!(bait_need.presence, Presence::Held);
        assert!(bait_need.sources.contains(SourceSet::PACK));
        assert!(bait_need.sources.contains(SourceSet::BANK));
        assert!(!bait_need.sources.contains(SourceSet::WORN));
        assert!(!bait_need.sources.contains(SourceSet::SHOP));

        let food_need = needs.iter().find(|need| need.item == food.id).unwrap();
        assert_eq!((food_need.target, food_need.minimum), (5, 1));
        assert_eq!(food_need.kind, NeedKind::Required);
        assert_eq!(food_need.presence, Presence::Held);
        assert!(food_need.sources.contains(SourceSet::PACK));
        assert!(food_need.sources.contains(SourceSet::BANK));
        assert!(!food_need.sources.contains(SourceSet::WORN));
        assert!(!food_need.sources.contains(SourceSet::SHOP));

        let coin_need = needs.iter().find(|need| need.item == COINS_ID).unwrap();
        assert_eq!(coin_need.name.as_ref(), coin_name);
        assert_eq!((coin_need.target, coin_need.minimum), (500, 0));
        assert_eq!(coin_need.kind, NeedKind::Optional);
        assert_eq!(coin_need.presence, Presence::Held);
        assert!(coin_need.sources.contains(SourceSet::PACK));
        assert!(coin_need.sources.contains(SourceSet::BANK));
        assert!(!coin_need.sources.contains(SourceSet::WORN));
        assert!(!coin_need.sources.contains(SourceSet::SHOP));

        let bank = memory(
            &[(bait.id, 7), (food.id, 3), (COINS_ID, 400)],
            Origin::Session,
        );
        let plan = planned(core_plan_needs(needs, with_bank(&held, &[], &bank)));
        assert_eq!(
            targets(&plan),
            vec![(bait.id, 10), (food.id, 5), (COINS_ID, 500)]
        );

        let empty_bait = [
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 0, ItemContainer::Inventory),
            item(food.id, &food.name, 2, ItemContainer::Inventory),
        ];
        assert!(due(&prepared, &stats, &empty_bait, &[]));
    }

    #[test]
    fn a_usable_minimum_allows_a_partial_bank_withdrawal() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = [
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
        ];
        let bank = memory(&[(bait.id, 2)], Origin::Session);
        let stats = stats(&prepared);

        let plan = planned(core_plan(&prepared, &stats, with_bank(&held, &[], &bank)));

        assert_eq!(targets(&plan), vec![(bait.id, 5)]);
    }

    #[test]
    fn session_hint_and_unknown_bank_origins_follow_core_verdicts() {
        let prepared = bait_fishing(10);
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = [item(tool_id, &tool_name, 1, ItemContainer::Inventory)];
        let stats = stats(&prepared);
        assert!(due(&prepared, &stats, &held, &[]));

        let session = memory(&[], Origin::Session);
        assert!(matches!(
            core_plan(&prepared, &stats, with_bank(&held, &[], &session)),
            AcquireVerdict::Blocked(_)
        ));

        for origin in [Origin::Hint, Origin::Unknown] {
            let bank = memory(&[], origin);
            let plan = planned(core_plan(&prepared, &stats, with_bank(&held, &[], &bank)));
            assert!(plan.scan, "{origin:?} must require a bank scan");
            assert_eq!(plan.withdrawal_count, 0);
        }

        let plan = planned(core_plan(&prepared, &stats, pages(&held, &[])));
        assert!(plan.scan, "an absent bank must require a bank scan");
    }

    #[test]
    fn a_hint_partial_bank_plans_what_it_knows_and_still_scans() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = [item(tool_id, &tool_name, 1, ItemContainer::Inventory)];
        let stats = stats(&prepared);
        let hint = memory(&[(bait.id, 3)], Origin::Hint);
        let hinted = planned(core_plan(&prepared, &stats, with_bank(&held, &[], &hint)));
        assert!(hinted.scan);
        assert_eq!(targets(&hinted), vec![(bait.id, 3)]);

        let session = memory(&[(bait.id, 3)], Origin::Session);
        let observed = planned(core_plan(
            &prepared,
            &stats,
            with_bank(&held, &[], &session),
        ));
        assert!(!observed.scan);
        assert_eq!(targets(&observed), vec![(bait.id, 3)]);
    }

    #[test]
    fn coins_never_admit_a_trip_and_optional_coin_shortage_never_blocks() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.saltfish.op1"));
        settings.insert("coinTarget".into(), json!(500));
        let prepared = prepare(settings);
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = [
            item(tool_id, &tool_name, 1, ItemContainer::Inventory),
            item(
                COINS_ID,
                prepared.supply.coin_name.as_deref().unwrap(),
                100,
                ItemContainer::Inventory,
            ),
        ];
        let current_stats = stats(&prepared);
        assert!(!due(&prepared, &current_stats, &held, &[]));

        let bank = memory(&[(COINS_ID, 400)], Origin::Session);
        let plan = planned(core_plan(
            &prepared,
            &current_stats,
            with_bank(&held, &[], &bank),
        ));
        assert_eq!(targets(&plan), vec![(COINS_ID, 500)]);

        let empty = memory(&[], Origin::Session);
        assert!(matches!(
            core_plan(&prepared, &current_stats, with_bank(&held, &[], &empty)),
            AcquireVerdict::Satisfied
        ));

        let mut zero_settings = crate::native::SettingsBag::new();
        zero_settings.insert("skill".into(), json!("Fishing"));
        zero_settings.insert("fishingMethod".into(), json!("fishing.saltfish.op1"));
        zero_settings.insert("coinTarget".into(), json!(0));
        let no_coins = prepare(zero_settings);
        let zero_stats = stats(&no_coins);
        let (zero_tool, zero_name) = known_tool(&no_coins);
        let zero_held = [item(zero_tool, &zero_name, 1, ItemContainer::Inventory)];
        assert!(!due(&no_coins, &zero_stats, &zero_held, &[]));
        let needs = SupplyPlan::build(&no_coins, &zero_stats, pages(&zero_held, &[])).unwrap();
        assert!(needs.iter().all(|need| need.item != COINS_ID));
    }

    #[test]
    fn reserve_batch_lists_every_rune_at_a_fixed_target_and_ignores_worn_runes() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let spell = selected
            .teleports()
            .iter()
            .find(|spell| {
                spell.available()
                    && spell.runes.len() > 1
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
        assert!(rune_rows.len() > 1);
        let mut held = vec![item(1359, "Rune axe", 1, ItemContainer::Inventory)];
        held.push(item(
            rune_rows[0].id,
            &rune_rows[0].name,
            rune_rows[0].per_cast,
            ItemContainer::Inventory,
        ));
        let bank_rows: Vec<_> = rune_rows.iter().map(|rune| (rune.id, 10_000)).collect();
        let bank = memory(&bank_rows, Origin::Session);
        let mut stats = stats(&prepared);
        stats.iter_mut().find(|stat| stat.index == 0).unwrap().base = 1;
        let stock = with_bank(&held, &[], &bank);
        assert!(due(&prepared, &stats, &held, &[]));

        let needs = SupplyPlan::build(&prepared, &stats, stock).unwrap();
        let rune_needs: Vec<_> = needs
            .iter()
            .filter(|need| rune_rows.iter().any(|rune| rune.id == need.item))
            .collect();
        assert_eq!(rune_needs.len(), rune_rows.len());
        for rune in &rune_rows {
            let need = rune_needs.iter().find(|need| need.item == rune.id).unwrap();
            assert_eq!(need.name, rune.name);
            assert_eq!(need.target, rune.per_cast * 5);
            assert_eq!(need.minimum, rune.per_cast);
            assert_eq!(need.presence, Presence::Held);
            assert_eq!(need.kind, NeedKind::Required);
            assert!(need.sources.contains(SourceSet::PACK));
            assert!(need.sources.contains(SourceSet::BANK));
            assert!(!need.sources.contains(SourceSet::WORN));
            assert!(!need.sources.contains(SourceSet::SHOP));
        }

        let plan = planned(core_plan_needs(needs, stock));
        let mut expected: Vec<_> = rune_rows
            .iter()
            .map(|rune| (rune.id, rune.per_cast * 5))
            .collect();
        expected.sort_unstable_by_key(|row| row.0);
        let mut actual = targets(&plan);
        actual.sort_unstable_by_key(|row| row.0);
        assert_eq!(actual, expected);

        let full_cast: Vec<_> =
            std::iter::once(item(1359, "Rune axe", 1, ItemContainer::Inventory))
                .chain(
                    rune_rows.iter().map(|rune| {
                        item(rune.id, &rune.name, rune.per_cast, ItemContainer::Inventory)
                    }),
                )
                .collect();
        assert!(!due(&prepared, &stats, &full_cast, &[]));
        let full_needs = SupplyPlan::build(&prepared, &stats, pages(&full_cast, &[])).unwrap();
        assert!(rune_rows
            .iter()
            .all(|rune| full_needs.iter().all(|need| need.item != rune.id)));

        let worn_runes: Vec<_> = rune_rows
            .iter()
            .map(|rune| item(rune.id, &rune.name, rune.per_cast, ItemContainer::Equipment))
            .collect();
        let axe = [item(1359, "Rune axe", 1, ItemContainer::Inventory)];
        assert!(due(&prepared, &stats, &axe, &worn_runes));
        assert!(!can_afford_cast(reserve, pages(&axe, &worn_runes)));
    }

    #[test]
    fn reserve_target_overflow_returns_a_typed_action_error() {
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
        settings.insert("reserveCasts".into(), json!(1));
        let mut prepared = prepare(settings);
        Arc::get_mut(&mut prepared)
            .expect("the test owns its prepared configuration")
            .settings
            .reserve_casts = i32::MAX;
        let stats = stats(&prepared);
        let held = [item(1359, "Rune axe", 1, ItemContainer::Inventory)];

        let result = SupplyPlan::build(&prepared, &stats, pages(&held, &[]));

        assert!(matches!(result, Err(ActionError::Unavailable(_))));
    }

    #[test]
    fn bank_unbankable_aliases_resolve_and_stay_out_of_the_deposit_plan() {
        if !api::test_support::game_data_available() {
            return;
        }
        let prepared = prepare(crate::native::SettingsBag::new());
        let mut inventory = Vec::new();
        for (index, alias) in UNBANKABLE_ALIASES.iter().enumerate() {
            let fact = prepared.selected.item_by_alias(alias).unwrap();
            inventory.push(item(
                fact.id,
                fact.name.as_deref().unwrap(),
                1,
                ItemContainer::Inventory,
            ));
            assert_eq!(
                unbankable_index(&prepared.selected, inventory.last().unwrap()),
                Some(index)
            );
        }
        inventory.push(item(383, "Raw shark", 1, ItemContainer::Inventory));
        assert_eq!(bank_deposit_ids(&prepared, -1, &inventory).as_ref(), &[383]);
        let mut kept = protected_ids(&prepared, -1);
        kept.keep_unbankable(&prepared.selected, &inventory);
        assert!(inventory[..UNBANKABLE_ALIASES.len()]
            .iter()
            .all(|item| kept.as_slice().contains(&item.def.id)));
        assert_eq!(
            unbankable_index(&prepared.selected, inventory.last().unwrap()),
            None
        );
    }

    #[test]
    fn bank_unbankable_set_covers_engine_content_refusals() {
        let Some(engine) =
            api::test_support::local_asset_path("ENGINE_DIR", "", "289 engine content")
        else {
            return;
        };
        let scripts = engine.parent().unwrap().join("content/scripts");
        let bank =
            std::fs::read_to_string(scripts.join("interface_bank/scripts/bank.rs2")).unwrap();
        let allowed = bank
            .split_once("[proc,bank_check_allowed]")
            .expect("bank_check_allowed content")
            .1
            .split_once("[label,")
            .expect("end of bank_check_allowed")
            .0;
        let mut content_items = Vec::new();
        for clause in allowed.split("$obj").skip(1) {
            let Some(clause) = clause.trim_start().strip_prefix('=') else {
                continue;
            };
            let alias = clause
                .trim_start()
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .next()
                .unwrap();
            assert!(
                UNBANKABLE_ALIASES.contains(&alias),
                "bank_check_allowed refuses {alias}; update the Gatherer kept set"
            );
            content_items.push(alias.to_owned());
        }
        assert!(
            !content_items.is_empty(),
            "the content scan must parse bank_check_allowed item clauses; update the parser if their syntax changed"
        );
        let mut directories = vec![scripts];
        let mut parameter_items = Vec::new();
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    directories.push(entry.path());
                    continue;
                }
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("obj") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                let mut alias = None;
                for line in source.lines() {
                    let line = line.split("//").next().unwrap().trim();
                    if let Some(section) = line
                        .strip_prefix('[')
                        .and_then(|line| line.strip_suffix(']'))
                    {
                        alias = Some(section);
                        continue;
                    }
                    let Some((key, value)) = line.split_once('=') else {
                        continue;
                    };
                    let Some((parameter, value)) = value.split_once(',') else {
                        continue;
                    };
                    if key.trim() != "param" || parameter.trim() != "unbankable" {
                        continue;
                    }
                    match value.trim() {
                        "^false" | "0" => continue,
                        "^true" | "1" => {}
                        other => {
                            panic!("unknown unbankable parameter {other} in {}", path.display())
                        }
                    }
                    let alias = alias.expect("an object parameter has a section");
                    assert!(
                        UNBANKABLE_ALIASES.contains(&alias),
                        "{} marks {alias} unbankable; update the Gatherer kept set",
                        path.display()
                    );
                    content_items.push(alias.to_owned());
                    parameter_items.push(alias.to_owned());
                }
            }
        }
        assert!(
            !parameter_items.is_empty(),
            "the content scan must find parameters"
        );
        for alias in UNBANKABLE_ALIASES {
            assert!(
                content_items.iter().any(|item| item == alias),
                "{alias} is no longer refused by bank content; remove the stale Gatherer kept alias"
            );
        }
    }

    #[test]
    fn bank_deposit_ids_include_incidental_items_and_protect_current_supplies() {
        if !api::test_support::game_data_available() {
            return;
        }
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
        settings.insert("fishingMethod".into(), json!("fishing.saltfish.op3"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Lobster"));
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
        if !api::test_support::game_data_available() {
            return;
        }
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

    /// A bank trip keeps every held antipoison dose (Argon review finding 2):
    /// antipoison and super antipoison doses of any count are not deposit
    /// products, so the next poison still finds them. Strange fruit stays an
    /// incidental random-event item that the trip banks (the fish bank soak
    /// oracle, `host-play/tests/gatherer_live.rs` `bank_conservation_tests`).
    #[test]
    fn tick_kit_preparation_does_not_decide_method_or_tool_applicability() {
        if !api::test_support::game_data_available() {
            return;
        }
        let prepared = prepare(crate::native::SettingsBag::new());
        for (skill, method) in [
            ("Woodcutting", "woodcutting.normal"),
            ("Mining", "mining.copper"),
            ("Fishing", "fishing.category_633.op1"),
        ] {
            let mut settings = crate::native::SettingsBag::new();
            settings.insert("skill".into(), json!(skill));
            settings.insert("tickMethod".into(), json!("Knife"));
            let mut settings = GathererSettings::from_bag(&settings).unwrap();
            let index = prepared
                .catalog
                .methods()
                .iter()
                .position(|row| row.id.0.as_ref() == method)
                .unwrap();
            // Exercise the supply boundary directly: the full card still
            // rejects incomplete native methods for their existing reasons.
            let supply = PreparedSupply::prepare(
                &mut settings,
                &prepared.selected,
                &prepared.catalog,
                &[index],
            )
            .unwrap();
            let kit = supply.tick_kit.as_ref().unwrap();
            assert_eq!(kit.knife.name.as_ref(), "Knife");
            assert_eq!(kit.logs.name.as_ref(), "Logs");
            assert_ne!(kit.knife.id, kit.logs.id);
        }
        assert!(prepared.supply.tick_kit.is_none());
    }

    #[test]
    fn tick_kit_trips_only_for_unknown_or_usable_bank_stock() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("tickMethod".into(), json!("Knife"));
        let prepared = prepare(settings);
        let kit = prepared.supply.tick_kit.as_ref().unwrap();
        let stats = stats(&prepared);
        let (tool_id, tool_name) = known_tool(&prepared);
        // The tool is worn, so only the optional kit can admit a trip.
        let worn = [item(tool_id, &tool_name, 1, ItemContainer::Equipment)];
        let held: [ItemView; 0] = [];
        for origin in [Origin::Hint, Origin::Session] {
            for logs in [1, 10] {
                let bank = memory(&[(kit.knife.id, 1), (kit.logs.id, logs)], origin);
                let stock = with_bank(&held, &worn, &bank);
                assert!(
                    SupplyPlan::due(&prepared, &stats, stock),
                    "a bank holding the usable kit must admit its trip"
                );
                let plan = planned(core_plan(&prepared, &stats, stock));
                assert_eq!(
                    targets(&plan),
                    vec![(kit.knife.id, 1), (kit.logs.id, logs.min(3))]
                );
                assert!(!SupplyPlan::tick_kit_missing(&prepared, stock));
            }
            for rows in [vec![], vec![(kit.knife.id, 1)], vec![(kit.logs.id, 3)]] {
                let bank = memory(&rows, origin);
                let stock = with_bank(&held, &worn, &bank);
                assert!(
                    !SupplyPlan::due(&prepared, &stats, stock),
                    "a known incomplete kit is not a trip or a gating missing item"
                );
                assert!(SupplyPlan::tick_kit_missing(&prepared, stock));
                let needs = SupplyPlan::build(&prepared, &stats, stock).unwrap();
                assert!(needs
                    .iter()
                    .all(|need| need.item != kit.knife.id && need.item != kit.logs.id));
            }
        }
        let unknown = memory(&[], Origin::Unknown);
        assert!(
            SupplyPlan::due(&prepared, &stats, with_bank(&held, &worn, &unknown)),
            "an unknown bank is learned by walking"
        );
        let empty = memory(&[], Origin::Session);
        let stocked = [
            item(kit.knife.id, &kit.knife.name, 1, ItemContainer::Inventory),
            item(kit.logs.id, &kit.logs.name, 1, ItemContainer::Inventory),
        ];
        let stock = with_bank(&stocked, &worn, &empty);
        assert!(!SupplyPlan::due(&prepared, &stats, stock));
        assert!(!SupplyPlan::tick_kit_missing(&prepared, stock));
        let knife_held = &stocked[..1];
        let logs_banked = memory(&[(kit.logs.id, 3)], Origin::Session);
        let stock = with_bank(knife_held, &worn, &logs_banked);
        assert!(
            SupplyPlan::due(&prepared, &stats, stock),
            "a held knife and banked logs form a usable kit"
        );
        assert_eq!(
            targets(&planned(core_plan(&prepared, &stats, stock))),
            vec![(kit.logs.id, 3)]
        );
        assert!(!SupplyPlan::tick_kit_missing(&prepared, stock));
    }

    #[test]
    fn bank_deposit_keeps_held_antipoison_doses() {
        if !api::test_support::game_data_available() {
            return;
        }
        let prepared = prepare(crate::native::SettingsBag::new());
        let tables = crate::combat::guard::shared_tables().unwrap();
        let mut inventory = vec![item(405, "Casket", 1, ItemContainer::Inventory)];
        let mut doses = Vec::new();
        for kind in [
            crate::combat::tables::PotionKind::Antipoison,
            crate::combat::tables::PotionKind::SuperAntipoison,
        ] {
            for dose in tables.potion(kind).unwrap().doses.iter().flatten() {
                doses.push(dose.id);
                inventory.push(item(dose.id, "Cure dose", 1, ItemContainer::Inventory));
            }
        }
        assert_eq!(doses.len(), 8, "four doses of each antipoison");
        let fruit = prepared
            .selected
            .item_by_alias("macro_triffidfruit")
            .unwrap()
            .id;
        inventory.push(item(fruit, "Strange fruit", 1, ItemContainer::Inventory));

        let deposit_ids = bank_deposit_ids(&prepared, -1, &inventory);
        assert_eq!(deposit_ids.as_ref(), &[405, fruit]);
        for id in doses {
            assert!(!deposit_ids.contains(&id), "dose {id} would be deposited");
        }
    }

    #[test]
    fn a_worn_requirement_item_satisfies_method_ready() {
        if !api::test_support::game_data_available() {
            return;
        }
        const AMULET: i32 = 1704;
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let mut method = prepared
            .catalog
            .method("fishing.saltfish.op3")
            .unwrap()
            .clone();
        let Knowledge::Known(rows) = &method.requirements else {
            panic!("the bait method's requirements are known");
        };
        let mut required = rows[0].clone();
        required.kind = api::selected::RequirementKind::Item(api::selected::ItemAmount {
            item: AMULET,
            count: 1,
        });
        method.requirements = Knowledge::Known(rows.iter().cloned().chain([required]).collect());
        let stats = stats(&prepared);
        let pack = vec![
            item(FISHING_ROD, "Fishing rod", 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 5, ItemContainer::Inventory),
        ];
        let mut ledger = None;
        for (worn, held, expected) in [
            (true, false, Ok(true)),
            (false, true, Ok(true)),
            (false, false, Err("gather required item missing")),
        ] {
            let mut snapshot = api::snapshot::GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_world(api::snapshot::WorldStateView {
                members: true,
                ..Default::default()
            });
            snapshot.seed_stats(stats.clone());
            let mut inventory = pack.clone();
            if held {
                inventory.push(item(AMULET, "Amulet", 1, ItemContainer::Inventory));
            }
            snapshot.seed_inventory(inventory, 28);
            snapshot.seed_equipment(if worn {
                vec![item(AMULET, "Amulet", 1, ItemContainer::Equipment)]
            } else {
                Vec::new()
            });
            let ready =
                crate::quester::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
                    method_ready(tick.cx.snapshot(), &method, false)
                });
            assert_eq!(ready, expected, "worn={worn} held={held}");
        }
        assert!(ledger.is_none());
    }

    #[test]
    fn unchanged_due_polls_over_known_memory_allocate_nothing() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let stats = stats(&prepared);
        let stocked = [
            item(FISHING_ROD, "Fishing rod", 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
        ];
        let rod = [item(
            FISHING_ROD,
            "Fishing rod",
            1,
            ItemContainer::Inventory,
        )];
        let session = memory(&[(bait.id, 30), (COINS_ID, 40)], Origin::Session);
        let stock = with_bank(&stocked, &[], &session);
        let idle = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(!SupplyPlan::due(&prepared, &stats, stock));
            }
        });
        assert_eq!(
            idle.bytes_total, 0,
            "unchanged admission polls allocate nothing"
        );

        let due = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(SupplyPlan::due(
                    &prepared,
                    &stats,
                    with_bank(&rod, &[], &session)
                ));
            }
        });
        assert_eq!(due.bytes_total, 0, "due admission polls allocate nothing");
    }

    #[test]
    fn buy_tools_adds_shops_only_to_gathering_tools_and_bait() {
        for enabled in [false, true] {
            let mut settings = crate::native::SettingsBag::new();
            settings.insert("skill".into(), json!("Fishing"));
            settings.insert("fishingMethod".into(), json!("fishing.saltfish.op3"));
            settings.insert("buyTools".into(), json!(enabled));
            settings.insert("food".into(), json!("Trout"));
            settings.insert("foodTarget".into(), json!(3));
            settings.insert("coinTarget".into(), json!(10));
            let prepared = prepare(settings);
            let stats = stats(&prepared);
            let bank = memory(&[], Origin::Session);
            let needs = SupplyPlan::build(&prepared, &stats, with_bank(&[], &[], &bank)).unwrap();
            let bait = prepared.supply.bait.as_ref().unwrap();
            assert!(needs.iter().any(|need| need.item == FISHING_ROD));
            assert!(needs.iter().any(|need| need.item == bait.id));
            assert!(needs.iter().any(|need| need.item == COINS_ID));
            assert!(needs.iter().any(|need| need.item != FISHING_ROD
                && need.item != bait.id
                && need.item != COINS_ID));
            for need in needs.iter() {
                let tool_or_bait = need.item == FISHING_ROD || need.item == bait.id;
                assert_eq!(
                    need.sources.contains(SourceSet::SHOP),
                    enabled && tool_or_bait,
                    "id={} enabled={enabled}",
                    need.item
                );
            }
        }
    }

    #[test]
    fn buy_tools_keeps_held_and_session_banked_tiers_ahead_of_shops() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("buyTools".into(), json!(true));
        let prepared = prepare(settings);
        let stats = stats(&prepared);
        for held in [false, true] {
            let bank = memory(if held { &[] } else { &[(1351, 1)] }, Origin::Session);
            let pack = [item(1351, "Bronze axe", 1, ItemContainer::Inventory)];
            let stock = with_bank(if held { &pack } else { &[] }, &[], &bank);
            let needs = SupplyPlan::build(&prepared, &stats, stock).unwrap();
            let mut request = request(needs);
            request.vendor = VendorChoice::Nearest;
            request.coins = CoinPolicy::ForPurchase {
                sources: SourceSet::PACK | SourceSet::BANK,
            };
            let verdict = acquire::plan::plan(&request, &stock, WorldFacts::default()).unwrap();
            let plan = planned(verdict);
            assert!(plan.purchases.iter().all(Option::is_none));
            if held {
                assert_eq!(plan.wear_count, 1);
                assert_eq!(plan.withdrawal_count, 0);
            } else {
                assert_eq!(targets(&plan), [(1351, 1)]);
            }
        }
    }
}
