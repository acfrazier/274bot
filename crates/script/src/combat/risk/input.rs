use super::super::arbiter;
use super::super::frame::Frame;
use super::super::request::{ActorKind, ActorRef};
use super::super::tables::{CombatStat, CombatTables, PotionKind, StyleMask};
use super::super::threats::{ProjectileFamily, StyleObs, ThreatSet};
use api::condition::PoisonState;
use api::snapshot::{StatView, VarpView, WorldTile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Survivable,
    Unsurvivable,
    Unknown(UnknownWhy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnknownWhy {
    /// An engaged zone's kind is outside the bounded risk model, for this class.
    #[error("unsupported risk kind: {0:?}")]
    Kind(super::UnknownKind),
    #[error("already-engaged threat is unsupported")]
    AlreadyEngaged,
    #[error("zone facts are unavailable")]
    NoZoneTable,
    #[error("required combat facts are unavailable")]
    MissingFacts,
    #[error("player input is locked")]
    InputLock,
    #[error("selected poison facts are missing or invalid")]
    Poison,
    #[error("risk model bounds were exceeded")]
    Overflow,
    /// The route, plan, zone key or exemption belongs to another nav pack,
    /// or names a row this pack does not have (`nav_planner::contract` refused it).
    #[error("navigation identities do not match")]
    NavIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Melee,
    Ranged,
    Unknown,
}

impl From<Style> for super::super::threats::StyleObs {
    fn from(style: Style) -> Self {
        match style {
            Style::Melee => Self::Melee,
            Style::Ranged => Self::Ranged,
            Style::Unknown => Self::Unknown,
        }
    }
}

/// A copied live threat row. `u16::MAX` means there is no attributed pending
/// impact; `due()` keeps the sentinel out of callers' tick arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveRow {
    pub actor: ActorRef,
    pub ident: i32,
    pub max_hit: u8,
    pub rate: u8,
    pub due_tick: u16,
}

impl LiveRow {
    const EMPTY: Self = Self {
        actor: ActorRef {
            kind: ActorKind::Npc,
            index: 0,
        },
        ident: -1,
        max_hit: 0,
        rate: 0,
        due_tick: u16::MAX,
    };

    #[inline]
    pub const fn due(self) -> Option<u16> {
        if self.due_tick == u16::MAX {
            None
        } else {
            Some(self.due_tick)
        }
    }
}

/// The fixed, copied inputs to one route estimate. No view rows or references
/// escape the capture call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskInput {
    pub pos: WorldTile,
    pub live: [LiveRow; 4],
    pub food_ids: [i32; 6],
    pub food_counts: [u8; 6],
    pub poison: PoisonState,
    pub tick: u16,
    pub hp: u8,
    pub hp_max: u8,
    pub prayer: u8,
    pub prayer_base: u8,
    pub combat: Option<u8>,
    pub doses_prayer: u8,
    pub live_len: u8,
    pub food_len: u8,
    pub free_slots: u8,
    pub defence: u8,
    pub worn_defence: [i16; 4],
    pub style_def: u8,
    pub active_protect: Style,
    flags: u16,
}

const MAP_MEMBERS: u16 = 1 << 0;
const RUN: u16 = 1 << 1;
const OTHER_PRAYERS_ON: u16 = 1 << 2;
const OFF_DEBT: u16 = 1 << 3;
const WORN_SHIELD: u16 = 1 << 4;
const MORE_FOOD: u16 = 1 << 5;
const MISSING_FACTS: u16 = 1 << 6;
const UNATTRIBUTED: u16 = 1 << 7;
const INPUT_LOCKED: u16 = 1 << 8;
const OVERFLOW: u16 = 1 << 9;

impl Default for RiskInput {
    fn default() -> Self {
        Self {
            pos: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            live: [LiveRow::EMPTY; 4],
            food_ids: [0; 6],
            food_counts: [0; 6],
            poison: PoisonState::Unknown { since: 0 },
            tick: 0,
            hp: 0,
            hp_max: 0,
            prayer: 0,
            prayer_base: 0,
            combat: None,
            doses_prayer: 0,
            live_len: 0,
            food_len: 0,
            free_slots: 0,
            defence: 0,
            worn_defence: [0; 4],
            style_def: 0,
            active_protect: Style::Unknown,
            flags: MISSING_FACTS | UNATTRIBUTED | INPUT_LOCKED | OVERFLOW,
        }
    }
}

