//! Immutable, selected-revision combat lookups shared by combat runs.
//!
//! The source facts stay in `SelectedGameData`; this index keeps combat-path
//! lookups borrowed and allocation-free after its one-time construction.

use std::sync::Arc;

use super::request::MeleeMode;
use crate::native::ActionError;
use api::game_data::{
    ConsumptionFact, NpcNameRow, PrayerFact, SelectedGameData, StyleSpotanimFact, WeaponStyleFact,
};

const STYLE_BITS: u8 = 0x0f;
const PRAYER_TIERS: usize = 3;
const MELEE_MODE_SLOTS: usize = 4;
const COMBAT_TAB_COUNT: usize = 16;
type MeleeModeIndex = [[Option<usize>; MELEE_MODE_SLOTS]; COMBAT_TAB_COUNT];

fn unavailable(reason: &'static str) -> ActionError {
    ActionError::Unavailable(Arc::from(reason))
}

/// Bitset of combat styles carried by one animation or weapon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyleMask(pub(crate) u8);

impl StyleMask {
    pub const MELEE: Self = Self(0x01);
    pub const RANGED: Self = Self(0x02);
    pub const MAGIC: Self = Self(0x04);
    pub const DRAGONFIRE: Self = Self(0x08);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, styles: Self) -> bool {
        self.0 & styles.0 == styles.0
    }

    fn from_bits(bits: u8) -> Option<Self> {
        (bits != 0 && bits & !STYLE_BITS == 0).then_some(Self(bits))
    }
}

impl std::ops::BitOr for StyleMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for StyleMask {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// The three generated prayer role groups.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum PrayerRole {
    /// Tiers 0–2 are Protect from Magic, Missiles, and Melee respectively.
    Protect = 0,
    Strength = 1,
    Attack = 2,
}

/// Combat stat index used by the fixed seven-stat frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum CombatStat {
    Attack = 0,
    Defence = 1,
    Strength = 2,
    Hitpoints = 3,
    Ranged = 4,
    Prayer = 5,
    Magic = 6,
}

impl CombatStat {
    pub const fn index(self) -> usize {
        self as usize
    }

