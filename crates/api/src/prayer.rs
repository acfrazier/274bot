//! Compact prayer queries over selected fact rows and observed stats/varps.
//! Lookup reuses [`SelectedGameData::prayer_by_name`]; this module does not
//! invent a second prayer table.

use crate::game_data::{PrayerFact, SelectedGameData};

pub const TOGGLE_MS: u64 = 2_000;
pub const PRAYER_VARP0: i32 = 83;
pub const PRAYER_COUNT: usize = 15;

/// Frozen `on` argument after JS marshalling classified the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnArg {
    Undefined,
    Bool(bool),
    Other { truthy: bool },
}

/// Compact observed prayer points/max and the 15 overlay varps (83–97).
/// `present` is a bit per selected overlay: missing/truncated rows stay
/// unobserved (not a proven 0, not a retained prior 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrayerObservation {
    pub points: i32,
    pub max: i32,
    pub varps: [i32; PRAYER_COUNT],
    present: u16,
}

impl PrayerObservation {
    pub const fn empty() -> Self {
        Self {
            points: 0,
            max: 0,
            varps: [0; PRAYER_COUNT],
            present: 0,
        }
    }

    fn slot(varp: i32) -> Option<usize> {
        let Ok(index) = usize::try_from(varp - PRAYER_VARP0) else {
            return None;
        };
        (index < PRAYER_COUNT).then_some(index)
    }

    pub fn varp(&self, varp: i32) -> i32 {
        let Some(index) = Self::slot(varp) else {
            return 0;
        };
        if self.present & (1 << index) == 0 {
            return 0;
        }
        self.varps[index]
    }

    pub fn varp_observed(&self, varp: i32) -> bool {
        Self::slot(varp).is_some_and(|index| self.present & (1 << index) != 0)
    }

    pub fn is_on(&self, varp: i32) -> bool {
        self.varp_observed(varp) && self.varp(varp) == 1
    }

    pub fn is_off(&self, varp: i32) -> bool {
        self.varp_observed(varp) && self.varp(varp) == 0
    }

    pub fn set_varp(&mut self, varp: i32, value: i32) {
        let Some(index) = Self::slot(varp) else {
            return;
        };
        self.varps[index] = value;
        self.present |= 1 << index;
    }

    pub fn unobserve_varp(&mut self, varp: i32) {
        let Some(index) = Self::slot(varp) else {
            return;
        };
        self.varps[index] = 0;
        self.present &= !(1 << index);
    }
}

pub fn lookup<'a>(data: &'a SelectedGameData, name: &str) -> Option<&'a PrayerFact> {
    data.prayer_by_name(name)
}

pub fn points(obs: &PrayerObservation) -> i32 {
    obs.points
}

pub fn max(obs: &PrayerObservation) -> i32 {
    obs.max
}

pub fn full(obs: &PrayerObservation) -> bool {
    obs.max > 0 && obs.points >= obs.max
}

pub fn known(data: &SelectedGameData, name: &str) -> bool {
    lookup(data, name).is_some()
}

pub fn available(data: &SelectedGameData, name: &str, obs: &PrayerObservation) -> bool {
    lookup(data, name).is_some_and(|row| obs.max >= row.level && obs.points > 0)
}

pub fn active(data: &SelectedGameData, name: &str, obs: &PrayerObservation) -> bool {
    lookup(data, name).is_some_and(|row| obs.is_on(row.varp))
}

/// Frozen `active(name) === on` using the classified raw `on`.
pub fn matches_on(is_active: bool, on: OnArg) -> bool {
    match on {
        OnArg::Bool(want) => is_active == want,
        OnArg::Undefined | OnArg::Other { .. } => false,
    }
}

/// Frozen `on &&` truthiness after marshalling classified the raw value.
pub fn on_is_truthy(on: OnArg) -> bool {
    match on {
        OnArg::Bool(value) => value,
        OnArg::Other { truthy } => truthy,
        OnArg::Undefined => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        crate::game_data::for_revision(rev).expect("selected data")
    }

    #[test]
    fn both_selected_caches_lookup_trim_and_case() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            assert_eq!(data.prayers().len(), 15);
            assert!(known(&data, "Protect from Melee"));
            assert!(known(&data, "  protect from melee  "));
            assert!(known(&data, "THICK SKIN"));
            assert!(!known(&data, "Not a prayer"));
            let melee = lookup(&data, "Protect from Melee").expect("melee");
            assert_eq!(melee.button_com, 5623);
            assert_eq!(melee.varp, 97);
            assert_eq!(melee.level, 43);
        }
    }

    #[test]
    fn available_requires_level_and_remaining_points() {
        let data = data(ClientRevision::R289);
        let mut obs = PrayerObservation::empty();
        obs.max = 43;
        obs.points = 1;
        assert!(available(&data, "Protect from Melee", &obs));
        obs.points = 0;
        assert!(!available(&data, "Protect from Melee", &obs));
        obs.points = 10;
        obs.max = 42;
        assert!(!available(&data, "Protect from Melee", &obs));
        assert!(!available(&data, "Nope", &obs));
    }

    #[test]
    fn active_is_varp_one_and_missing_stats_are_zero() {
        let data = data(ClientRevision::R274);
        let obs = PrayerObservation::empty();
        assert_eq!(points(&obs), 0);
        assert_eq!(max(&obs), 0);
        assert!(!full(&obs));
        assert!(!active(&data, "Protect from Melee", &obs));
        let mut on = obs;
        on.set_varp(97, 1);
        on.max = 43;
        on.points = 43;
        assert!(active(&data, "Protect from Melee", &on));
        assert!(full(&on));
        assert!(!matches_on(true, OnArg::Undefined));
        assert!(matches_on(true, OnArg::Bool(true)));
        assert!(!on_is_truthy(OnArg::Undefined));
        assert!(on_is_truthy(OnArg::Other { truthy: true }));
    }

    #[test]
    fn unobserved_overlay_is_neither_on_nor_off() {
        let mut obs = PrayerObservation::empty();
        assert!(!obs.varp_observed(97));
        assert!(!obs.is_on(97));
        assert!(!obs.is_off(97));
        obs.set_varp(97, 1);
        assert!(obs.is_on(97));
        obs.unobserve_varp(97);
        assert!(!obs.varp_observed(97));
        assert!(!obs.is_on(97));
        assert!(!obs.is_off(97));
        obs.set_varp(97, 0);
        assert!(obs.is_off(97));
        assert!(!obs.is_on(97));
    }
}
