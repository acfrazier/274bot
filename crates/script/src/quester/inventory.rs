//! Immutable Path inventory ownership and per-stage held-item reservations.
//!
//! This module reads authored Path declarations and selected content facts only;
//! it never runs a step or derives inventory metadata from execution.

use super::super::path::{PathRoleDocument, PredicateDocument, QuestHeaderDocument, StepDocument};
use super::{BankItem, CompileContext, CompileError, CompiledRecipeItem};
use api::game_data::{self, GameItem, SelectedGameData};
use api::loadout::Loadout;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

const DYNAMIC_HELD_TARGET: i32 = 28;
const MAX_INVENTORY_METADATA_ITEMS: usize = 4096;
const MAX_RECIPE_NESTING_DEPTH: usize = super::MAX_RECIPE_NESTING_DEPTH;

/// Sorted managed IDs and their carried-only subset.
pub(super) type ManagedItemSets = (Arc<[i32]>, Arc<[i32]>);

/// Items a single stage may need held, plus the equipment positions its authored
/// steps can alter or require. Item quantities are per-ID maxima, not sums.
pub(crate) struct StageInventory {
    pub items: Arc<[CompiledRecipeItem]>,
    pub equipment_slots: u16,
}

const _: () = assert!(std::mem::size_of::<StageInventory>() <= 24);
/// Whether a candidate can be worn without displacing Path-owned inventory.
///
/// `managed_ids` protects authored items already worn; `carried_ids` excludes
/// gear reserved for carried-only steps. `claimed_slots` excludes equipment
/// positions reserved by the current stage.
pub(crate) fn gear_allowed(
    selected: &SelectedGameData,
    id: i32,
    stock: api::stock::Stock<'_>,
    managed_ids: &[i32],
    carried_ids: &[i32],
    claimed_slots: u16,
) -> bool {
    if carried_ids.contains(&id) {
        return false;
    }
    let Some(item) = selected.item_by_id(id) else {
        return false;
    };
    let Some(mask) = wear_mask(item) else {
        return false;
    };
    mask & claimed_slots == 0
        && !stock.worn.into_iter().flatten().any(|row| {
            row.count > 0
                && row.def.id != id
                && managed_ids.contains(&row.def.id)
                && selected
                    .item_by_id(row.def.id)
                    .and_then(wear_mask)
                    .is_none_or(|worn| mask & worn != 0)
        })
}

fn wear_mask(item: &GameItem) -> Option<u16> {
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

/// Combine Path reservations with every currently held ordinary food item.
///
/// This is built once when a capacity-deposit request is prepared, not during
/// polling. The inventory contributes at most 28 distinct rows.
pub(crate) fn capacity_keep_with_held_foods(
    stock: api::stock::Stock<'_>,
    keep_ids: &Arc<[i32]>,
    mut is_food: impl FnMut(i32) -> bool,
) -> Arc<[i32]> {
    let new_food = stock
        .pack
        .into_iter()
        .flatten()
        .any(|row| row.count > 0 && is_food(row.def.id) && !keep_ids.contains(&row.def.id));
    if !new_food {
        return Arc::clone(keep_ids);
    }
    let mut keep = Vec::with_capacity(keep_ids.len() + 28);
    keep.extend_from_slice(keep_ids);
    for row in stock.pack.into_iter().flatten() {
        if row.count > 0 && is_food(row.def.id) && !keep.contains(&row.def.id) {
            keep.push(row.def.id);
        }
    }
    Arc::from(keep)
}


struct ItemFacts {
    id: i32,
    name: Arc<str>,
    stackable: bool,
    wear_positions: [i32; 3],
}

impl ItemFacts {
    fn from_item(item: &GameItem, fallback: &str) -> Self {
        Self {
            id: item.id,
            name: Arc::from(item.name.as_deref().unwrap_or(fallback)),
            stackable: item.stackable,
            wear_positions: [
                item.wear_position,
                item.wear_position_2,
                item.wear_position_3,
            ],
        }
    }
}

struct Collector<'a> {
    selected: &'a SelectedGameData,
    managed: BTreeSet<i32>,
    carried: BTreeSet<i32>,
    stage: BTreeMap<i32, CompiledRecipeItem>,
    equipment_slots: u16,
    managed_mode: bool,
    stage_mode: bool,
}

