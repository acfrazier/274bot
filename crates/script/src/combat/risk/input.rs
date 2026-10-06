use super::super::arbiter;
use super::super::frame::Frame;
use super::super::request::{ActorKind, ActorRef};
use super::super::tables::{CombatStat, CombatTables, PotionKind};
use super::super::threats::{ProjectileFamily, ThreatSet};
use super::consts::{LAG, POISON_PERIOD};
use api::snapshot::{StatView, VarpView, WorldTile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Survivable,
    FixableWith,
    Unsurvivable,
    Unknown(UnknownWhy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownWhy {
    Kind(u16),
    AlreadyEngaged,
    NoZoneTable,
    MissingFacts,
    InputLock,
    Poison,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Melee,
    Ranged,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonState {
    Unknown { since: u16 },
    Clear,
    Poisoned { per_tick: u8, last_tick: u16 },
}

impl Default for PoisonState {
    fn default() -> Self {
        Self::Unknown { since: 0 }
    }
}

const _: () = assert!(std::mem::size_of::<PoisonState>() <= 8);

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
    pub map_members: bool,
    pub run: bool,
    pub other_prayers_on: bool,
    pub off_debt: bool,
    pub worn_shield: bool,
    pub more_food: bool,
    pub missing_facts: bool,
    pub unattributed: bool,
    pub input_locked: bool,
    pub overflow: bool,
}

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
            map_members: false,
            run: false,
            other_prayers_on: false,
            off_debt: false,
            worn_shield: false,
            more_food: false,
            missing_facts: true,
            unattributed: true,
            input_locked: true,
            overflow: true,
        }
    }
}

impl RiskInput {
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
            off_debt,
            map_members: frame.world.members,
            input_locked: false,
            missing_facts: false,
            unattributed: threats.has_unattributed_event(frame.tick),
            overflow: threats.has_overflow(frame.tick),
            ..Self::default()
        };

        if let Some(hp) = unique_stat(frame.stats, CombatStat::Hitpoints.index() as i32) {
            input.hp = compact_u8(hp.effective, &mut input.missing_facts, &mut input.overflow);
            input.hp_max = compact_u8(hp.base, &mut input.missing_facts, &mut input.overflow);
            if input.hp == 0 || input.hp_max == 0 {
                input.missing_facts = true;
            }
        } else {
            input.missing_facts = true;
        }

        if let Some(prayer) = unique_stat(frame.stats, CombatStat::Prayer.index() as i32) {
            input.prayer = compact_u8(
                prayer.effective,
                &mut input.missing_facts,
                &mut input.overflow,
            );
            input.prayer_base =
                compact_u8(prayer.base, &mut input.missing_facts, &mut input.overflow);
            if input.prayer_base == 0 {
                input.missing_facts = true;
            }
        } else {
            input.missing_facts = true;
        }

        input.combat = match u8::try_from(frame.local.player.combat_level) {
            Ok(level) if level > 0 => Some(level),
            Ok(_) => {
                input.missing_facts = true;
                None
            }
            Err(_) if frame.local.player.combat_level > i32::from(u8::MAX) => {
                input.missing_facts = true;
                input.overflow = true;
                None
            }
            Err(_) => {
                input.missing_facts = true;
                None
            }
        };

        input.run = match unique_varp(frame.varps, 173) {
            Some(0) => false,
            Some(1) => true,
            Some(_) | None => {
                input.missing_facts = true;
                false
            }
        };
        input.other_prayers_on = frame.prayers.iter().any(|active| *active);

        match tables.selected().item_by_alias("antidragonbreathshield") {
            Some(shield) => {
                input.worn_shield = frame
                    .equipment
                    .iter()
                    .any(|item| item.slot == 5 && item.def.id == shield.id);
            }
            None => input.missing_facts = true,
        }

        if tables.potion(PotionKind::Prayer).is_some() {
            let doses = arbiter::doses(frame, tables, PotionKind::Prayer);
            if doses < 0 {
                input.missing_facts = true;
            } else if doses > i16::from(u8::MAX) {
                input.doses_prayer = u8::MAX;
                input.overflow = true;
            } else {
                input.doses_prayer = u8::try_from(doses).unwrap_or(u8::MAX);
            }
        } else {
            input.missing_facts = true;
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

fn compact_u8(value: i32, missing: &mut bool, overflow: &mut bool) -> u8 {
    match u8::try_from(value) {
        Ok(value) => value,
        Err(_) if value > i32::from(u8::MAX) => {
            *missing = true;
            *overflow = true;
            u8::MAX
        }
        Err(_) => {
            *missing = true;
            0
        }
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
            input.missing_facts = true;
            continue;
        }
        if item.count == 0 {
            continue;
        }
        let Ok(slot) = usize::try_from(item.slot) else {
            input.missing_facts = true;
            input.overflow = true;
            continue;
        };
        let Some(used) = occupied.get_mut(slot) else {
            input.missing_facts = true;
            input.overflow = true;
            continue;
        };
        if *used {
            input.missing_facts = true;
            input.overflow = true;
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
                input.missing_facts = true;
            }
            continue;
        };
        if food.heal < 0 {
            input.missing_facts = true;
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
                input.overflow = true;
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
    input.more_food = more_food;
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
        let Some(slot) = input.live.get_mut(index) else {
            input.overflow = true;
            break;
        };
        let max_hit = match threat.max_hit_est() {
            Some(value) => value,
            None => {
                input.missing_facts = true;
                0
            }
        };
        if threat.rate == 0 {
            input.missing_facts = true;
        }
        let family = threat.projectile_family();
        let has_due = family != ProjectileFamily::Unknown || threat.due_tick != 0;
        let due_tick = if has_due {
            if threat.due_tick == u16::MAX {
                input.overflow = true;
                u16::MAX
            } else {
                threat.due_tick
            }
        } else {
            if frame.tick >= u16::MAX - 3 {
                // `Threat` uses zero for both no impact and a wrapped impact
                // without exposing its private due bit; do not silently omit it.
                input.overflow = true;
            }
            u16::MAX
        };
        *slot = LiveRow {
            actor: threat.actor,
            ident: threat.ident,
            max_hit,
            rate: threat.rate,
            due_tick,
        };
        input.live_len += 1;
    }
}