impl RiskInput {
    #[inline]
    pub const fn map_members(&self) -> bool {
        self.flags & MAP_MEMBERS != 0
    }
    #[inline]
    pub fn set_map_members(&mut self, value: bool) {
        self.set_flag(MAP_MEMBERS, value);
    }
    #[inline]
    pub const fn run(&self) -> bool {
        self.flags & RUN != 0
    }
    #[inline]
    pub fn set_run(&mut self, value: bool) {
        self.set_flag(RUN, value);
    }
    #[inline]
    pub const fn other_prayers_on(&self) -> bool {
        self.flags & OTHER_PRAYERS_ON != 0
    }
    #[inline]
    pub fn set_other_prayers_on(&mut self, value: bool) {
        self.set_flag(OTHER_PRAYERS_ON, value);
    }
    #[inline]
    pub const fn off_debt(&self) -> bool {
        self.flags & OFF_DEBT != 0
    }
    #[inline]
    pub fn set_off_debt(&mut self, value: bool) {
        self.set_flag(OFF_DEBT, value);
    }
    #[inline]
    pub const fn worn_shield(&self) -> bool {
        self.flags & WORN_SHIELD != 0
    }
    #[inline]
    pub fn set_worn_shield(&mut self, value: bool) {
        self.set_flag(WORN_SHIELD, value);
    }
    #[inline]
    pub const fn more_food(&self) -> bool {
        self.flags & MORE_FOOD != 0
    }
    #[inline]
    pub fn set_more_food(&mut self, value: bool) {
        self.set_flag(MORE_FOOD, value);
    }
    #[inline]
    pub const fn missing_facts(&self) -> bool {
        self.flags & MISSING_FACTS != 0
    }
    #[inline]
    pub fn set_missing_facts(&mut self, value: bool) {
        self.set_flag(MISSING_FACTS, value);
    }
    #[inline]
    pub const fn unattributed(&self) -> bool {
        self.flags & UNATTRIBUTED != 0
    }
    #[inline]
    pub fn set_unattributed(&mut self, value: bool) {
        self.set_flag(UNATTRIBUTED, value);
    }
    #[inline]
    pub const fn input_locked(&self) -> bool {
        self.flags & INPUT_LOCKED != 0
    }
    #[inline]
    pub fn set_input_locked(&mut self, value: bool) {
        self.set_flag(INPUT_LOCKED, value);
    }
    #[inline]
    pub const fn overflow(&self) -> bool {
        self.flags & OVERFLOW != 0
    }
    #[inline]
    pub fn set_overflow(&mut self, value: bool) {
        self.set_flag(OVERFLOW, value);
    }
    #[inline]
    fn set_flag(&mut self, flag: u16, value: bool) {
        self.flags = if value {
            self.flags | flag
        } else {
            self.flags & !flag
        };
    }