impl<'a> Collector<'a> {
    fn managed(selected: &'a SelectedGameData) -> Self {
        Self {
            selected,
            managed: BTreeSet::new(),
            carried: BTreeSet::new(),
            stage: BTreeMap::new(),
            equipment_slots: 0,
            managed_mode: true,
            stage_mode: false,
        }
    }

    fn stage(selected: &'a SelectedGameData) -> Self {
        Self {
            selected,
            managed: BTreeSet::new(),
            carried: BTreeSet::new(),
            stage: BTreeMap::new(),
            equipment_slots: 0,
            managed_mode: false,
            stage_mode: true,
        }
    }

    fn add_item(
        &mut self,
        item: &ItemFacts,
        qty: i32,
        carried: bool,
        reserve: bool,
    ) -> Result<(), CompileError> {
        if self.managed_mode {
            self.managed.insert(item.id);
            if carried {
                self.carried.insert(item.id);
            }
            if self.managed.len() > MAX_INVENTORY_METADATA_ITEMS
                || self.carried.len() > MAX_INVENTORY_METADATA_ITEMS
            {
                return Err(CompileError::code("too-many-managed-items"));
            }
        }
        if self.stage_mode && reserve {
            let entry = self
                .stage
                .entry(item.id)
                .or_insert_with(|| CompiledRecipeItem {
                    item: BankItem {
                        id: item.id,
                        name: Arc::clone(&item.name),
                    },
                    qty: 0,
                    stackable: item.stackable,
                });
            entry.qty = entry.qty.max(qty.max(1));
            if self.stage.len() > MAX_INVENTORY_METADATA_ITEMS {
                return Err(CompileError::code("too-many-stage-inventory-items"));
            }
        }
        Ok(())
    }

    fn add_alias(
        &mut self,
        alias: &str,
        qty: i32,
        carried: bool,
        reserve: bool,
    ) -> Result<ItemFacts, CompileError> {
        let row = self
            .selected
            .item_by_alias(alias)
            .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias))?;
        let item = ItemFacts::from_item(row, alias);
        self.add_item(&item, qty, carried, reserve)?;
        Ok(item)
    }

    fn add_exact_id(
        &mut self,
        id: i32,
        qty: i32,
        carried: bool,
        reserve: bool,
    ) -> Result<ItemFacts, CompileError> {
        let row = self
            .selected
            .item_by_id(id)
            .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(id.to_string()))?;
        let item = ItemFacts::from_item(row, "");
        self.add_item(&item, qty, carried, reserve)?;
        Ok(item)
    }

    fn add_name(
        &mut self,
        name: &str,
        qty: i32,
        carried: bool,
        reserve: bool,
    ) -> Result<ItemFacts, CompileError> {
        let item = self
            .selected
            .resolve_item_name(name)
            .ok_or_else(|| CompileError::code("unresolved-loadout-item").with_detail(name))?;
        let facts = ItemFacts::from_item(item, name);
        self.add_item(&facts, qty, carried, reserve)?;
        Ok(facts)
    }

    fn add_raw_managed_id(&mut self, id: i32) -> Result<(), CompileError> {
        if self.managed_mode {
            self.managed.insert(id);
            if self.managed.len() > MAX_INVENTORY_METADATA_ITEMS {
                return Err(CompileError::code("too-many-managed-items"));
            }
        }
        Ok(())
    }

    fn claim_slot_name(&mut self, slot: &str) {
        let lower = slot.to_ascii_lowercase();
        if let Some(position) = game_data::wearpos_for_loadout_slot(&lower) {
            self.claim_position(position);
        }
    }

    fn claim_all_loadout_slots(&mut self) {
        for slot in api::loadout::WORN_SLOTS {
            if let Some(position) = game_data::wearpos_for_loadout_slot(slot) {
                self.claim_position(position);
            }
        }
    }

    fn claim_item_positions(&mut self, positions: &[i32; 3]) {
        for position in positions.iter().copied() {
            // The selected decoder also reports appearance-only positions.
            // Only actual loadout positions belong in this claim mask.
            if game_data::loadout_slot_for_wearpos(position).is_some() {
                self.claim_position(position);
            }
        }
    }

    fn claim_position(&mut self, position: i32) {
        if let Ok(position) = u32::try_from(position) {
            if position < u16::BITS {
                self.equipment_slots |= 1u16 << position;
            }
        }
    }

    fn finish_managed(self) -> ManagedItemSets {
        (
            Arc::from(self.managed.into_iter().collect::<Vec<_>>()),
            Arc::from(self.carried.into_iter().collect::<Vec<_>>()),
        )
    }

    fn finish_stage(self) -> StageInventory {
        StageInventory {
            items: Arc::from(self.stage.into_values().collect::<Vec<_>>()),
            equipment_slots: self.equipment_slots,
        }
    }
}