/// The additional evidence required to turn a session's `Unknown` into
/// `Clear`. Each field represents an observed phase fact, not elapsed silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedClickWindow {
    pub t: u16,
    pub click_sent_at: u16,
    pub map_aim_at_t: bool,
    pub tile_progress_at_t: bool,
    pub main_clear_t: bool,
    pub main_clear_t_plus_1: bool,
    pub chat_clear_t: bool,
    pub chat_clear_t_plus_1: bool,
    pub no_hold_t: bool,
    pub no_hold_t_plus_1: bool,
    pub snapshots_from: u16,
    pub snapshots_through: u16,
    pub snapshots_consecutive: bool,
    pub poison_mark_seen_through_end: bool,
}

impl AcceptedClickWindow {
    fn proves_timer_ran(self, since: u16) -> bool {
        let Ok(period) = u16::try_from(POISON_PERIOD) else {
            return false;
        };
        let Ok(lag) = u16::try_from(LAG) else {
            return false;
        };
        let elapsed = self.t.wrapping_sub(since);
        let expected_from = self.t.wrapping_sub(1);
        let expected_through = self.t.wrapping_add(1).wrapping_add(lag);
        elapsed >= period
            && elapsed < 0x8000
            && self.click_sent_at == self.t.wrapping_sub(1)
            && self.map_aim_at_t
            && self.tile_progress_at_t
            && self.main_clear_t
            && self.main_clear_t_plus_1
            && self.chat_clear_t
            && self.chat_clear_t_plus_1
            && self.no_hold_t
            && self.no_hold_t_plus_1
            && self.snapshots_from == expected_from
            && self.snapshots_through == expected_through
            && self.snapshots_consecutive
            && !self.poison_mark_seen_through_end
    }
}

/// An already-captured state and its session-only evidence. This value reducer
/// performs no observation, I/O, clock reads, or host operations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PoisonMemory {
    pub state: PoisonState,
    pub session_invalid: bool,
    pub death_seen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonEvent {
    None,
    Reconnect { tick: u16 },
    Hitmark { kind: u16, value: u8, tick: u16 },
    PoisonChat { tick: u16 },
    ConfirmedAntipoisonDecrement,
    DeathObserved,
    RespawnObserved,
}

impl PoisonMemory {
    pub const fn from_state(state: PoisonState) -> Self {
        Self {
            state,
            session_invalid: false,
            death_seen: false,
        }
    }

    /// Apply one explicit event and optional full accepted-click evidence.
    /// Silence does not decrement, expire, or clear a known poison reserve.
    pub fn transition(
        mut self,
        event: PoisonEvent,
        accepted_click: Option<AcceptedClickWindow>,
    ) -> Self {
        match event {
            PoisonEvent::None => {}
            PoisonEvent::Reconnect { tick } => {
                self.state = PoisonState::Unknown { since: tick };
                self.session_invalid = false;
                self.death_seen = false;
            }
            PoisonEvent::Hitmark {
                kind: 2,
                value,
                tick,
            } => {
                if !self.session_invalid {
                    let per_tick = match self.state {
                        PoisonState::Poisoned {
                            per_tick: previous, ..
                        } => previous.max(value),
                        PoisonState::Unknown { .. } | PoisonState::Clear => value,
                    };
                    self.state = PoisonState::Poisoned {
                        per_tick,
                        last_tick: tick,
                    };
                }
            }
            PoisonEvent::Hitmark { kind: 0 | 1, .. } => {}
            PoisonEvent::Hitmark { tick, .. } => {
                self.state = PoisonState::Unknown { since: tick };
                self.session_invalid = true;
                self.death_seen = false;
            }
            PoisonEvent::PoisonChat { tick } => {
                if !self.session_invalid && !matches!(self.state, PoisonState::Poisoned { .. }) {
                    self.state = PoisonState::Unknown { since: tick };
                }
            }
            PoisonEvent::ConfirmedAntipoisonDecrement if !self.session_invalid => {
                self.state = PoisonState::Clear;
                self.death_seen = false;
            }
            PoisonEvent::ConfirmedAntipoisonDecrement => {}
            PoisonEvent::DeathObserved => self.death_seen = true,
            PoisonEvent::RespawnObserved if self.death_seen && !self.session_invalid => {
                self.state = PoisonState::Clear;
                self.death_seen = false;
            }
            PoisonEvent::RespawnObserved => {}
        }

        if matches!(event, PoisonEvent::None)
            && !self.session_invalid
            && !self.death_seen
            && matches!(self.state, PoisonState::Unknown { .. })
            && accepted_click.is_some_and(|window| {
                let PoisonState::Unknown { since } = self.state else {
                    return false;
                };
                window.proves_timer_ran(since)
            })
        {
            self.state = PoisonState::Clear;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_input_fails_closed_and_stays_compact() {
        let input = RiskInput::default();
        assert!(input.missing_facts);
        assert!(input.unattributed);
        assert!(input.input_locked);
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
