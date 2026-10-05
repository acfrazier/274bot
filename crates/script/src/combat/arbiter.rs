//! Fixed, owned decisions; request strings are constructed only at admission.

use super::frame::Frame;
use super::policy;
use super::tables::{CombatTables, PotionKind};
pub(crate) const ARM_SIDE_TAB_FLAG: u8 = 1 << 7;
pub(crate) const ARM_WAIT_MASK: u8 = !ARM_SIDE_TAB_FLAG;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub(crate) enum RowKind {
    #[default]
    Empty,
    Eat,
    Drink,
    Prayer,
    Wear,
    Style,
    Retaliate,
    Attack,
    Cast,
    Arm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PlanRow {
    pub id: i32,
    pub kind: RowKind,
    pub aux: u8,
}
impl PlanRow {
    pub fn new(kind: RowKind, id: i32, aux: u8) -> Self {
        Self { kind, id, aux }
    }
    pub fn cost(self) -> u8 {
        if self.kind == RowKind::Arm && self.aux & ARM_SIDE_TAB_FLAG != 0 {
            return 0;
        }
        if matches!(self.kind, RowKind::Attack | RowKind::Cast) {
            2
        } else {
            1
        }
    }
    pub fn terminal(self, tables: &CombatTables) -> bool {
        matches!(self.kind, RowKind::Attack | RowKind::Cast | RowKind::Arm)
            || self.kind == RowKind::Drink
            || (self.kind == RowKind::Eat
                && tables
                    .food(self.id)
                    .is_some_and(|food| food.message_delay.is_some()))
            || (self.kind == RowKind::Wear
                && tables
                    .selected()
                    .item_by_alias("elemental_shield")
                    .is_some_and(|item| item.id == self.id))
    }
}
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TickPlan {
    pub rows: [PlanRow; 5],
    pub len: u8,
    pub events: u8,
}
impl TickPlan {
    pub fn iter(&self) -> impl Iterator<Item = PlanRow> + '_ {
        self.rows[..usize::from(self.len)].iter().copied()
    }
    pub fn closed(&self, tables: &CombatTables) -> bool {
        self.iter().last().is_some_and(|row| row.terminal(tables))
    }
    pub fn push(&mut self, row: PlanRow, reserve: u8, tables: &CombatTables) -> bool {
        if self.len == 5 || self.closed(tables) || self.events + row.cost() + reserve > 5 {
            return false;
        }
        self.rows[usize::from(self.len)] = row;
        self.len += 1;
        self.events += row.cost();
        true
    }
    pub fn contains(&self, kind: RowKind) -> bool {
        self.iter().any(|row| row.kind == kind)
    }
}
/// Largest held heal fitting the HP deficit, otherwise the smallest eligible
/// heal. Recovery applies the cap, not the nominal heal.
pub(crate) fn food_by(
    frame: &Frame<'_>,
    tables: &CombatTables,
    gate: Option<i32>,
    eligible: impl Fn(&super::tables::FoodFact) -> bool,
) -> Option<i32> {
    let (hp, hp_max) = stat(frame, 3);
    policy::food_by(
        hp,
        hp_max,
        frame.inventory.iter().map(|row| (row.def.id, row.count)),
        tables,
        gate,
        eligible,
    )
}
pub(crate) fn stat(frame: &Frame<'_>, index: i32) -> (i32, i32) {
    frame
        .stats
        .iter()
        .find(|row| row.index == index)
        .map_or((0, 0), |row| (row.effective, row.base))
}
pub(crate) fn count(frame: &Frame<'_>, id: i32) -> i32 {
    frame
        .inventory
        .iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}
pub(crate) fn potion_id(frame: &Frame<'_>, tables: &CombatTables, kind: PotionKind) -> Option<i32> {
    let family = tables.potion(kind)?;
    family
        .doses
        .iter()
        .flatten()
        .filter(|dose| count(frame, dose.id) > 0)
        .min_by_key(|dose| dose.doses)
        .map(|dose| dose.id)
}
pub(crate) fn doses(frame: &Frame<'_>, tables: &CombatTables, kind: PotionKind) -> i16 {
    tables.potion(kind).map_or(0, |family| {
        family
            .doses
            .iter()
            .flatten()
            .map(|dose| count(frame, dose.id).saturating_mul(i32::from(dose.doses)))
            .sum::<i32>()
            .min(i32::from(i16::MAX)) as i16
    })
}
