//! Safety rows share the same food eligibility and actual-heal rule.
use super::frame::Frame;
use super::tables::{CombatTables, PotionKind};
use api::WorldTile;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Intent {
    Eat { id: i32, recovery: bool },
    Drink(PotionKind),
    Prayer { varp: i32, button: i32, on: bool },
    Wear { id: i32, slot: u8 },
    Retaliate(bool),
    Attack { restoration: bool },
    Walk { tile: WorldTile, escape: bool },
}

#[derive(Default)]
pub(crate) struct TickPlan {
    ops: [Option<Intent>; 5],
}
impl TickPlan {
    pub(crate) fn single(intent: Intent) -> Self {
        Self::from_option(Some(intent))
    }
    pub(crate) fn from_option(first: Option<Intent>) -> Self {
        Self { ops: [first, None, None, None, None] }
    }
    pub(crate) fn take_first(&mut self) -> Option<Intent> {
        self.ops[0].take()
    }
}
/// Largest held heal that fits, otherwise the smallest eligible heal. A
/// recovery candidate must actually cross the gate after the HP cap.
pub(crate) fn food(frame: &Frame<'_>, tables: &CombatTables, gate: Option<i32>, readiness: Option<(&super::schedule::Schedule, u16)>) -> Option<i32> {
    let hp = stat(frame, 3).0;
    let max = stat(frame, 3).1;
    let mut fitting: Option<(i32, i32)> = None;
    let mut smallest: Option<(i32, i32)> = None;
    for row in frame.inventory {
        let Some(food) = tables.food(row.def.id) else { continue; };
        let heal = food.heal;
        if row.count <= 0 || gate.is_some_and(|gate| hp.saturating_add(heal).min(max) <= gate)
            || readiness.is_some_and(|(schedule, tick)| !schedule.food_ready(food, tick)) { continue; }
        if smallest.is_none_or(|(_, current)| heal < current) { smallest = Some((row.def.id, heal)); }
        if hp.saturating_add(heal) <= max && fitting.is_none_or(|(_, current)| heal > current) {
            fitting = Some((row.def.id, heal));
        }
    }
    fitting.or(smallest).map(|(id, _)| id)
}
pub(crate) fn stat(frame: &Frame<'_>, index: i32) -> (i32, i32) {
    frame.stats.iter().find(|row| row.index == index).map_or((0, 0), |row| (row.effective, row.base))
}
pub(crate) fn count(frame: &Frame<'_>, id: i32) -> i32 {
    frame.inventory.iter().filter(|row| row.def.id == id).map(|row| row.count).sum()
}
pub(crate) fn potion_id(frame: &Frame<'_>, tables: &CombatTables, kind: PotionKind) -> Option<i32> {
    let family = tables.potion(kind)?;
    family.doses.iter().flatten().filter(|dose| count(frame, dose.id) > 0).min_by_key(|dose| dose.doses).map(|dose| dose.id)
}
pub(crate) fn doses(frame: &Frame<'_>, tables: &CombatTables, kind: PotionKind) -> i16 {
    tables.potion(kind).map_or(0, |family| {
        family.doses.iter().flatten().map(|dose| count(frame, dose.id).saturating_mul(i32::from(dose.doses))).sum::<i32>().min(i32::from(i16::MAX)) as i16
    })
}