    /// Copy the facts needed for one assessment from a complete combat frame.
    /// The caller overlays any slot-owned input-lock state after capture.
    pub fn capture(
        frame: &Frame<'_>,
        tables: &CombatTables,
        threats: &ThreatSet,
        poison: PoisonState,
        off_debt: bool,
    ) -> Self {
        let mut input = Self {
            pos: frame.here,
            tick: frame.tick,
            poison,
            flags: 0,
            ..Self::default()
        };
        input.set_map_members(frame.world.members);
        input.set_off_debt(off_debt);
        input.set_input_locked(false);
        input.set_missing_facts(false);
        input.set_unattributed(threats.has_unattributed_event(frame.tick));
        input.set_overflow(threats.has_overflow(frame.tick));

        if let Some(hp) = unique_stat(frame.stats, CombatStat::Hitpoints.index() as i32) {
            input.hp = compact_u8(hp.effective, &mut input);
            input.hp_max = compact_u8(hp.base, &mut input);
            if input.hp == 0 || input.hp_max == 0 {
                input.set_missing_facts(true);
            }
        } else {
            input.set_missing_facts(true);
        }

        if let Some(defence) = unique_stat(frame.stats, CombatStat::Defence.index() as i32) {
            let effective = compact_u8(defence.effective, &mut input);
            let base = compact_u8(defence.base, &mut input);
            input.defence = effective.min(base);
        } else {
            input.set_missing_facts(true);
        }

        if let Some(prayer) = unique_stat(frame.stats, CombatStat::Prayer.index() as i32) {
            input.prayer = compact_u8(prayer.effective, &mut input);
            input.prayer_base = compact_u8(prayer.base, &mut input);
            if input.prayer_base == 0 {
                input.set_missing_facts(true);
            }
        } else {
            input.set_missing_facts(true);
        }

        input.combat = match u8::try_from(frame.local.player.combat_level) {
            Ok(level) if level > 0 => Some(level),
            Ok(_) => {
                input.set_missing_facts(true);
                None
            }
            Err(_) if frame.local.player.combat_level > i32::from(u8::MAX) => {
                input.set_missing_facts(true);
                input.set_overflow(true);
                None
            }
            Err(_) => {
                input.set_missing_facts(true);
                None
            }
        };

        let run = match unique_varp(frame.varps, 173) {
            Some(0) => false,
            Some(1) => true,
            Some(_) | None => {
                input.set_missing_facts(true);
                false
            }
        };
        input.set_run(run);
        input.set_other_prayers_on(frame.prayers.iter().any(|active| *active));

        match tables.selected().item_by_alias("antidragonbreathshield") {
            Some(shield) => input.set_worn_shield(
                frame
                    .equipment
                    .iter()
                    .any(|item| item.slot == 5 && item.def.id == shield.id),
            ),
            None => input.set_missing_facts(true),
        }

        capture_worn_defence(frame, tables, &mut input);
        input.style_def = capture_style_def(frame, tables);
        input.active_protect = match super::super::select::active_protect(frame, tables) {
            Some(StyleObs::Melee) => Style::Melee,
            Some(StyleObs::Ranged) => Style::Ranged,
            Some(StyleObs::Magic | StyleObs::Dragonfire | StyleObs::Unknown) | None => {
                Style::Unknown
            }
        };

        if tables.potion(PotionKind::Prayer).is_some() {
            let doses = arbiter::doses(frame, tables, PotionKind::Prayer);
            if doses < 0 {
                input.set_missing_facts(true);
            } else if doses > i16::from(u8::MAX) {
                input.doses_prayer = u8::MAX;
                input.set_overflow(true);
            } else {
                input.doses_prayer = u8::try_from(doses).unwrap_or(u8::MAX);
            }
        } else {
            input.set_missing_facts(true);
        }

        capture_inventory(frame, tables, &mut input);
        capture_live(frame, threats, &mut input);
        input
    }

    /// Iterate the retained food kinds and their actual copied counts.
    #[inline]
    pub fn food_iter(&self) -> impl Iterator<Item = (i32, u8)> + '_ {
        self.food_ids
            .iter()
            .copied()
            .zip(self.food_counts.iter().copied())
            .take(usize::from(self.food_len.min(6)))
            .filter(|(_, count)| *count != 0)
    }

    #[inline]
    pub fn live_iter(&self) -> impl Iterator<Item = &LiveRow> {
        self.live.iter().take(usize::from(self.live_len.min(4)))
    }
}

const _: () = assert!(std::mem::size_of::<RiskInput>() <= 128);

fn unique_stat(stats: &[StatView], index: i32) -> Option<&StatView> {
    let mut matches = stats.iter().filter(|row| row.index == index);
    let row = matches.next()?;
    matches.next().is_none().then_some(row)
}

fn unique_varp(varps: &[VarpView], index: i32) -> Option<i32> {
    let mut matches = varps.iter().filter(|row| row.index == index);
    let value = matches.next()?.value;
    matches.next().is_none().then_some(value)
}