    fn from_source(stat: &str) -> Option<Self> {
        match stat {
            "attack" => Some(Self::Attack),
            "defence" => Some(Self::Defence),
            "strength" => Some(Self::Strength),
            "hitpoints" => Some(Self::Hitpoints),
            "ranged" => Some(Self::Ranged),
            "prayer" => Some(Self::Prayer),
            "magic" => Some(Self::Magic),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StyleWhere {
    Attacker,
    Projectile,
    OnUs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpotanimStyle {
    pub style: StyleMask,
    pub where_: StyleWhere,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PotionKind {
    Prayer,
    SuperAttack,
    SuperStrength,
    SuperDefence,
    Ranging,
    Magic,
    Antifire,
}

impl PotionKind {
    const ALL: [Self; 7] = [
        Self::Prayer,
        Self::SuperAttack,
        Self::SuperStrength,
        Self::SuperDefence,
        Self::Ranging,
        Self::Magic,
        Self::Antifire,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Prayer => 0,
            Self::SuperAttack => 1,
            Self::SuperStrength => 2,
            Self::SuperDefence => 3,
            Self::Ranging => 4,
            Self::Magic => 5,
            Self::Antifire => 6,
        }
    }

    fn from_family(family: &str) -> Option<Self> {
        match family {
            "potion_prayerrestore" => Some(Self::Prayer),
            "potion_2attack" => Some(Self::SuperAttack),
            "potion_2strength" => Some(Self::SuperStrength),
            "potion_2defense" => Some(Self::SuperDefence),
            "potion_rangerspotion" => Some(Self::Ranging),
            "potion_1magic" => Some(Self::Magic),
            "potion_1antidragon" => Some(Self::Antifire),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PotionDose {
    pub id: i32,
    pub doses: u8,
    pub next_id: i32,
    pub next_doses: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PotionFamily {
    pub kind: PotionKind,
    /// Indexed by dose count minus one; unavailable selected stages remain `None`.
    pub doses: [Option<PotionDose>; 4],
    pub stat: Option<CombatStat>,
    pub constant: i32,
    pub percent: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoodFact {
    pub item_id: i32,
    pub heal: i32,
    pub eat_delay_arg: Option<i32>,
    pub skill_delay_arg: Option<i32>,
    pub message_delay: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CombatTab {
    Unarmed = 0,
    HeavySword = 1,
    Axe = 2,
    Blunt = 3,
    Pickaxe = 4,
    Scythe = 5,
    HackSword = 6,
    Spear = 7,
    Spiked = 8,
    StabSword = 9,
    Claw = 10,
    Polearm = 11,
    Bow = 12,
    Crossbow = 13,
    Thrown = 14,
    Staff = 15,
}

impl CombatTab {
    pub const fn code(self) -> u8 {
        self as u8
    }

    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Unarmed,
            1 => Self::HeavySword,
            2 => Self::Axe,
            3 => Self::Blunt,
            4 => Self::Pickaxe,
            5 => Self::Scythe,
            6 => Self::HackSword,
            7 => Self::Spear,
            8 => Self::Spiked,
            9 => Self::StabSword,
            10 => Self::Claw,
            11 => Self::Polearm,
            12 => Self::Bow,
            13 => Self::Crossbow,
            14 => Self::Thrown,
            15 => Self::Staff,
            _ => return None,
        })
    }
}

/// Selected button for a requested mode, including any XP-equivalent fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeleeModeChoice {
    pub button: i32,
    pub slot: u8,
    pub actual: MeleeMode,
    pub fallback: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WeaponStyle {
    pub obj_id: i32,
    pub style: u8,
    pub attackrate: u8,
    pub category: u8,
    pub tab: Option<CombatTab>,
}

#[derive(Clone, Copy, Debug)]
pub struct NpcFact<'a> {
    pub row: &'a NpcNameRow,
    pub attackable: bool,
}

/// A one-time index over the selected revision's immutable combat facts.
///
/// It owns only ids, timings, and source indices; large generated rows remain
/// shared through `selected`.
pub struct CombatTables {
    selected: Arc<SelectedGameData>,
    prayer_rows: [Option<usize>; 9],
    foods: Box<[FoodFact]>,
    potions: [Option<PotionFamily>; 7],
    melee_mode_rows: MeleeModeIndex,
    melee_mode_varp: Option<i32>,
}

impl CombatTables {
    pub fn build(selected: Arc<SelectedGameData>) -> Result<Arc<Self>, ActionError> {
        let prayers = selected.prayers();
        let prayer_names = [
            [
                "Protect from Magic",
                "Protect from Missiles",
                "Protect from Melee",
            ],
            [
                "Burst of Strength",
                "Superhuman Strength",
                "Ultimate Strength",
            ],
            [
                "Clarity of Thought",
                "Improved Reflexes",
                "Incredible Reflexes",
            ],
        ];
        let mut prayer_rows = [None; 9];
        for (role, names) in prayer_names.iter().enumerate() {
            for (tier, name) in names.iter().enumerate() {
                prayer_rows[role * PRAYER_TIERS + tier] = prayers
                    .iter()
                    .position(|row| row.name.eq_ignore_ascii_case(name));
            }
        }

        let consumption = selected.consumption_facts();

        let foods = build_foods(consumption)?;
        let potions = build_potions(&selected, consumption)?;
        validate_combat_indexes(&selected)?;
        let (melee_mode_rows, melee_mode_varp) = build_melee_mode_index(&selected)?;

        Ok(Arc::new(Self {
            selected,
            prayer_rows,
            foods: foods.into_boxed_slice(),
            potions,
            melee_mode_rows,
            melee_mode_varp,
        }))
    }

    /// Shared source facts; cloning the `Arc` does not copy the selected data.
    pub fn selected(&self) -> &Arc<SelectedGameData> {
        &self.selected
    }

    /// Selected NPC row by packed type id.
    pub fn npc(&self, id: i32) -> Option<&NpcNameRow> {
        self.selected.npc_name(id)
    }

    pub fn npc_fact(&self, id: i32) -> Option<NpcFact<'_>> {
        let row = self.npc(id)?;
        Some(NpcFact {
            row,
            attackable: row.ops.iter().any(|op| op == "Attack"),
        })
    }

    /// Whether this NPC has the exact attack option used by the server op.
    pub fn npc_attackable(&self, id: i32) -> bool {
        self.npc_fact(id).is_some_and(|fact| fact.attackable)
    }

    /// Server NPC attack interval; source facts default to four when absent.
    pub fn npc_rate(&self, row: &NpcNameRow) -> u8 {
        row.attackrate
            .and_then(|rate| u8::try_from(rate).ok())
            .unwrap_or(4)
    }

    /// Prayer selected by role and tier. Protect tiers are magic/missiles/melee;
    /// strength and attack tiers are low/middle/high.
    pub fn prayer(&self, role: PrayerRole, tier: u8) -> Option<&PrayerFact> {
        let tier = usize::from(tier);
        if tier >= PRAYER_TIERS {
            return None;
        }
        let index = self.prayer_rows[usize::from(role as u8) * PRAYER_TIERS + tier]?;
        self.selected.prayers().get(index)
    }

    pub fn food(&self, item_id: i32) -> Option<&FoodFact> {
        self.foods
            .binary_search_by_key(&item_id, |row| row.item_id)
            .ok()
            .map(|index| &self.foods[index])
    }

    /// Dose family for one of the supported PvM potion roles.
    pub fn potion(&self, kind: PotionKind) -> Option<&PotionFamily> {
        self.potions[kind.index()].as_ref()
    }

    /// Style mask for a selected-cache sequence id.
    pub fn style_seq(&self, seq_id: i32) -> Option<StyleMask> {
        let rows = self.selected.style_seqs();
        let index = rows.binary_search_by_key(&seq_id, |row| row.seq_id).ok()?;
        StyleMask::from_bits(rows[index].style)
    }

    /// Style and placement for a selected-cache spot animation id.
    pub fn style_spotanim(&self, spotanim_id: i32) -> Option<SpotanimStyle> {
        let rows = self.selected.style_spotanims();
        let index = rows
            .binary_search_by_key(&spotanim_id, |row| row.spotanim_id)
            .ok()?;
        spotanim_style(&rows[index])
    }

    pub fn weapon_style(&self, obj_id: i32) -> Option<WeaponStyle> {
        let rows = self.selected.weapon_styles();
        let index = rows.binary_search_by_key(&obj_id, |row| row.obj_id).ok()?;
        weapon_style(&rows[index])
    }

    /// Root interface ID for one tab in this selected content pin.
    pub fn combat_tab_root(&self, tab: CombatTab) -> Option<i32> {
        let rows = self.selected.combat_tabs();
        let index = rows.binary_search_by_key(&tab.code(), |row| row.tab).ok()?;
        Some(rows[index].root_id)
    }

    /// Packed `%com_mode` varp, when the selected facts resolve it.
    pub fn melee_mode_varp(&self) -> Option<i32> {
        self.melee_mode_varp
    }

    /// Exact mode first, then the closest offered mode by shared XP skills.
    /// No string parsing or allocation occurs on this lookup path.
    pub fn melee_mode(
        &self,
        tab: CombatTab,
        wanted: MeleeMode,
        observed_slot: Option<u8>,
    ) -> Option<MeleeModeChoice> {
        let slots = &self.melee_mode_rows[usize::from(tab.code())];
        let rows = self.selected.melee_modes();
        let observed_slot = observed_slot.filter(|slot| usize::from(*slot) < MELEE_MODE_SLOTS);
        let exact = observed_slot
            .and_then(|slot| slots[usize::from(slot)].map(|index| (slot, index)))
            .filter(|(_, index)| rows[*index].mode == wanted as u8)
            .or_else(|| {
                slots.iter().enumerate().find_map(|(slot, index)| {
                    let index = (*index)?;
                    (rows[index].mode == wanted as u8).then_some((slot as u8, index))
                })
            });
        if let Some((slot, index)) = exact {
            let row = &rows[index];
            return Some(MeleeModeChoice {
                button: row.button,
                slot,
                actual: MeleeMode::from_code(row.mode)?,
                fallback: false,
            });
        }

        let mut best: Option<(u32, bool, u8, usize, MeleeMode)> = None;
        for (slot, index) in slots.iter().enumerate() {
            let Some(index) = *index else {
                continue;
            };
            let row = &rows[index];
            let actual = MeleeMode::from_code(row.mode)?;
            let overlap = (wanted.xp_mask() & actual.xp_mask()).count_ones();
            if overlap == 0 {
                continue;
            }
            let slot = slot as u8;
            let preserves_slot = observed_slot == Some(slot);
            let replace = best.is_none_or(|(best_overlap, best_preserves, best_slot, _, _)| {
                overlap > best_overlap
                    || (overlap == best_overlap
                        && (preserves_slot && !best_preserves
                            || preserves_slot == best_preserves && slot < best_slot))
            });
            if replace {
                best = Some((overlap, preserves_slot, slot, index, actual));
            }
        }
        best.map(|(_, _, slot, index, actual)| MeleeModeChoice {
            button: rows[index].button,
            slot,
            actual,
            fallback: true,
        })
    }
}

fn validate_delay(delay: Option<i32>) -> bool {
    delay.is_none_or(|delay| delay >= 0)
}

fn validate_ordinary_eat_delay(delay: Option<i32>) -> bool {
    delay.is_none_or(|delay| (0..=2).contains(&delay))
}

fn build_foods(consumption: &[ConsumptionFact]) -> Result<Vec<FoodFact>, ActionError> {
    if consumption
        .iter()
        .any(|fact| !validate_ordinary_eat_delay(fact.eat_delay_arg))
    {
        return Err(unavailable("ordinary food fact has an invalid eat delay"));
    }

    let mut foods = Vec::new();
    for fact in consumption {
        let Some(heal) = fact.fixed_hp_heal() else {
            continue;
        };
        if fact.item.id < 0
            || !validate_delay(fact.eat_delay_arg)
            || !validate_delay(fact.skill_delay_arg)
            || !validate_delay(fact.message_delay)
        {
            return Err(unavailable("combat food fact has an invalid id or delay"));
        }
        foods.push(FoodFact {
            item_id: fact.item.id,
            heal,
            eat_delay_arg: fact.eat_delay_arg,
            skill_delay_arg: fact.skill_delay_arg,
            message_delay: fact.message_delay,
        });
    }
    foods.sort_unstable_by_key(|row| row.item_id);
    for pair in foods.windows(2) {
        if pair[0].item_id == pair[1].item_id {
            if pair[0] != pair[1] {
                return Err(unavailable("combat food item has conflicting source facts"));
            }
            return Err(unavailable("combat food item id is duplicated"));
        }
    }
    Ok(foods)
}

fn build_potions(
    selected: &SelectedGameData,
    consumption: &[ConsumptionFact],
) -> Result<[Option<PotionFamily>; 7], ActionError> {
    let mut doses: [[Option<PotionDose>; 4]; 7] = [[None; 4]; 7];
    let mut present = [false; 7];
    let mut effects: [Option<(CombatStat, i32, i32)>; 7] = [None; 7];
    for fact in consumption {
        let Some(kind) = fact
            .dose_family
            .as_deref()
            .and_then(PotionKind::from_family)
        else {
            continue;
        };
        let Some(count) = fact.dose_count.filter(|count| (1..=4).contains(count)) else {
            return Err(unavailable("combat potion has an invalid dose count"));
        };
        let Some(next_stage) = fact
            .next_stage
            .as_deref()
            .and_then(|alias| selected.item_by_alias(alias))
        else {
            return Err(unavailable("combat potion next stage is unresolved"));
        };
        if fact.item.id < 0
            || next_stage.id < 0
            || !validate_delay(fact.eat_delay_arg)
            || !validate_delay(fact.skill_delay_arg)
            || !validate_delay(fact.message_delay)
        {
            return Err(unavailable("combat potion fact has an invalid id or delay"));
        }
        let family_index = kind.index();
        if fact.stat_change.len() > 1 {
            return Err(unavailable("combat potion has multiple stat changes"));
        }
        if let Some(change) = fact.stat_change.first() {
            let Some(stat) = CombatStat::from_source(&change.stat) else {
                return Err(unavailable("combat potion has an unsupported stat change"));
            };
            let effect = (stat, change.base, change.percent);
            if effects[family_index].is_some_and(|existing| existing != effect) {
                return Err(unavailable(
                    "combat potion family has conflicting stat changes",
                ));
            }
            effects[family_index] = Some(effect);
        }
        let dose_slot = usize::from(count - 1);
        if doses[family_index][dose_slot].is_some() {
            return Err(unavailable("combat potion family has a duplicate dose"));
        }
        doses[family_index][dose_slot] = Some(PotionDose {
            id: fact.item.id,
            doses: count,
            next_id: next_stage.id,
            next_doses: count.saturating_sub(1),
        });
        present[family_index] = true;
    }

    let mut families = [None; 7];
    for kind in PotionKind::ALL {
        let index = kind.index();
        if !present[index] {
            continue;
        }
        for pair in doses[index].windows(2) {
            if let [Some(lower), Some(upper)] = pair {
                if upper.next_id != lower.id || upper.next_doses != lower.doses {
                    return Err(unavailable("combat potion dose stages do not form a chain"));
                }
            }
        }
        let (stat, constant, percent) = effects[index]
            .map_or((None, 0, 0), |(stat, constant, percent)| {
                (Some(stat), constant, percent)
            });
        families[index] = Some(PotionFamily {
            kind,
            doses: doses[index],
            stat,
            constant,
            percent,
        });
    }
    Ok(families)
}

fn validate_combat_indexes(selected: &SelectedGameData) -> Result<(), ActionError> {
    if let Some(facts) = selected.npc_names() {
        if facts.rows.windows(2).any(|pair| pair[0].id >= pair[1].id) {
            return Err(unavailable("combat NPC rows are not strictly sorted by id"));
        }
        if facts.rows.iter().any(|row| {
            row.id < 0
                || row
                    .attackrate
                    .is_some_and(|rate| !(0..=255).contains(&rate))
                || row.forced_max_hit.is_some_and(|hit| hit < 0)
        }) {
            return Err(unavailable(
                "combat NPC row has an invalid id, rate, or forced hit",
            ));
        }
    }
    if selected
        .style_seqs()
        .windows(2)
        .any(|pair| pair[0].seq_id >= pair[1].seq_id)
        || selected
            .style_spotanims()
            .windows(2)
            .any(|pair| pair[0].spotanim_id >= pair[1].spotanim_id)
        || selected
            .weapon_styles()
            .windows(2)
            .any(|pair| pair[0].obj_id >= pair[1].obj_id)
        || selected
            .combat_tabs()
            .windows(2)
            .any(|pair| pair[0].tab >= pair[1].tab)
    {
        return Err(unavailable(
            "combat style or tab rows are not strictly sorted",
        ));
    }
    let combat_tabs = selected.combat_tabs();
    if combat_tabs.iter().enumerate().any(|(index, row)| {
        CombatTab::from_code(row.tab).is_none()
            || row.root_id < 0
            || combat_tabs[..index]
                .iter()
                .any(|previous| previous.root_id == row.root_id)
    }) {
        return Err(unavailable("combat tab row has an invalid code or root"));
    }
    if selected.weapon_styles().iter().any(|row| match row.tab {
        Some(code) => match CombatTab::from_code(code) {
            Some(tab) => combat_tabs
                .binary_search_by_key(&tab.code(), |row| row.tab)
                .is_err(),
            None => true,
        },
        None => false,
    }) {
        return Err(unavailable("weapon style tab has no selected root mapping"));
    }
    if selected
        .style_seqs()
        .iter()
        .any(|row| StyleMask::from_bits(row.style).is_none())
        || selected
            .style_spotanims()
            .iter()
            .any(|row| spotanim_style(row).is_none())
        || selected
            .weapon_styles()
            .iter()
            .any(|row| weapon_style(row).is_none())
    {
        return Err(unavailable(
            "combat style row has an unsupported mask or value",
        ));
    }
    Ok(())
}
fn build_melee_mode_index(
    selected: &SelectedGameData,
) -> Result<(MeleeModeIndex, Option<i32>), ActionError> {
    let rows = selected.melee_modes();
    if rows.windows(2).any(|pair| {
        pair[0].tab > pair[1].tab || (pair[0].tab == pair[1].tab && pair[0].slot >= pair[1].slot)
    }) {
        return Err(unavailable(
            "combat melee mode rows are not strictly sorted by tab and slot",
        ));
    }
    let mut indexes = [[None; MELEE_MODE_SLOTS]; COMBAT_TAB_COUNT];
    for (index, row) in rows.iter().enumerate() {
        let Some(tab) = CombatTab::from_code(row.tab) else {
            return Err(unavailable("combat melee mode has an unknown tab"));
        };
        let slot = usize::from(row.slot);
        if slot >= MELEE_MODE_SLOTS
            || row.button < 0
            || MeleeMode::from_code(row.mode).is_none()
            || selected
                .combat_tabs()
                .binary_search_by_key(&row.tab, |fact| fact.tab)
                .is_err()
        {
            return Err(unavailable(
                "combat melee mode has an invalid slot, mode, button, or root",
            ));
        }
        if indexes[usize::from(tab.code())][slot]
            .replace(index)
            .is_some()
        {
            return Err(unavailable("combat melee mode slot is duplicated"));
        }
    }
    let varp = selected.melee_mode_varp();
    if varp.is_some_and(|id| id < 0) {
        return Err(unavailable("combat melee mode varp is invalid"));
    }
    Ok((indexes, varp))
}

fn spotanim_style(row: &StyleSpotanimFact) -> Option<SpotanimStyle> {
    let style = StyleMask::from_bits(row.style)?;
    let where_ = match row.location.as_str() {
        "attacker" => StyleWhere::Attacker,
        "projectile" => StyleWhere::Projectile,
        "on_us" => StyleWhere::OnUs,
        _ => return None,
    };
    Some(SpotanimStyle { style, where_ })
}

fn weapon_style(row: &WeaponStyleFact) -> Option<WeaponStyle> {
    StyleMask::from_bits(row.style)?;
    if row.category > 5 {
        return None;
    }
    Some(WeaponStyle {
        obj_id: row.obj_id,
        style: row.style,
        attackrate: row.attackrate,
        category: row.category,
        tab: match row.tab {
            Some(code) => Some(CombatTab::from_code(code)?),
            None => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn prayer(name: &str, level: i32) -> Value {
        json!({
            "name": name,
            "level": level,
            "source_row": "combat_test",
            "prayer_constant": "test",
            "button_com": level,
            "com_alias": "test",
            "varp": level,
            "varp_alias": "test"
        })
    }

    fn item(alias: &str, id: i32) -> Value {
        json!({
            "alias": alias,
            "id": id,
            "name": alias,
            "cost": 0,
            "stackable": false,
            "members": false,
            "certificate_link": -1,
            "certificate_template": -1,
            "wear_position": -1,
            "wear_position_2": -1,
            "wear_position_3": -1,
            "tradeable": true,
            "stack_variant": false
        })
    }

    fn consume(
        alias: &str,
        id: i32,
        name: &str,
        heal: Option<i32>,
        delays: [Option<i32>; 3],
        dose: Option<(&str, u8, &str)>,
    ) -> Value {
        let [eat_delay_arg, skill_delay_arg, message_delay] = delays;
        let (dose_family, dose_count, next_stage) = dose
            .map_or((None, None, None), |(family, count, next)| {
                (Some(family), Some(count), Some(next))
            });
        let stat_heal = heal.map_or_else(Vec::new, |base| {
            vec![json!({"stat":"hitpoints", "base":base, "percent":0})]
        });
        json!({
            "item": {"alias":alias, "id":id, "name":name},
            "source_row":"test",
            "source_file": if heal.is_some() {"consume_normal.dbrow"} else {"consume_effects.dbrow"},
            "effect":"test",
            "eat_delay_arg":eat_delay_arg,
            "skill_delay_arg":skill_delay_arg,
            "message_delay":message_delay,
            "stat_change":[],
            "stat_heal":stat_heal,
            "heal_energy":[],
            "qualification":if heal.is_some() {"fixed_hp_heal"} else {"not_fixed_hp_heal"},
            "next_stage":next_stage,
            "dose_family":dose_family,
            "dose_count":dose_count
        })
    }

    fn selected_fixture_value() -> Value {
        let prayer_rows = [
            ("Protect from Magic", 37),
            ("Protect from Missiles", 40),
            ("Protect from Melee", 43),
            ("Burst of Strength", 1),
            ("Superhuman Strength", 13),
            ("Ultimate Strength", 31),
            ("Clarity of Thought", 1),
            ("Improved Reflexes", 16),
            ("Incredible Reflexes", 34),
        ];
        let prayers: Vec<_> = prayer_rows
            .iter()
            .map(|(name, level)| prayer(name, *level))
            .collect();
        let mut items = vec![
            item("bread", 100),
            item("karambwan", 101),
            item("empty_vial", 299),
        ];
        let mut consumption = vec![
            consume(
                "bread",
                100,
                "Bread",
                Some(4),
                [Some(2), Some(3), None],
                None,
            ),
            consume(
                "karambwan",
                101,
                "Cooked karambwan",
                Some(18),
                [None, None, Some(2)],
                None,
            ),
        ];
        for count in 1..=4u8 {
            let alias = format!("prayer_restore_{count}");
            let next = if count == 1 {
                "empty_vial".to_string()
            } else {
                format!("prayer_restore_{}", count - 1)
            };
            let id = 200 + i32::from(count);
            items.push(item(&alias, id));
            consumption.push(consume(
                &alias,
                id,
                &alias,
                None,
                [None, None, Some(1)],
                Some(("potion_prayerrestore", count, &next)),
            ));
        }
        for count in 1..=4u8 {
            let alias = format!("super_attack_{count}");
            let next = if count == 1 {
                "empty_vial".to_string()
            } else {
                format!("super_attack_{}", count - 1)
            };
            let id = 300 + i32::from(count);
            items.push(item(&alias, id));
            let mut fact = consume(
                &alias,
                id,
                &alias,
                None,
                [None, None, Some(1)],
                Some(("potion_2attack", count, &next)),
            );
            if count == 4 {
                fact["stat_change"] = json!([{"stat":"attack","base":5,"percent":15}]);
            }
            consumption.push(fact);
        }
        let item_rows = items;
        let data = json!({
            "schema_version":4,
            "revision":289,
            "provenance":{
                "cache_identity":{"cache_id":"test"},
                "inputs":[],"content_inputs":[],"decoder_sources":[]
            },
            "items":item_rows,
            "consumption":consumption,
            "pickpocket":[],
            "prayers":prayers,
            "npc_names":{"rows":[
                {
                    "id":7,"config":"test_npc","display":"Test NPC","ops":["Attack"],
                    "size":1,"wanderrange":0,"maxrange":1,"attackrange":1,"huntrange":1,
                    "vislevel":0,"hitpoints":30,"damagetype":null,
                    "ap_attack":true,"attack_kind":null,"forced_max_hit":null,
                    "dragonfire":null,"attackrate":6,"bespoke":false
                },
                {
                    "id":8,"config":"not_attackable","display":"Not attackable","ops":["attack"],
                    "size":1,"wanderrange":0,"maxrange":0,"attackrange":0,"huntrange":0,
                    "vislevel":0,"hitpoints":10,"damagetype":null
                }
            ]},
            "style_seqs":[{"seq_id":10,"style":1},{"seq_id":11,"style":6}],
            "style_spotanims":[{"spotanim_id":20,"style":8,"where":"projectile"}],
            "combat_tabs":[
                {"tab":0,"root_id":904},{"tab":1,"root_id":901},{"tab":3,"root_id":903},
                {"tab":6,"root_id":902},{"tab":7,"root_id":905}
            ],
            "melee_modes":[
                {"tab":1,"slot":0,"mode":0,"button":1010},
                {"tab":1,"slot":1,"mode":1,"button":1011},
                {"tab":1,"slot":2,"mode":1,"button":1012},
                {"tab":1,"slot":3,"mode":2,"button":1013},
                {"tab":3,"slot":0,"mode":2,"button":1030},
                {"tab":6,"slot":0,"mode":0,"button":1060},
                {"tab":6,"slot":1,"mode":1,"button":1061},
                {"tab":6,"slot":2,"mode":3,"button":1062},
                {"tab":6,"slot":3,"mode":2,"button":1063},
                {"tab":7,"slot":0,"mode":3,"button":1070},
                {"tab":7,"slot":1,"mode":3,"button":1071},
                {"tab":7,"slot":2,"mode":3,"button":1072},
                {"tab":7,"slot":3,"mode":2,"button":1073}
            ],
            "melee_mode_varp":43,
            "weapon_styles":[
                {"obj_id":30,"style":1,"attackrate":5,"category":0,"tab":1},
                {"obj_id":31,"style":1,"attackrate":4,"category":1,"tab":null}
            ]
        });
        data
    }

    fn selected_fixture() -> Arc<SelectedGameData> {
        Arc::new(
            serde_json::from_value(selected_fixture_value())
                .expect("selected combat facts deserialize"),
        )
    }

    #[test]
    fn selected_tables_resolve_combat_facts_without_per_lookup_copies() {
        let tables = CombatTables::build(selected_fixture()).expect("valid combat facts");
        assert_eq!(
            tables.food(100),
            Some(&FoodFact {
                item_id: 100,
                heal: 4,
                eat_delay_arg: Some(2),
                skill_delay_arg: Some(3),
                message_delay: None,
            })
        );
        assert_eq!(tables.food(101).unwrap().message_delay, Some(2));
        assert_eq!(tables.food(999), None);
        assert_eq!(
            tables
                .selected()
                .consumable("karambwan")
                .unwrap()
                .eat_delay_arg,
            None
        );

        let potion = tables.potion(PotionKind::Prayer).expect("prayer family");
        assert_eq!(
            potion.doses.map(|dose| dose.map(|dose| dose.id)),
            [Some(201), Some(202), Some(203), Some(204)]
        );
        assert_eq!(
            potion.doses.map(|dose| dose.map(|dose| dose.next_id)),
            [Some(299), Some(201), Some(202), Some(203)]
        );
        assert_eq!(
            potion.doses.map(|dose| dose.map(|dose| dose.doses)),
            [Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(
            potion.doses.map(|dose| dose.map(|dose| dose.next_doses)),
            [Some(0), Some(1), Some(2), Some(3)]
        );
        assert_eq!((potion.stat, potion.constant, potion.percent), (None, 0, 0));
        let boost = tables
            .potion(PotionKind::SuperAttack)
            .expect("attack family");
        assert_eq!(
            (boost.stat, boost.constant, boost.percent),
            (Some(CombatStat::Attack), 5, 15)
        );
        assert_eq!(tables.potion(PotionKind::Antifire), None);

        assert_eq!(tables.npc_fact(7).map(|fact| fact.attackable), Some(true));
        assert_eq!(tables.npc_fact(8).map(|fact| fact.attackable), Some(false));
        assert_eq!(tables.npc(8).map(|row| tables.npc_rate(row)), Some(4));
        assert_eq!(tables.npc(7).map(|row| tables.npc_rate(row)), Some(6));
        assert!(tables.npc(999).is_none());
        assert_eq!(
            tables.prayer(PrayerRole::Protect, 0).unwrap().name,
            "Protect from Magic"
        );
        assert_eq!(
            tables.prayer(PrayerRole::Strength, 2).unwrap().name,
            "Ultimate Strength"
        );
        assert!(tables.prayer(PrayerRole::Attack, 3).is_none());

        assert_eq!(tables.style_seq(10), Some(StyleMask::MELEE));
        assert!(tables
            .style_seq(11)
            .unwrap()
            .contains(StyleMask::RANGED | StyleMask::MAGIC));
        assert_eq!(tables.style_seq(999), None);
        assert_eq!(
            tables.style_spotanim(20).unwrap().where_,
            StyleWhere::Projectile
        );
        assert_eq!(
            tables.weapon_style(30).unwrap().tab,
            Some(CombatTab::HeavySword)
        );
        assert_eq!(tables.weapon_style(31).unwrap().tab, None);
        assert_eq!(tables.weapon_style(31).unwrap().category, 1);
        assert_eq!(tables.combat_tab_root(CombatTab::HeavySword), Some(901));
        assert_eq!(tables.combat_tab_root(CombatTab::Unarmed), Some(904));
        assert_eq!(tables.combat_tab_root(CombatTab::Axe), None);
        assert_eq!(CombatStat::Magic.index(), 6);
        assert_eq!(tables.melee_mode_varp(), Some(43));
    }

    #[test]
    fn selected_foods_use_per_item_heals_when_names_collide() {
        let mut data = selected_fixture_value();
        let consumption = data["consumption"].as_array_mut().unwrap();
        consumption.push(consume(
            "cooked_karambwan",
            3144,
            "Cooked karambwan",
            Some(18),
            [None, None, Some(2)],
            None,
        ));
        let mut harmful = consume(
            "harmful_karambwan",
            3142,
            "Cooked karambwan",
            None,
            [None, None, Some(2)],
            None,
        );
        harmful["stat_change"] = json!([{"stat":"hitpoints","base":-5,"percent":0}]);
        consumption.push(harmful);

        let selected = Arc::new(serde_json::from_value(data).unwrap());
        let tables = CombatTables::build(selected).expect("valid food remains indexed");
        assert_eq!(
            tables.food(3144),
            Some(&FoodFact {
                item_id: 3144,
                heal: 18,
                eat_delay_arg: None,
                skill_delay_arg: None,
                message_delay: Some(2),
            })
        );
        assert_eq!(tables.food(3142), None);
    }

    #[test]
    fn selected_tables_reject_ordinary_eat_delay_above_two_on_non_healing_rows() {
        let mut data = selected_fixture_value();
        data["consumption"].as_array_mut().unwrap().push(consume(
            "harmful_food",
            3142,
            "Harmful food",
            None,
            [Some(3), None, None],
            None,
        ));
        let selected = Arc::new(serde_json::from_value(data).unwrap());
        assert!(
            CombatTables::build(selected).is_err(),
            "all ordinary eat-delay arguments must remain within 0..=2"
        );
    }

    #[test]
    fn melee_mode_lookup_prefers_exact_then_best_xp_overlap() {
        let tables = CombatTables::build(selected_fixture()).expect("valid mode facts");
        let exact_duplicate = tables
            .melee_mode(CombatTab::HeavySword, MeleeMode::Aggressive, Some(2))
            .unwrap();
        assert_eq!(
            exact_duplicate,
            MeleeModeChoice {
                button: 1012,
                slot: 2,
                actual: MeleeMode::Aggressive,
                fallback: false,
            },
        );
        let exact_ignores_unmatched_observed = tables
            .melee_mode(CombatTab::HeavySword, MeleeMode::Accurate, Some(1))
            .unwrap();
        assert_eq!(
            exact_ignores_unmatched_observed,
            MeleeModeChoice {
                button: 1010,
                slot: 0,
                actual: MeleeMode::Accurate,
                fallback: false,
            },
        );
        let current_tie = tables
            .melee_mode(CombatTab::HeavySword, MeleeMode::Controlled, Some(2))
            .unwrap();
        assert_eq!(
            (current_tie.slot, current_tie.actual),
            (2, MeleeMode::Aggressive)
        );
        assert!(current_tie.fallback);
        let lowest_tie = tables
            .melee_mode(CombatTab::HeavySword, MeleeMode::Controlled, None)
            .unwrap();
        assert_eq!(
            (lowest_tie.slot, lowest_tie.actual),
            (0, MeleeMode::Accurate)
        );
        assert!(lowest_tie.fallback);
        let accurate_fallback = tables
            .melee_mode(CombatTab::Spear, MeleeMode::Accurate, Some(1))
            .unwrap();
        assert_eq!(
            (accurate_fallback.button, accurate_fallback.slot),
            (1071, 1)
        );
        assert_eq!(accurate_fallback.actual, MeleeMode::Controlled);
        assert!(accurate_fallback.fallback);

        let spear_fallback = tables
            .melee_mode(CombatTab::Spear, MeleeMode::Aggressive, Some(1))
            .unwrap();
        assert_eq!((spear_fallback.button, spear_fallback.slot), (1071, 1));
        assert_eq!(spear_fallback.actual, MeleeMode::Controlled);
        assert!(spear_fallback.fallback);
        let controlled_exact = tables
            .melee_mode(CombatTab::Spear, MeleeMode::Controlled, Some(2))
            .unwrap();
        assert_eq!(
            (controlled_exact.slot, controlled_exact.actual),
            (2, MeleeMode::Controlled)
        );
        assert!(!controlled_exact.fallback);
        assert_eq!(
            tables.melee_mode(CombatTab::Blunt, MeleeMode::Accurate, None),
            None,
            "a defensive-only option shares no XP with accurate",
        );
    }

    #[test]
    fn melee_mode_lookup_fails_closed_for_missing_sparse_and_invalid_facts() {
        let mut missing = selected_fixture_value();
        missing.as_object_mut().unwrap().remove("melee_modes");
        missing.as_object_mut().unwrap().remove("melee_mode_varp");
        let tables = CombatTables::build(Arc::new(serde_json::from_value(missing).unwrap()))
            .expect("legacy combat facts without mode controls remain usable");
        assert_eq!(tables.melee_mode_varp(), None);
        assert_eq!(
            tables.melee_mode(CombatTab::HeavySword, MeleeMode::Accurate, None),
            None
        );

        let mut sparse = selected_fixture_value();
        sparse["melee_modes"]
            .as_array_mut()
            .unwrap()
            .retain(|row| row["tab"] != 1 || row["slot"] != 1);
        let sparse = CombatTables::build(Arc::new(serde_json::from_value(sparse).unwrap()))
            .expect("sparse source choices remain usable");
        assert_eq!(
            sparse
                .melee_mode(CombatTab::HeavySword, MeleeMode::Aggressive, None)
                .map(|choice| (choice.slot, choice.button)),
            Some((2, 1012)),
        );

        let mut invalid = selected_fixture_value();
        invalid["melee_modes"][0]["mode"] = json!(4);
        let invalid = Arc::new(serde_json::from_value(invalid).unwrap());
        assert!(CombatTables::build(invalid).is_err());
    }

    #[test]
    fn selected_tables_reject_unknown_combat_tab_discriminants() {
        let mut data = selected_fixture_value();
        data["combat_tabs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"tab":16,"root_id":910}));
        let selected = Arc::new(serde_json::from_value(data).unwrap());
        assert!(CombatTables::build(selected).is_err());
    }

    #[test]
    fn selected_tables_reject_weapon_tabs_without_root_mappings() {
        let mut data = selected_fixture_value();
        data["combat_tabs"]
            .as_array_mut()
            .unwrap()
            .retain(|row| row["tab"] != 1);
        let selected = Arc::new(serde_json::from_value(data).unwrap());
        assert!(CombatTables::build(selected).is_err());
    }

    #[test]
    fn selected_tables_preserve_sparse_potion_stages() {
        let mut data = selected_fixture_value();
        data["consumption"]
            .as_array_mut()
            .unwrap()
            .retain(|row| row["dose_count"] != 3);
        let selected = Arc::new(serde_json::from_value(data).unwrap());
        let tables = CombatTables::build(selected).expect("sparse stage facts remain usable");
        let potion = tables.potion(PotionKind::Prayer).expect("family retained");
        assert_eq!(
            potion.doses.map(|dose| dose.map(|dose| dose.id)),
            [Some(201), Some(202), None, Some(204)]
        );
        assert_eq!(
            potion.doses.map(|dose| dose.map(|dose| dose.next_id)),
            [Some(299), Some(201), None, Some(203)]
        );
    }
}
