//! Fixed-capacity supply planning for Gatherer bank trips.
//!
//! Trip admission ([`SupplyPlan::plan`]) reads the frame's [`Stock`]: the
//! pack and worn pages decide whether a trip is due, and the account's bank
//! memory decides what that trip will find (design-bank-snapshot §2.4). The
//! withdrawal batch is rebuilt from the live rows at the loaded bank
//! ([`SupplyPlan::from_loaded_bank`]), which wins over the memory. Plans
//! contain exact final inventory counts, so rune decisions stay latched
//! across the whole withdrawal batch instead of changing as each rune arrives.

use super::card::Prepared;
use super::settings::{GathererSettings, Skill};
use crate::bank::ops;
use crate::native::ConfigError;
use api::bank_memory::Origin;
use api::game_data::{SelectedGameData, TeleportSpell};
use api::gather_methods::{known_rows, GatherCatalog, GatherMethod};
use api::selected::{Knowledge, SkillMinimum, Truth};
use api::snapshot::{ItemView, StatView};
use api::stock::Stock;
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
        if id < 0 || item.count <= 0 || protected.as_slice().contains(&id) || ids.contains(&id) {
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
    /// the bank the plan was built from. Executable withdrawals remain in the
    /// plan.
    pub fn missing(&self) -> Option<&Arc<str>> {
        self.missing.as_ref()
    }

    fn note_missing(&mut self, missing: Arc<str>) {
        if self.missing.is_none() {
            self.missing = Some(missing);
        }
    }

    /// Trip admission (design-bank-snapshot §2.4, F1/N7). The pack and worn
    /// pages decide whether a trip is due; the bank memory decides what it
    /// will find. `Unknown` learns by walking (`TripToLearn`); a `Hint` always
    /// earns the trip it predicts, short or not, because only the open bank
    /// can verify it; only a `Session`-observed bank that cannot serve a
    /// gating item fails in place (`Missing`) — that trip would end in the
    /// same `supply-missing` after the walk. Unposted pages are never due.
    pub fn plan(prepared: &Prepared, stats: &[StatView], stock: Stock<'_>) -> SupplyVerdict {
        if !Self::due(prepared, stats, stock) {
            return SupplyVerdict::NotDue;
        }
        let Some(bank) = stock.bank.filter(|bank| bank.known()) else {
            return SupplyVerdict::TripToLearn;
        };
        let plan = Self::build(prepared, stats, stock, |id| bank.count(id).unwrap_or(0));
        match (bank.origin(), plan.missing()) {
            (Origin::Session, Some(missing)) => SupplyVerdict::Missing(Arc::clone(missing)),
            _ => SupplyVerdict::Trip(plan),
        }
    }

    /// Coins are deliberately absent from trip admission: they top up only
    /// during a trip already required for another reason.
    fn due(prepared: &Prepared, stats: &[StatView], stock: Stock<'_>) -> bool {
        let (Some(inventory), Some(equipment)) = (stock.pack, stock.worn) else {
            return false;
        };
        best_tool(prepared, stats, inventory, equipment).is_none()
            || supplies_due(&prepared.supply, &prepared.settings, stock)
    }

    /// Build the entire immutable withdrawal batch from one loaded-bank view.
    /// `None` means the bank observation is not yet available and is never
    /// interpreted as an empty bank. The live rows win over any plan the
    /// bank memory made before the walk.
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
        let stock = Stock {
            pack: Some(inventory),
            capacity: None,
            worn: Some(equipment),
            bank: None,
        };
        let plan = Self::build(prepared, stats, stock, |id| ops::count_id(bank, id));
        if plan.len == 0 {
            if let Some(missing) = &plan.missing {
                return SupplyPlanResult::Missing(Arc::clone(missing));
            }
        }
        SupplyPlanResult::Ready(plan)
    }

    /// The withdrawal batch over posted pack and worn pages and a bank that
    /// holds `banked(id)` of each id: the live rows at an open bank, or the
    /// bank memory before the walk.
    fn build(
        prepared: &Prepared,
        stats: &[StatView],
        stock: Stock<'_>,
        banked: impl Fn(i32) -> i32,
    ) -> Self {
        let inventory = stock.pack.unwrap_or_default();
        let equipment = stock.worn.unwrap_or_default();
        let held = |id| stock.held(id).unwrap_or(0);
        let mut plan = Self::default();
        if best_tool(prepared, stats, inventory, equipment).is_none() {
            if let Some(tool) =
                best_tool_where(prepared, stats, |id| (banked(id) > 0).then_some(false))
            {
                if let Some(name) = tool_name(prepared, tool.id) {
                    let fact = SupplyItemFact { id: tool.id, name };
                    top_up(&mut plan, &fact, 1, 1, held(tool.id), banked(tool.id));
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
                held(bait.id),
                banked(bait.id),
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
                held(food.id),
                banked(food.id),
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
                    held(COINS_ID),
                    banked(COINS_ID),
                );
            }
        }
        if prepared.settings.reserve_casts > 0 {
            if let Some(reserve) = &prepared.supply.reserve {
                if !can_afford_cast(reserve, stock) {
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
                        top_up(
                            &mut plan,
                            &fact,
                            target,
                            rune.per_cast,
                            held(rune.id),
                            banked(rune.id),
                        );
                    }
                }
            }
        }
        plan
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