fn compact_u8(value: i32, input: &mut RiskInput) -> u8 {
    match u8::try_from(value) {
        Ok(value) => value,
        Err(_) if value > i32::from(u8::MAX) => {
            input.set_missing_facts(true);
            input.set_overflow(true);
            u8::MAX
        }
        Err(_) => {
            input.set_missing_facts(true);
            0
        }
    }
}

fn capture_worn_defence(frame: &Frame<'_>, tables: &CombatTables, input: &mut RiskInput) {
    let selected = tables.selected();
    for item in frame.equipment {
        let Some(bonus) = selected.item_combat_bonus(item.def.id) else {
            input.set_missing_facts(true);
            continue;
        };
        checked_add_worn_defence(input, 0, bonus.stabdefence);
        checked_add_worn_defence(input, 1, bonus.slashdefence);
        checked_add_worn_defence(input, 2, bonus.crushdefence);
        checked_add_worn_defence(input, 3, bonus.rangedefence);
    }
}

fn checked_add_worn_defence(input: &mut RiskInput, index: usize, bonus: i16) {
    match input.worn_defence[index].checked_add(bonus) {
        Some(total) => input.worn_defence[index] = total,
        None => {
            input.set_missing_facts(true);
            input.set_overflow(true);
        }
    }
}

fn capture_style_def(frame: &Frame<'_>, tables: &CombatTables) -> u8 {
    let Some(weapon) = frame.equipment.iter().find(|item| item.slot == 3) else {
        return 0;
    };
    let selected = tables.selected();
    let Some(style) = tables.weapon_style(weapon.def.id) else {
        return 0;
    };
    let ranged = style.style & StyleMask::RANGED.bits() != 0;
    let varp = if ranged {
        selected.ranged_mode_varp()
    } else {
        tables.melee_mode_varp()
    };
    let Some(varp) = varp else {
        return 0;
    };
    let Some(slot) = unique_varp(frame.varps, varp).and_then(|value| u8::try_from(value).ok())
    else {
        return 0;
    };
    let Some(tab) = style.tab else {
        return 0;
    };
    let Some(style_fact) = tables.weapon_style_slot(weapon.def.id, slot) else {
        return 0;
    };
    let tab = tab.code();
    let mode_available = if ranged {
        selected
            .ranged_modes()
            .iter()
            .any(|row| row.tab == tab && row.slot == slot)
    } else {
        selected
            .melee_modes()
            .iter()
            .any(|row| row.tab == tab && row.slot == slot)
    };
    if mode_available {
        style_fact.style_bonus.2
    } else {
        0
    }
}

#[derive(Clone, Copy)]
struct FoodCandidate {
    id: i32,
    count: u8,
    heal: i32,
}

impl FoodCandidate {
    const EMPTY: Self = Self {
        id: 0,
        count: 0,
        heal: i32::MIN,
    };
}