/// Compile the narrow Path-owned item set and its carried-only subset.
pub(super) fn compile_managed_items(
    header: &QuestHeaderDocument,
    role: &PathRoleDocument,
    cx: &CompileContext<'_>,
) -> Result<ManagedItemSets, CompileError> {
    let mut collector = Collector::managed(cx.selected);
    for item in &header.items {
        collector.add_alias(&item.obj, 1, false, false)?;
    }
    for tool in &header.tools {
        let alias = tool
            .strip_prefix("obj:")
            .ok_or_else(|| CompileError::code("invalid-tool"))?;
        collector.add_alias(alias, 1, false, false)?;
    }
    collect_header_loadouts(&mut collector, header, cx, false)?;

    let mut visited = HashSet::new();
    let mut active = Vec::new();
    for name in header.acquire.keys() {
        scan_recipe(
            &mut collector,
            name,
            header,
            cx,
            &mut visited,
            &mut active,
            0,
        )?;
    }
    for step in role
        .prelude
        .iter()
        .chain(
            role.sequences
                .iter()
                .flat_map(|sequence| sequence.steps.iter()),
        )
        .chain(role.progress_reader.iter())
    {
        scan_step(
            &mut collector,
            step,
            header,
            cx,
            &mut visited,
            &mut active,
            0,
        )?;
    }
    Ok(collector.finish_managed())
}

/// Build the exact immutable reservation for one compiled Path stage.
pub(crate) fn stage_inventory<'a>(
    steps: impl Iterator<Item = &'a StepDocument>,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
) -> Result<StageInventory, CompileError> {
    let mut collector = Collector::stage(cx.selected);
    let mut visited = HashSet::new();
    let mut active = Vec::new();
    for step in steps {
        scan_step(
            &mut collector,
            step,
            header,
            cx,
            &mut visited,
            &mut active,
            0,
        )?;
    }
    Ok(collector.finish_stage())
}