/// What trip admission decided ([`SupplyPlan::plan`]; the verbs are
/// design-bank-snapshot §2.4's Gatherer column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupplyVerdict {
    /// The pack and worn kit lack nothing a trip would fetch.
    NotDue,
    /// A trip is due and the known bank memory planned it. The open bank
    /// re-plans from its live rows ([`SupplyPlan::from_loaded_bank`]).
    Trip(SupplyPlan),
    /// A trip is due and the bank was never observed: the trip is the lookup.
    TripToLearn,
    /// A trip is due and the bank observed this session cannot serve this
    /// gating item: fail in place, without a walk.
    Missing(Arc<str>),
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
}

/// Runes are consumed from the pack: worn runes never pay a cast.
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

/// Plan `item` toward a final held `target` from `current` held and `banked`
/// in the bank. The target clamps to what the two can reach; below
/// `minimum_usable` the item is the plan's missing gate.
fn top_up(
    plan: &mut SupplyPlan,
    item: &SupplyItemFact,
    target: i32,
    minimum_usable: i32,
    current: i32,
    banked: i32,
) {
    if current >= target && current >= minimum_usable {
        return;
    }
    let available = current.saturating_add(banked.max(0));
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
                candidate
                    .use_skill
                    .is_some_and(|skill| meets_named_gate(stats, skill, level))
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

    /// Posted pack and worn pages with no bank memory.
    fn pages<'a>(held: &'a [ItemView], worn: &'a [ItemView]) -> Stock<'a> {
        Stock {
            pack: Some(held),
            capacity: Some(28),
            worn: Some(worn),
            bank: None,
        }
    }

    fn due(prepared: &Prepared, stats: &[StatView], held: &[ItemView], worn: &[ItemView]) -> bool {
        SupplyPlan::due(prepared, stats, pages(held, worn))
    }

    #[test]
    fn supply_tool_selection_uses_effective_levels_for_boosts_and_drains() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Mining"));
        let prepared = prepare(settings);
        let held = [item(1275, "Rune pickaxe", 1, ItemContainer::Inventory)];
        let mut stats = stats(&prepared);
        let mining = stats.iter_mut().find(|stat| stat.index == 14).unwrap();
        mining.base = 40;
        mining.effective = 41;
        assert_eq!(best_tool(&prepared, &stats, &held, &[]).unwrap().id, 1275);
        let mining = stats.iter_mut().find(|stat| stat.index == 14).unwrap();
        mining.base = 41;
        mining.effective = 40;
        assert!(best_tool(&prepared, &stats, &held, &[]).is_none());
        assert!(!meets_gate(
            &stats,
            SkillMinimum {
                skill: 14,
                level: 41
            }
        ));
    }

    #[test]
    fn partial_supplies_do_not_admit_a_trip_and_loaded_bank_plan_targets_final_counts() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.freshfish.op1"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Lobster"));
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
        assert!(!due(&prepared, &stats, &held, &equipment));

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
        assert!(due(&prepared, &stats, &empty_bait, &equipment));
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
        settings.insert("food".into(), json!("Lobster"));
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

        assert!(due(&prepared, &stats, &held, &equipment));
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
    fn food_setting_resolves_alias_or_case_insensitive_name() {
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
    fn missing_gating_supply_keeps_tool_withdrawal_and_does_not_gate_on_empty_coins() {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.freshfish.op1"));
        settings.insert("baitTarget".into(), json!(10));
        settings.insert("food".into(), json!("Lobster"));
        settings.insert("foodTarget".into(), json!(5));
        settings.insert("coinTarget".into(), json!(500));
        let prepared = prepare(settings);
        let bait = prepared.supply.bait.as_ref().unwrap();
        let (tool_id, tool_name) = known_tool(&prepared);
        let held = [];
        let bank = [item(tool_id, &tool_name, 1, ItemContainer::Bank)];
        let stats = stats(&prepared);
        let equipment = [];
        assert!(due(&prepared, &stats, &held, &equipment));

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

        assert!(!due(&prepared, &stats, &held, &equipment));
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
        assert!(!due(&prepared, &stats, &held, &equipment));
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
        assert!(!due(&prepared, &stats, &held, &equipment));
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
        assert!(due(&prepared, &stats, &held, &equipment));
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

        assert!(due(&prepared, &stats, &held, &equipment));
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
            assert!(due(&prepared, &stats, &handle, &[]));
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
                assert!(!due(prepared, &stats, &held, &[]));
                assert!(protected_ids(prepared, choice.id)
                    .as_slice()
                    .contains(&tool.item));
                assert!(matches!(
                    &SupplyPlan::from_loaded_bank(prepared, &stats, &held, &[], Some(&[])),
                    SupplyPlanResult::Ready(plan) if plan.iter().next().is_none()
                ));

                // A lost tool must still use the same bank stock machinery.
                held.remove(0);
                assert!(due(prepared, &stats, &held, &[]));
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

    // ---- trip admission over the bank memory (design-bank-snapshot §7 S4) ----

    const FISHING_ROD: i32 = 307;

    /// Draynor's sardine/herring Bait method: a fishing rod and one bait a catch.
    fn bait_fishing(bait_target: i32) -> Arc<Prepared> {
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), json!("Fishing"));
        settings.insert("fishingMethod".into(), json!("fishing.saltfish.op3"));
        settings.insert("baitTarget".into(), json!(bait_target));
        prepare(settings)
    }

    /// A memory holding `rows` the way the host fills it: an open-bank
    /// observe for `Session`, a hint-file load for `Hint`.
    fn memory(rows: &[(i32, i32)], origin: Origin) -> api::bank_memory::BankMemory {
        let mut memory = api::bank_memory::BankMemory::default();
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
                    api::bank_memory::ObservedAt {
                        unix_secs: 1,
                        tick: 1,
                        bank_session: 1,
                    },
                );
            }
        }
        memory
    }

    fn with_bank<'a>(
        held: &'a [ItemView],
        worn: &'a [ItemView],
        bank: &'a api::bank_memory::BankMemory,
    ) -> Stock<'a> {
        Stock {
            bank: Some(bank),
            ..pages(held, worn)
        }
    }

    fn targets(plan: &SupplyPlan) -> Vec<(i32, i32)> {
        plan.iter().map(|item| (item.id, item.target)).collect()
    }

    #[test]
    fn session_zero_bait_is_missing_in_place_and_hint_zero_bait_earns_the_trip() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.clone().unwrap();
        let stats = stats(&prepared);
        let rod = [item(
            FISHING_ROD,
            "Fishing rod",
            1,
            ItemContainer::Inventory,
        )];
        let session = memory(&[(COINS_ID, 40)], Origin::Session);
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &session)),
            SupplyVerdict::Missing(Arc::clone(&bait.name)),
            "a bank seen this session without bait fails without a walk"
        );

        // D4: a hint is advisory, so its predicted shortage is verified by
        // exactly one trip, never refused in place.
        let hint = memory(&[(COINS_ID, 40)], Origin::Hint);
        match SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &hint)) {
            SupplyVerdict::Trip(plan) => {
                assert_eq!(plan.missing(), Some(&bait.name));
                assert!(targets(&plan).is_empty());
            }
            other => panic!("a hint shortage must earn its verifying trip: {other:?}"),
        }
    }

    #[test]
    fn an_unknown_bank_trips_to_learn() {
        let prepared = bait_fishing(10);
        let stats = stats(&prepared);
        let rod = [item(
            FISHING_ROD,
            "Fishing rod",
            1,
            ItemContainer::Inventory,
        )];
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, pages(&rod, &[])),
            SupplyVerdict::TripToLearn,
            "no memory attached"
        );
        let unknown = memory(&[], Origin::Unknown);
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &unknown)),
            SupplyVerdict::TripToLearn,
            "a memory never observed"
        );
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, Stock::default()),
            SupplyVerdict::NotDue,
            "unposted pages decide nothing"
        );
    }

    /// Bug 9: a bank of 3 of 10 bait used to cost two trips (the clamped top
    /// up, then a trip that found 0). The trip still targets 3; the next due
    /// check is a `Session` shortage, decided in place.
    #[test]
    fn three_of_ten_bait_trips_for_three_then_misses_in_place() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.clone().unwrap();
        let stats = stats(&prepared);
        let rod = [item(
            FISHING_ROD,
            "Fishing rod",
            1,
            ItemContainer::Inventory,
        )];
        for origin in [Origin::Hint, Origin::Session] {
            let bank = memory(&[(bait.id, 3)], origin);
            match SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &bank)) {
                SupplyVerdict::Trip(plan) => {
                    assert_eq!(targets(&plan), vec![(bait.id, 3)], "{origin:?}");
                    assert_eq!(plan.missing(), None, "{origin:?}");
                }
                other => panic!("{origin:?}: 3 banked bait is one trip, got {other:?}"),
            }
        }

        // The open bank mirrored the withdrawal into the memory.
        let after = memory(&[], Origin::Session);
        let stocked = [
            item(FISHING_ROD, "Fishing rod", 1, ItemContainer::Inventory),
            item(bait.id, &bait.name, 3, ItemContainer::Inventory),
        ];
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, with_bank(&stocked, &[], &after)),
            SupplyVerdict::NotDue
        );
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &after)),
            SupplyVerdict::Missing(Arc::clone(&bait.name)),
            "the bait ran out: no second trip"
        );
    }

    #[test]
    fn the_open_bank_replan_overrides_a_stale_memory_plan() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.clone().unwrap();
        let stats = stats(&prepared);
        let rod = [item(
            FISHING_ROD,
            "Fishing rod",
            1,
            ItemContainer::Inventory,
        )];
        let stale = memory(&[(bait.id, 20)], Origin::Hint);
        let SupplyVerdict::Trip(remembered) =
            SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &stale))
        else {
            panic!("a hinted surplus is acted on");
        };
        assert_eq!(targets(&remembered), vec![(bait.id, 10)]);

        // Stale positive: the open bank has none, so the one trip ends here.
        assert_eq!(
            SupplyPlan::from_loaded_bank(&prepared, &stats, &rod, &[], Some(&[])),
            SupplyPlanResult::Missing(Arc::clone(&bait.name))
        );
        let live = [item(bait.id, &bait.name, 4, ItemContainer::Bank)];
        match SupplyPlan::from_loaded_bank(&prepared, &stats, &rod, &[], Some(&live)) {
            SupplyPlanResult::Ready(plan) => assert_eq!(targets(&plan), vec![(bait.id, 4)]),
            other => panic!("the live rows win over the memory: {other:?}"),
        }
    }

    #[test]
    fn a_tool_only_in_the_memory_is_planned_and_a_session_absent_tool_is_missing() {
        let prepared = bait_fishing(1);
        let bait = prepared.supply.bait.clone().unwrap();
        let stats = stats(&prepared);
        let held = [item(bait.id, &bait.name, 5, ItemContainer::Inventory)];
        let banked_rod = memory(&[(FISHING_ROD, 1)], Origin::Session);
        let SupplyVerdict::Trip(plan) =
            SupplyPlan::plan(&prepared, &stats, with_bank(&held, &[], &banked_rod))
        else {
            panic!("a banked rod is fetched");
        };
        assert_eq!(targets(&plan), vec![(FISHING_ROD, 1)]);
        let no_rod = memory(&[(COINS_ID, 1)], Origin::Session);
        assert!(matches!(
            SupplyPlan::plan(&prepared, &stats, with_bank(&held, &[], &no_rod)),
            SupplyVerdict::Missing(_)
        ));
        let worn_rod = [item(
            FISHING_ROD,
            "Fishing rod",
            1,
            ItemContainer::Equipment,
        )];
        assert_eq!(
            SupplyPlan::plan(&prepared, &stats, with_bank(&held, &worn_rod, &no_rod)),
            SupplyVerdict::NotDue,
            "a worn tool needs no trip"
        );
    }

    #[test]
    fn worn_runes_never_pay_a_reserve_cast() {
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
        let runes: Vec<ItemView> = reserve
            .runes()
            .map(|rune| item(rune.id, &rune.name, rune.per_cast, ItemContainer::Equipment))
            .collect();
        let axe = [item(1359, "Rune axe", 1, ItemContainer::Inventory)];
        let stats = stats(&prepared);
        assert!(due(&prepared, &stats, &axe, &runes));
        assert!(!can_afford_cast(reserve, pages(&axe, &runes)));
    }

    /// Bug 8: a worn requirement item satisfies `method_ready`, the way a worn
    /// tool already did.
    #[test]
    fn a_worn_requirement_item_satisfies_method_ready() {
        const AMULET: i32 = 1704;
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.clone().unwrap();
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

    /// FLOOR: the admission read path allocates nothing — not while idle with
    /// a known memory attached, and not when the memory plans a bait trip.
    #[test]
    fn trip_admission_over_a_known_memory_allocates_nothing() {
        let prepared = bait_fishing(10);
        let bait = prepared.supply.bait.clone().unwrap();
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
        let empty = memory(&[(COINS_ID, 40)], Origin::Session);
        let idle = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                let verdict =
                    SupplyPlan::plan(&prepared, &stats, with_bank(&stocked, &[], &session));
                assert_eq!(verdict, SupplyVerdict::NotDue);
            }
        });
        assert_eq!(idle.bytes_total, 0, "idle admission allocates nothing");
        let due = allocation_counter::measure(|| {
            for _ in 0..100 {
                let trip = SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &session));
                assert!(matches!(trip, SupplyVerdict::Trip(_)));
                let missing = SupplyPlan::plan(&prepared, &stats, with_bank(&rod, &[], &empty));
                assert!(matches!(missing, SupplyVerdict::Missing(_)));
            }
        });
        assert_eq!(
            due.bytes_total, 0,
            "a memory-planned bait verdict allocates nothing"
        );
    }
}
