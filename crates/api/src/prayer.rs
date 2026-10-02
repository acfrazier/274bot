//! Compact prayer queries over selected fact rows and observed stats/varps.
//! Lookup reuses [`SelectedGameData::prayer_by_name`]; this module does not
//! invent a second prayer table.

use crate::game_data::{PrayerFact, SelectedGameData};
use smallvec::SmallVec;

pub const TOGGLE_MS: u64 = 2_000;

/// Frozen `on` argument after JS marshalling classified the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnArg {
    Undefined,
    Bool(bool),
    Other { truthy: bool },
}

/// Compact observed prayer points/max and selected overlay varps. Each key
/// comes from the selected prayer rows; missing/truncated rows stay
/// unobserved (not a proven 0, not a retained prior 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrayerObservation {
    pub points: i32,
    pub max: i32,
    varps: SmallVec<[(i32, Option<i32>); 15]>,
}

impl PrayerObservation {
    pub fn empty() -> Self {
        Self {
            points: 0,
            max: 0,
            varps: SmallVec::new(),
        }
    }

    pub fn for_prayers(prayers: &[PrayerFact]) -> Self {
        Self {
            points: 0,
            max: 0,
            varps: prayers.iter().map(|prayer| (prayer.varp, None)).collect(),
        }
    }

    pub fn varp(&self, varp: i32) -> i32 {
        self.varps
            .iter()
            .find(|(known, _)| *known == varp)
            .and_then(|(_, value)| *value)
            .unwrap_or(0)
    }

    pub fn varp_observed(&self, varp: i32) -> bool {
        self.varps
            .iter()
            .any(|(known, value)| *known == varp && value.is_some())
    }

    pub fn is_on(&self, varp: i32) -> bool {
        self.varp_observed(varp) && self.varp(varp) == 1
    }

    pub fn is_off(&self, varp: i32) -> bool {
        self.varp_observed(varp) && self.varp(varp) == 0
    }

    pub fn set_varp(&mut self, varp: i32, value: i32) {
        if let Some((_, observed)) = self.varps.iter_mut().find(|(known, _)| *known == varp) {
            *observed = Some(value);
        }
    }

    pub fn unobserve_varp(&mut self, varp: i32) {
        if let Some((_, observed)) = self.varps.iter_mut().find(|(known, _)| *known == varp) {
            *observed = None;
        }
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

    fn fact(varp: i32) -> PrayerFact {
        PrayerFact {
            name: format!("Prayer {varp}"),
            level: 1,
            source_row: String::new(),
            prayer_constant: String::new(),
            button_com: 0,
            com_alias: String::new(),
            varp,
            varp_alias: String::new(),
        }
    }

    #[test]
    fn both_selected_caches_lookup_trim_and_case() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
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
        let required = lookup(&data, "Protect from Melee").unwrap().level;
        let mut obs = PrayerObservation::for_prayers(data.prayers());
        obs.max = required;
        obs.points = 1;
        assert!(available(&data, "Protect from Melee", &obs));
        obs.points = 0;
        assert!(!available(&data, "Protect from Melee", &obs));
        obs.points = 10;
        obs.max = required - 1;
        assert!(!available(&data, "Protect from Melee", &obs));
        assert!(!available(&data, "Nope", &obs));
    }

    #[test]
    fn active_is_varp_one_and_missing_stats_are_zero() {
        let data = data(ClientRevision::R274);
        let melee = lookup(&data, "Protect from Melee").unwrap();
        let obs = PrayerObservation::for_prayers(data.prayers());
        assert_eq!(points(&obs), 0);
        assert_eq!(max(&obs), 0);
        assert!(!full(&obs));
        assert!(!active(&data, "Protect from Melee", &obs));
        let mut on = obs.clone();
        on.set_varp(melee.varp, 1);
        on.max = melee.level;
        on.points = melee.level;
        assert!(active(&data, "Protect from Melee", &on));
        assert!(full(&on));
        assert!(!matches_on(true, OnArg::Undefined));
        assert!(matches_on(true, OnArg::Bool(true)));
        assert!(!on_is_truthy(OnArg::Undefined));
        assert!(on_is_truthy(OnArg::Other { truthy: true }));
    }

    #[test]
    fn unobserved_overlay_is_neither_on_nor_off() {
        let data = data(ClientRevision::R289);
        let varp = lookup(&data, "Protect from Melee").unwrap().varp;
        let mut obs = PrayerObservation::for_prayers(data.prayers());
        assert!(!obs.varp_observed(varp));
        assert!(!obs.is_on(varp));
        assert!(!obs.is_off(varp));
        obs.set_varp(varp, 1);
        assert!(obs.is_on(varp));
        obs.unobserve_varp(varp);
        assert!(!obs.varp_observed(varp));
        assert!(!obs.is_on(varp));
        assert!(!obs.is_off(varp));
        obs.set_varp(varp, 0);
        assert!(obs.is_off(varp));
        assert!(!obs.is_on(varp));
    }

    #[test]
    fn observation_tracks_noncontiguous_selected_varps_only() {
        let prayers = [fact(12), fact(97), fact(301)];
        let mut obs = PrayerObservation::for_prayers(&prayers);
        obs.set_varp(12, 1);
        obs.set_varp(97, 0);
        obs.set_varp(83, 1);
        assert!(obs.is_on(12));
        assert!(obs.is_off(97));
        assert!(!obs.varp_observed(83));
        assert!(!obs.is_on(83));
        obs.unobserve_varp(12);
        assert!(!obs.varp_observed(12));
    }
}