fn capture_inventory(frame: &Frame<'_>, tables: &CombatTables, input: &mut RiskInput) {
    const INVENTORY_SLOTS: usize = 28;
    let mut occupied = [false; INVENTORY_SLOTS];
    for item in frame.inventory {
        if item.count < 0 {
            input.set_missing_facts(true);
            continue;
        }
        if item.count == 0 {
            continue;
        }
        let Ok(slot) = usize::try_from(item.slot) else {
            input.set_missing_facts(true);
            input.set_overflow(true);
            continue;
        };
        let Some(used) = occupied.get_mut(slot) else {
            input.set_missing_facts(true);
            input.set_overflow(true);
            continue;
        };
        if *used {
            input.set_missing_facts(true);
            input.set_overflow(true);
        }
        *used = true;
    }
    let occupied_count = occupied.iter().filter(|used| **used).count();
    input.free_slots = u8::try_from(INVENTORY_SLOTS - occupied_count).unwrap_or(0);

    let mut foods = [FoodCandidate::EMPTY; 6];
    let mut len = 0usize;
    let mut more_food = false;
    for (index, item) in frame.inventory.iter().enumerate() {
        if item.count <= 0 {
            continue;
        }
        let Some(food) = tables.food(item.def.id) else {
            if item
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case("eat"))
            {
                input.set_missing_facts(true);
            }
            continue;
        };
        if food.heal < 0 {
            input.set_missing_facts(true);
            continue;
        }
        if frame.inventory[..index]
            .iter()
            .any(|previous| previous.def.id == item.def.id && previous.count > 0)
        {
            continue;
        }
        let total = frame
            .inventory
            .iter()
            .filter(|row| row.def.id == item.def.id)
            .try_fold(0u32, |sum, row| {
                let count = u32::try_from(row.count.max(0)).ok()?;
                sum.checked_add(count)
            });
        let count = match total {
            Some(total) if total <= u32::from(u8::MAX) => u8::try_from(total).unwrap_or(u8::MAX),
            Some(_) | None => {
                input.set_overflow(true);
                u8::MAX
            }
        };
        let candidate = FoodCandidate {
            id: item.def.id,
            count,
            heal: food.heal,
        };
        retain_food(&mut foods, &mut len, &mut more_food, candidate);
    }
    input.food_len = u8::try_from(len).unwrap_or(6);
    input.set_more_food(more_food);
    for (index, food) in foods[..len].iter().enumerate() {
        input.food_ids[index] = food.id;
        input.food_counts[index] = food.count;
    }
}

#[inline]
fn food_before(left: FoodCandidate, right: FoodCandidate) -> bool {
    left.heal > right.heal || (left.heal == right.heal && left.id < right.id)
}

fn retain_food(
    foods: &mut [FoodCandidate; 6],
    len: &mut usize,
    more_food: &mut bool,
    candidate: FoodCandidate,
) {
    let insertion = foods[..*len]
        .iter()
        .position(|held| food_before(candidate, *held))
        .unwrap_or(*len);
    if *len < foods.len() {
        for slot in (insertion..*len).rev() {
            foods[slot + 1] = foods[slot];
        }
        foods[insertion] = candidate;
        *len += 1;
    } else {
        *more_food = true;
        if insertion < foods.len() {
            for slot in (insertion..foods.len() - 1).rev() {
                foods[slot + 1] = foods[slot];
            }
            foods[insertion] = candidate;
        }
    }
}