fn collect_header_loadouts(
    collector: &mut Collector<'_>,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
    reserve: bool,
) -> Result<(), CompileError> {
    for (name, authored) in &header.loadouts {
        for (slot, alias) in &authored.worn {
            let item = collector.add_alias(alias, 1, false, reserve)?;
            collector.claim_slot_name(slot);
            collector.claim_item_positions(&item.wear_positions);
        }
        for carry in &authored.carry {
            let qty =
                i32::try_from(carry.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
            collector.add_alias(&carry.item, qty, false, reserve)?;
        }

        let qualified = format!("{}/{}", cx.path.0, name);
        let Some(loadout) = cx.loadouts.resolve(&qualified) else {
            return Err(CompileError::code("unknown-loadout").with_detail(qualified));
        };
        if loadout.operator_override() {
            collect_operator_loadout(collector, loadout.row(), false, reserve)?;
        }
    }
    Ok(())
}

fn collect_operator_loadout(
    collector: &mut Collector<'_>,
    row: &Loadout,
    required: bool,
    reserve_worn: bool,
) -> Result<(), CompileError> {
    for (slot, name) in &row.worn {
        let item = collector.add_name(name, 1, false, reserve_worn)?;
        collector.claim_slot_name(slot);
        collector.claim_item_positions(&item.wear_positions);
    }
    for name in &row.unassigned {
        collector.add_name(name, 1, false, false)?;
    }
    for carry in &row.carry {
        let qty = i32::try_from(carry.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
        collector.add_name(carry.item.as_str(), qty, required, required)?;
    }
    Ok(())
}

fn collect_loadout_reference(
    collector: &mut Collector<'_>,
    name: &str,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
    required: bool,
    reserve_worn: bool,
    allow_lower_tier: bool,
) -> Result<(), CompileError> {
    let qualified = if name.contains('/') {
        name.to_owned()
    } else {
        format!("{}/{}", cx.path.0, name)
    };
    let loadout = cx
        .loadouts
        .resolve(&qualified)
        .ok_or_else(|| CompileError::code("unknown-loadout").with_detail(qualified.as_str()))?;
    let local_name = qualified
        .strip_prefix(&format!("{}/", cx.path.0))
        .and_then(|local| header.loadouts.get(local));
    if loadout.operator_override() {
        collect_operator_loadout(collector, loadout.row(), required, reserve_worn)?;
    } else if let Some(authored) = local_name {
        for (slot, alias) in &authored.worn {
            let item = collector.add_alias(alias, 1, false, reserve_worn)?;
            collector.claim_slot_name(slot);
            collector.claim_item_positions(&item.wear_positions);
        }
        for carry in &authored.carry {
            let qty =
                i32::try_from(carry.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
            collector.add_alias(&carry.item, qty, required, required)?;
        }
    } else {
        // A compiled row without its source header is not expected, but its
        // display-name values remain the compiler's established resolution form.
        collect_operator_loadout(collector, loadout.row(), required, reserve_worn)?;
    }

    if allow_lower_tier {
        collect_lower_tier_candidates(collector, loadout.row(), reserve_worn)?;
    }
    Ok(())
}

fn collect_lower_tier_candidates(
    collector: &mut Collector<'_>,
    row: &Loadout,
    reserve: bool,
) -> Result<(), CompileError> {
    for (slot, wanted) in &row.worn {
        let wanted_item = collector
            .selected
            .resolve_item_name(wanted)
            .ok_or_else(|| {
                CompileError::code("unresolved-loadout-item").with_detail(wanted.as_str())
            })?;
        let wanted_name = wanted_item.name.as_deref().unwrap_or(wanted);
        for candidate in collector.selected.items() {
            if candidate.is_certificate() {
                continue;
            }
            let Some(name) = candidate.name.as_deref() else {
                continue;
            };
            let special = (wanted_name.eq_ignore_ascii_case("dragon longsword")
                && name.eq_ignore_ascii_case("rune sword"))
                || (wanted_name.eq_ignore_ascii_case("rune platebody")
                    && name.eq_ignore_ascii_case("rune chainbody"));
            let relevant = candidate.id == wanted_item.id
                || special
                || crate::melee_weapons::same_or_lower_metal_item(name, wanted_name)
                || (slot.eq_ignore_ascii_case("righthand")
                    && crate::melee_weapons::same_or_lower_melee_weapon(name, wanted_name));
            if relevant
                && collector
                    .selected
                    .resolve_item_name(name)
                    .is_some_and(|resolved| resolved.id == candidate.id)
            {
                let item = ItemFacts::from_item(candidate, name);
                collector.add_item(&item, 1, false, reserve)?;
                collector.claim_slot_name(slot);
                collector.claim_item_positions(&item.wear_positions);
            }
        }
    }
    Ok(())
}

fn scan_step(
    collector: &mut Collector<'_>,
    step: &StepDocument,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
    visited_recipes: &mut HashSet<String>,
    active_recipes: &mut Vec<String>,
    recipe_depth: usize,
) -> Result<(), CompileError> {
    scan_step_core(
        collector,
        step,
        header,
        cx,
        visited_recipes,
        active_recipes,
        recipe_depth,
    )?;
    scan_embedded_steps(
        collector,
        &step.args,
        header,
        cx,
        visited_recipes,
        active_recipes,
        recipe_depth,
    )
}

fn scan_step_core(
    collector: &mut Collector<'_>,
    step: &StepDocument,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
    visited_recipes: &mut HashSet<String>,
    active_recipes: &mut Vec<String>,
    recipe_depth: usize,
) -> Result<(), CompileError> {
    scan_predicate(collector, &step.skip_if, true, header, cx)?;
    scan_predicate(collector, &step.settle, true, header, cx)?;
    if let Some(signal) = &step.failure_signal {
        scan_predicate(collector, signal, true, header, cx)?;
    }
    scan_embedded_predicates(collector, &step.args, header, cx)?;

    let args = &step.args;
    match step.kind.as_str() {
        "bank" => {
            let op = args.get("op").and_then(serde_json::Value::as_str);
            if let Some(items) = args.get("items").and_then(serde_json::Value::as_array) {
                for row in items {
                    if let Some(alias) = row.get("obj").and_then(serde_json::Value::as_str) {
                        let qty = value_quantity(row.get("qty"), collector, false)?;
                        collector.add_alias(alias, qty, false, op == Some("withdraw"))?;
                    }
                }
            }
            if let Some(keep) = args.get("keep").and_then(serde_json::Value::as_array) {
                for alias in keep.iter().filter_map(serde_json::Value::as_str) {
                    collector.add_alias(alias, 1, false, false)?;
                }
            }
            if let Some(ids) = args.get("keep_ids").and_then(serde_json::Value::as_array) {
                for id in ids.iter().filter_map(serde_json::Value::as_i64) {
                    if let Ok(id) = i32::try_from(id) {
                        collector.add_raw_managed_id(id)?;
                    }
                }
            }
        }
        "buy" => {
            if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                let qty = value_quantity(args.get("qty"), collector, false)?;
                collector.add_alias(alias, qty, false, true)?;
            }
        }
        "make" => {
            let qty = value_quantity(args.get("qty"), collector, true)?;
            if let Some(alias) = args.get("product").and_then(serde_json::Value::as_str) {
                collector.add_alias(alias, qty, false, true)?;
            }
            if let Some(alias) = args
                .get("menu")
                .and_then(|menu| menu.get("obj"))
                .and_then(serde_json::Value::as_str)
            {
                collector.add_alias(alias, 1, true, true)?;
            }
        }
        "equip" | "unequip" => {
            if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                let item = collector.add_alias(alias, 1, false, false)?;
                collector.claim_item_positions(&item.wear_positions);
            } else if step.kind == "unequip"
                && args.get("all").and_then(serde_json::Value::as_bool) == Some(true)
            {
                collector.claim_all_loadout_slots();
            }
        }
        "loadout" => {
            if let Some(name) = args.get("loadout").and_then(serde_json::Value::as_str) {
                collect_loadout_reference(
                    collector,
                    name,
                    header,
                    cx,
                    true,
                    true,
                    args.get("allow_lower_tier")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true),
                )?;
            }
            if args.get("strip").and_then(serde_json::Value::as_bool) == Some(true)
                || args.get("exclusive").and_then(serde_json::Value::as_bool) == Some(true)
            {
                collector.claim_all_loadout_slots();
            }
        }
        "interact" => {
            let target = args.get("target");
            if let Some(alias) = target
                .and_then(|target| target.get("ground"))
                .and_then(serde_json::Value::as_str)
            {
                let take = args
                    .get("op")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|op| op.eq_ignore_ascii_case("Take"));
                collector.add_alias(alias, 1, false, take)?;
            }
            if let Some(alias) = target
                .and_then(|target| target.get("held"))
                .and_then(serde_json::Value::as_str)
            {
                collector.add_alias(alias, 1, true, true)?;
            }
            if let Some(until) = args.get("until") {
                scan_until_goal(collector, until, true, true)?;
            }
            if let Some(predicate) = args.get("wait_until") {
                scan_embedded_predicates(collector, predicate, header, cx)?;
            }
        }
        "use_on" => {
            if let Some(alias) = args.get("item").and_then(serde_json::Value::as_str) {
                collector.add_alias(alias, 1, true, true)?;
            }
            if let Some(target) = args.get("target") {
                if let Some(alias) = target.get("item").and_then(serde_json::Value::as_str) {
                    collector.add_alias(alias, 1, true, true)?;
                }
                if let Some(alias) = target.get("ground").and_then(serde_json::Value::as_str) {
                    collector.add_alias(alias, 1, false, false)?;
                }
            }
            if let Some(alias) = args.get("product").and_then(serde_json::Value::as_str) {
                collector.add_alias(alias, 1, false, true)?;
            }
            if let Some(until) = args.get("until") {
                scan_until_goal(collector, until, true, true)?;
            }
            if let Some(predicate) = args.get("no_product") {
                scan_embedded_predicates(collector, predicate, header, cx)?;
            }
        }
        "combat" => {
            if let Some(loot) = args.get("loot").and_then(serde_json::Value::as_array) {
                for row in loot {
                    match row {
                        serde_json::Value::String(alias) => {
                            collector.add_alias(alias, 1, false, true)?;
                        }
                        serde_json::Value::Object(_) => {
                            if let Some(alias) = row.get("obj").and_then(serde_json::Value::as_str)
                            {
                                let qty = value_quantity(row.get("qty"), collector, false)?;
                                collector.add_alias(alias, qty, false, true)?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(drop) = args.get("until_drop") {
                if let Some(alias) = drop.get("obj").and_then(serde_json::Value::as_str) {
                    collector.add_alias(alias, 1, false, true)?;
                }
            }
            if let Some(predicate) = args.get("until") {
                scan_embedded_predicates(collector, predicate, header, cx)?;
            }
            if let Some(predicate) = args.get("win") {
                scan_embedded_predicates(collector, predicate, header, cx)?;
            }
            for name in combat_loadout_names(args) {
                collect_loadout_reference(collector, name, header, cx, true, true, false)?;
            }
        }
        "gather" | "thieve" => {
            if let Some(until) = args.get("until") {
                if let Some(obj) = until.get("obj") {
                    let qty = value_quantity(until.get("qty"), collector, true)?;
                    add_item_arg(collector, obj, qty, true, true)?;
                }
            }
        }
        "partner" => {
            for (field, carried) in [("give", true), ("take", false)] {
                if let Some(rows) = args.get(field).and_then(serde_json::Value::as_array) {
                    for row in rows {
                        if let Some(alias) = row.get("obj").and_then(serde_json::Value::as_str) {
                            let alias = alias.strip_prefix("obj:").unwrap_or(alias);
                            let qty = value_quantity(row.get("qty"), collector, false)?;
                            collector.add_alias(alias, qty, carried, true)?;
                        }
                    }
                }
            }
        }
        "acquire" => {
            let recipe = args
                .get("recipe")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| CompileError::code("invalid-args"))?;
            for item in header
                .items
                .iter()
                .filter(|item| item.acquire.as_deref() == Some(recipe))
            {
                let qty =
                    i32::try_from(item.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
                collector.add_alias(&item.obj, qty, false, true)?;
            }
            scan_recipe(
                collector,
                recipe,
                header,
                cx,
                visited_recipes,
                active_recipes,
                recipe_depth,
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn combat_loadout_names(args: &serde_json::Value) -> impl Iterator<Item = &str> {
    args.get("loadout")
        .and_then(serde_json::Value::as_str)
        .into_iter()
        .chain(
            ["melee", "ranged", "mage"]
                .into_iter()
                .filter_map(|style| args.get("loadouts")?.get(style)?.as_str()),
        )
}

fn scan_recipe(
    collector: &mut Collector<'_>,
    name: &str,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
    visited: &mut HashSet<String>,
    active: &mut Vec<String>,
    depth: usize,
) -> Result<(), CompileError> {
    if active.iter().any(|entry| entry == name) {
        return Err(CompileError::code("recipe-cycle").with_detail(name));
    }
    if visited.contains(name) {
        return Ok(());
    }
    let recipe = header
        .acquire
        .get(name)
        .ok_or_else(|| CompileError::code("unresolved-recipe").with_detail(name))?;
    if depth >= MAX_RECIPE_NESTING_DEPTH {
        return Err(CompileError::code("recipe-nesting-limit").with_detail(name));
    }
    active.push(name.to_owned());
    for step in recipe {
        scan_step(collector, step, header, cx, visited, active, depth + 1)?;
    }
    active.pop();
    visited.insert(name.to_owned());
    Ok(())
}

fn scan_until_goal(
    collector: &mut Collector<'_>,
    value: &serde_json::Value,
    carried: bool,
    reserve: bool,
) -> Result<(), CompileError> {
    let alias = value
        .get("obj")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CompileError::code("invalid-args"))?;
    let qty = value_quantity(value.get("qty"), collector, true)?;
    collector.add_alias(alias, qty, carried, reserve)?;
    Ok(())
}

fn add_item_arg(
    collector: &mut Collector<'_>,
    value: &serde_json::Value,
    qty: i32,
    carried: bool,
    reserve: bool,
) -> Result<(), CompileError> {
    match value {
        serde_json::Value::String(alias) => {
            collector.add_alias(alias, qty, carried, reserve)?;
        }
        serde_json::Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(serde_json::Value::as_i64) {
                let id = i32::try_from(id).map_err(|_| CompileError::code("unresolved-obj"))?;
                collector.add_exact_id(id, qty, carried, reserve)?;
            } else if let Some(alias) = object.get("alias").and_then(serde_json::Value::as_str) {
                collector.add_alias(alias, qty, carried, reserve)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn value_quantity(
    value: Option<&serde_json::Value>,
    collector: &mut Collector<'_>,
    include_minus_item: bool,
) -> Result<i32, CompileError> {
    match value {
        None => Ok(1),
        Some(value) if value.as_i64().is_some() || value.as_u64().is_some() => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|qty| i64::try_from(qty).ok()))
            .and_then(|qty| i32::try_from(qty).ok())
            .ok_or_else(|| CompileError::code("invalid-quantity")),
        Some(serde_json::Value::Object(object)) if object.contains_key("progress") => {
            if include_minus_item {
                if let Some(alias) = object.get("minus_item").and_then(serde_json::Value::as_str) {
                    collector.add_alias(alias, 1, true, false)?;
                }
            }
            Ok(DYNAMIC_HELD_TARGET)
        }
        Some(_) => Err(CompileError::code("invalid-quantity")),
    }
}

fn scan_embedded_predicates(
    collector: &mut Collector<'_>,
    value: &serde_json::Value,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
) -> Result<(), CompileError> {
    if is_predicate_value(value) {
        let predicate: PredicateDocument = serde_json::from_value(value.clone())
            .map_err(|_| CompileError::code("invalid-args"))?;
        return scan_predicate(collector, &predicate, true, header, cx);
    }
    match value {
        serde_json::Value::Object(object) => {
            for child in object.values() {
                scan_embedded_predicates(collector, child, header, cx)?;
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                scan_embedded_predicates(collector, child, header, cx)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_predicate_value(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 1
        && (object.get("All").is_some_and(serde_json::Value::is_array)
            || object.get("Any").is_some_and(serde_json::Value::is_array)
            || object.get("Not").is_some_and(serde_json::Value::is_object)
            || object.get("Fact").is_some_and(serde_json::Value::is_object))
}

fn scan_embedded_steps(
    collector: &mut Collector<'_>,
    value: &serde_json::Value,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
    visited_recipes: &mut HashSet<String>,
    active_recipes: &mut Vec<String>,
    recipe_depth: usize,
) -> Result<(), CompileError> {
    if super::nested_step_parts(value).is_some() {
        let step: StepDocument = serde_json::from_value(value.clone())
            .map_err(|_| CompileError::code("invalid-args"))?;
        scan_step(
            collector,
            &step,
            header,
            cx,
            visited_recipes,
            active_recipes,
            recipe_depth,
        )?;
        return Ok(());
    }
    match value {
        serde_json::Value::Object(object) => {
            for child in object.values() {
                scan_embedded_steps(
                    collector,
                    child,
                    header,
                    cx,
                    visited_recipes,
                    active_recipes,
                    recipe_depth,
                )?;
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                scan_embedded_steps(
                    collector,
                    child,
                    header,
                    cx,
                    visited_recipes,
                    active_recipes,
                    recipe_depth,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn scan_predicate(
    collector: &mut Collector<'_>,
    predicate: &PredicateDocument,
    positive: bool,
    header: &QuestHeaderDocument,
    cx: &CompileContext<'_>,
) -> Result<(), CompileError> {
    match predicate {
        PredicateDocument::All(items) | PredicateDocument::Any(items) => {
            for item in items {
                scan_predicate(collector, item, positive, header, cx)?;
            }
        }
        PredicateDocument::Not(inner) => scan_predicate(collector, inner, !positive, header, cx)?,
        PredicateDocument::Fact { kind, args, .. } => match kind.as_str() {
            "has_item" => {
                if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                    collector.add_alias(alias, 1, positive, positive)?;
                }
            }
            "item_count_at_least" => {
                if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                    let qty = value_quantity(args.get("qty"), collector, true)?;
                    collector.add_alias(alias, qty, positive, positive)?;
                }
            }
            "partner_item_count_at_least" => {
                if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                    let alias = alias.strip_prefix("obj:").unwrap_or(alias);
                    let qty = value_quantity(args.get("qty"), collector, false)?;
                    collector.add_alias(alias, qty, positive, positive)?;
                }
            }
            "ground_item_near" | "bank_has" => {
                if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                    let reserve = kind == "ground_item_near" && positive;
                    collector.add_alias(alias, 1, false, reserve)?;
                }
            }
            "worn" => {
                if let Some(alias) = args.get("obj").and_then(serde_json::Value::as_str) {
                    let item = collector.add_alias(alias, 1, false, false)?;
                    collector.claim_item_positions(&item.wear_positions);
                }
            }
            "equipment_only" => {
                if let Some(items) = args.get("objs").and_then(serde_json::Value::as_array) {
                    for alias in items.iter().filter_map(serde_json::Value::as_str) {
                        let item = collector.add_alias(alias, 1, false, false)?;
                        collector.claim_item_positions(&item.wear_positions);
                    }
                }
                if positive {
                    collector.claim_all_loadout_slots();
                }
            }
            "pack_only" => {
                if let Some(items) = args.get("objs").and_then(serde_json::Value::as_array) {
                    for alias in items.iter().filter_map(serde_json::Value::as_str) {
                        collector.add_alias(alias, 1, false, false)?;
                    }
                }
            }
            "loadout_ready" => {
                if let Some(name) = args.get("loadout").and_then(serde_json::Value::as_str) {
                    collect_loadout_reference(
                        collector,
                        name,
                        header,
                        cx,
                        positive,
                        false,
                        args.get("allow_lower_tier")
                            .and_then(serde_json::Value::as_bool)
                            == Some(true),
                    )?;
                }
                if args.get("strip").and_then(serde_json::Value::as_bool) == Some(true)
                    || args.get("exclusive").and_then(serde_json::Value::as_bool) == Some(true)
                {
                    collector.claim_all_loadout_slots();
                }
            }
            _ => {}
        },
    }
    Ok(())
}

/// Every acquire recipe and role step is finite under Path's established
/// recipe nesting limit. This keeps metadata allocation bounded independently
/// of the authored document's repeated references to the same recipe.
const _: () = assert!(MAX_INVENTORY_METADATA_ITEMS <= 1 << 16);

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