fn capture_live(frame: &Frame<'_>, threats: &ThreatSet, input: &mut RiskInput) {
    for threat in threats.iter(frame.tick) {
        let index = usize::from(input.live_len);
        if index >= input.live.len() {
            input.set_overflow(true);
            break;
        }
        let max_hit = match threat.max_hit_est() {
            Some(value) => value,
            None => {
                input.set_missing_facts(true);
                0
            }
        };
        if threat.rate == 0 {
            input.set_missing_facts(true);
        }
        let family = threat.projectile_family();
        let has_due = family != ProjectileFamily::Unknown || threat.due_tick != 0;
        let due_tick = if has_due {
            if threat.due_tick == u16::MAX {
                input.set_overflow(true);
                u16::MAX
            } else {
                threat.due_tick
            }
        } else {
            if frame.tick >= u16::MAX - 3 {
                // `Threat` uses zero for both no impact and a wrapped impact
                // without exposing its private due bit; do not silently omit it.
                input.set_overflow(true);
            }
            u16::MAX
        };
        input.live[index] = LiveRow {
            actor: threat.actor,
            ident: threat.ident,
            max_hit,
            rate: threat.rate,
            due_tick,
        };
        input.live_len += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::{
        ActorView, CombatView, HitmarkView, ItemActionFamily, ItemContainer, ItemView,
        LocalPlayerView, PlayerView, StatView, VarpView, WorldStateView,
    };
    use api::ItemDefView;

    #[expect(
        dead_code,
        reason = "preserve the pre-P1-A input layout for an exact size comparison"
    )]
    struct LegacyRiskInput {
        pos: WorldTile,
        live: [LiveRow; 4],
        food_ids: [i32; 6],
        food_counts: [u8; 6],
        poison: PoisonState,
        tick: u16,
        hp: u8,
        hp_max: u8,
        prayer: u8,
        prayer_base: u8,
        combat: Option<u8>,
        doses_prayer: u8,
        live_len: u8,
        food_len: u8,
        free_slots: u8,
        map_members: bool,
        run: bool,
        other_prayers_on: bool,
        off_debt: bool,
        worn_shield: bool,
        more_food: bool,
        missing_facts: bool,
        unattributed: bool,
        input_locked: bool,
        overflow: bool,
    }

    fn combat_tables() -> std::sync::Arc<CombatTables> {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        CombatTables::build(selected).unwrap()
    }

    fn stat(index: usize, effective: i32, base: i32) -> StatView {
        StatView {
            index: index as i32,
            name: String::new(),
            effective,
            base,
            xp: 0,
            used: true,
        }
    }

    fn equipment_item(id: i32, slot: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: None,
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Equipment,
            action_family: ItemActionFamily::None,
            slot,
            count: 1,
            actions: Vec::new(),
            component_id: -1,
        }
    }

    fn local_player() -> LocalPlayerView {
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        LocalPlayerView {
            player: PlayerView {
                index: 0,
                actor: ActorView {
                    name: None,
                    actions: Vec::new(),
                    tile,
                    distance: 0,
                    animation: -1,
                    animation_frame: -1,
                    pose_animation: -1,
                    orientation: 0,
                    target_orientation: 0,
                    overhead_text: None,
                    spot_animation: -1,
                    spot_animation_stamp: -1,
                    health: 40,
                    total_health: 40,
                    face_entity: -1,
                    target: None,
                    moving: false,
                    running: false,
                    in_combat: false,
                },
                network: tile,
                combat_level: 50,
                skill_level: 0,
                headicons: 0,
                weapon: None,
            },
            energy: 100,
            weight: 0,
            animation_update: None,
        }
    }

    fn capture_input(
        tables: &CombatTables,
        stats: &[StatView],
        varps: &[VarpView],
        equipment: &[ItemView],
        prayers: [bool; 15],
    ) -> RiskInput {
        let local = local_player();
        let world = WorldStateView {
            members: true,
            ..WorldStateView::default()
        };
        let frame = Frame {
            here: local.player.actor.tile,
            combat_tab: None,
            tick: 10,
            stats,
            inventory: &[],
            equipment,
            npcs: &[],
            players: &[],
            local: &local,
            world: &world,
            projectiles: &[],
            hitmarks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
            prayers,
            varps,
            in_combat: CombatView {
                in_combat: false,
                target: None,
            },
            reach: None,
            wilderness: None,
        };
        RiskInput::capture(
            &frame,
            tables,
            &ThreatSet::default(),
            PoisonState::Clear,
            false,
        )
    }

    #[test]
    fn risk_input_layout_records_old_and_new_sizes() {
        let old = std::mem::size_of::<LegacyRiskInput>();
        let new = std::mem::size_of::<RiskInput>();
        eprintln!("RiskInput legacy={old} new={new} delta={}", new - old);
        assert_eq!(old, 120);
        assert_eq!(new, 124);
        assert_eq!(new - old, 4);
        assert!(new <= 128);
    }

    #[test]
    fn packed_flags_round_trip_without_aliasing() {
        let getters: [fn(&RiskInput) -> bool; 10] = [
            RiskInput::map_members,
            RiskInput::run,
            RiskInput::other_prayers_on,
            RiskInput::off_debt,
            RiskInput::worn_shield,
            RiskInput::more_food,
            RiskInput::missing_facts,
            RiskInput::unattributed,
            RiskInput::input_locked,
            RiskInput::overflow,
        ];
        let setters: [fn(&mut RiskInput, bool); 10] = [
            RiskInput::set_map_members,
            RiskInput::set_run,
            RiskInput::set_other_prayers_on,
            RiskInput::set_off_debt,
            RiskInput::set_worn_shield,
            RiskInput::set_more_food,
            RiskInput::set_missing_facts,
            RiskInput::set_unattributed,
            RiskInput::set_input_locked,
            RiskInput::set_overflow,
        ];
        let mut input = RiskInput::default();
        for setter in &setters {
            setter(&mut input, false);
        }
        for (selected, setter) in setters.iter().enumerate() {
            setter(&mut input, true);
            for (index, getter) in getters.iter().enumerate() {
                assert_eq!(getter(&input), index == selected, "flag {selected}/{index}");
            }
            setter(&mut input, false);
        }
    }

    #[test]
    fn capture_defence_is_min_of_effective_and_base() {
        if !api::test_support::game_data_available() {
            return;
        }
        // Old: capture retained no Defence input. New: boosted 75/40 captures 40.
        let tables = combat_tables();
        let stats = [
            stat(CombatStat::Hitpoints.index(), 90, 99),
            stat(CombatStat::Defence.index(), 75, 40),
            stat(CombatStat::Prayer.index(), 10, 10),
        ];
        let varps = [VarpView {
            index: 173,
            value: 0,
        }];
        let input = capture_input(&tables, &stats, &varps, &[], [false; 15]);
        assert_eq!(input.defence, 40);
        assert!(!input.missing_facts());
    }

    #[test]
    fn capture_worn_defence_sums_signed_facts_and_marks_missing_rows() {
        if !api::test_support::game_data_available() {
            return;
        }
        let tables = combat_tables();
        let rows = tables.selected().item_combat_bonus_rows();
        let (left, right) = rows
            .iter()
            .find_map(|left| {
                rows.iter()
                    .find(|right| {
                        right.obj_id != left.obj_id
                            && [
                                left.stabdefence.checked_add(right.stabdefence),
                                left.slashdefence.checked_add(right.slashdefence),
                                left.crushdefence.checked_add(right.crushdefence),
                                left.rangedefence.checked_add(right.rangedefence),
                            ]
                            .into_iter()
                            .flatten()
                            .any(|sum| sum < 0)
                    })
                    .map(|right| (left, right))
            })
            .expect("selected wearable facts include negative Defence bonuses");
        let equipment = [
            equipment_item(left.obj_id, 0),
            equipment_item(right.obj_id, 1),
        ];
        let stats = [
            stat(CombatStat::Hitpoints.index(), 90, 99),
            stat(CombatStat::Defence.index(), 40, 40),
            stat(CombatStat::Prayer.index(), 10, 10),
        ];
        let varps = [VarpView {
            index: 173,
            value: 0,
        }];
        let input = capture_input(&tables, &stats, &varps, &equipment, [false; 15]);
        let expected = [
            left.stabdefence.checked_add(right.stabdefence).unwrap(),
            left.slashdefence.checked_add(right.slashdefence).unwrap(),
            left.crushdefence.checked_add(right.crushdefence).unwrap(),
            left.rangedefence.checked_add(right.rangedefence).unwrap(),
        ];
        assert!(expected.iter().any(|bonus| *bonus < 0));
        assert_eq!(input.worn_defence, expected);
        assert!(!input.missing_facts());

        let missing = [equipment_item(i32::MAX, 0)];
        assert!(tables.selected().item_combat_bonus(i32::MAX).is_none());
        let input = capture_input(&tables, &stats, &varps, &missing, [false; 15]);
        assert_eq!(input.worn_defence, [0; 4]);
        assert!(input.missing_facts());
    }

    #[test]
    fn capture_style_from_varp_else_zero() {
        if !api::test_support::game_data_available() {
            return;
        }
        let tables = combat_tables();
        let selected = tables.selected();
        let style = selected
            .weapon_styles()
            .iter()
            .find(|row| {
                row.style & StyleMask::RANGED.bits() == 0
                    && row.style_bonus.2 > 0
                    && row.tab.is_some_and(|tab| {
                        selected
                            .melee_modes()
                            .iter()
                            .any(|mode| mode.tab == tab && mode.slot == row.style_slot)
                    })
            })
            .expect("selected melee facts include a nonzero Defence style bonus");
        let weapon_id = style.obj_id;
        let slot = style.style_slot;
        let expected = style.style_bonus.2;
        let varp = tables
            .melee_mode_varp()
            .expect("selected melee style varp is present");
        let equipment = [equipment_item(weapon_id, 3)];
        let stats = [
            stat(CombatStat::Hitpoints.index(), 90, 99),
            stat(CombatStat::Defence.index(), 40, 40),
            stat(CombatStat::Prayer.index(), 10, 10),
        ];
        let selected_mode = [
            VarpView {
                index: 173,
                value: 0,
            },
            VarpView {
                index: varp,
                value: i32::from(slot),
            },
        ];
        // Old: capture supplied no style Defence. New: the selected style slot yields its fact.
        let input = capture_input(&tables, &stats, &selected_mode, &equipment, [false; 15]);
        assert_eq!(input.style_def, expected);

        let missing_mode = [VarpView {
            index: 173,
            value: 0,
        }];
        let input = capture_input(&tables, &stats, &missing_mode, &equipment, [false; 15]);
        assert_eq!(input.style_def, 0);
    }

    #[test]
    fn capture_active_protect_uses_selected_prayer_facts() {
        if !api::test_support::game_data_available() {
            return;
        }
        let tables = combat_tables();
        let stats = [
            stat(CombatStat::Hitpoints.index(), 90, 99),
            stat(CombatStat::Defence.index(), 40, 40),
            stat(CombatStat::Prayer.index(), 10, 10),
        ];
        let varps = [VarpView {
            index: 173,
            value: 0,
        }];
        let input = capture_input(&tables, &stats, &varps, &[], [false; 15]);
        assert_eq!(input.active_protect, Style::Unknown);
        for (observed, expected) in [
            (StyleObs::Melee, Style::Melee),
            (StyleObs::Ranged, Style::Ranged),
        ] {
            let fact = crate::combat::select::protect_fact(&tables, observed).unwrap();
            let index = usize::try_from(fact.varp - 83).unwrap();
            let mut prayers = [false; 15];
            prayers[index] = true;
            let input = capture_input(&tables, &stats, &varps, &[], prayers);
            assert_eq!(input.active_protect, expected);
            assert!(input.other_prayers_on());
            assert!(!input.missing_facts());
        }
    }

    #[test]
    fn default_input_fails_closed_and_stays_compact() {
        let input = RiskInput::default();
        assert!(input.missing_facts());
        assert!(input.unattributed());
        assert!(input.input_locked());
        assert!(matches!(input.poison, PoisonState::Unknown { .. }));
        assert!(std::mem::size_of::<RiskInput>() <= 128);
        assert!(std::mem::size_of::<PoisonState>() <= 8);
    }

    #[test]
    fn due_sentinel_and_food_iterator_are_bounded() {
        assert_eq!(LiveRow::EMPTY.due(), None);
        let input = RiskInput {
            food_ids: [10, 11, 0, 0, 0, 0],
            food_counts: [2, 1, 0, 0, 0, 0],
            food_len: 2,
            ..RiskInput::default()
        };
        assert_eq!(input.food_iter().collect::<Vec<_>>(), [(10, 2), (11, 1)]);
    }

    #[test]
    fn retention_keeps_the_six_largest_heals_and_marks_a_seventh_kind() {
        let mut foods = [FoodCandidate::EMPTY; 6];
        let mut len = 0;
        let mut more_food = false;
        for (id, heal) in [
            (1, 1),
            (2, 7),
            (3, 3),
            (4, 9),
            (5, 5),
            (6, 8),
            (7, 2),
            (8, 10),
        ] {
            retain_food(
                &mut foods,
                &mut len,
                &mut more_food,
                FoodCandidate { id, count: 1, heal },
            );
        }
        assert_eq!(len, 6);
        assert!(more_food);
        assert_eq!(
            foods[..len]
                .iter()
                .map(|food| (food.id, food.heal))
                .collect::<Vec<_>>(),
            [(8, 10), (4, 9), (6, 8), (2, 7), (5, 5), (3, 3)]
        );
    }
}
