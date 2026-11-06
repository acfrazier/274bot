//! Real, opt-in W6 memory workloads. This module owns benchmark control,
//! never a replacement runtime, trade policy, journal provider or router.

mod active_route;
mod journal_compat;
mod native_api;
mod native_combat;
mod native_trade;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    PriestJournal,
    CompatibilityObjectCache,
    NativePartnerTrade,
    MergedNativeApi,
    NativeNpcCombat,
    ActiveRouteNavigation,
}

impl Kind {
    pub(super) fn supports_count(self, n: usize) -> bool {
        match self {
            Self::NativePartnerTrade => matches!(n, 2 | 10 | 16 | 32 | 50 | 128),
            Self::MergedNativeApi | Self::NativeNpcCombat => matches!(n, 1 | 10),
            _ => matches!(n, 1 | 10 | 16 | 32 | 50 | 128),
        }
    }

    pub(super) fn build(self, names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
        if !self.supports_count(names.len()) {
            return Err(format!(
                "memory feature {self:?} does not support {} accounts",
                names.len(),
            ));
        }
        match self {
            Self::PriestJournal => journal_compat::priest(names),
            Self::CompatibilityObjectCache => journal_compat::objects(names),
            Self::NativePartnerTrade => native_trade::build(names),
            Self::MergedNativeApi => native_api::build(names),
            Self::NativeNpcCombat => native_combat::build(names),
            Self::ActiveRouteNavigation => active_route::build(names),
        }
    }
}

/// Counts backed by each case's actual client and product observations.
/// An unused feature has no active-feature proof; readiness is separate.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Observation {
    pub ready: usize,
    pub active: usize,
    pub proved: usize,
    pub continuing: usize,
}

pub(super) trait FeatureCase: Send {
    fn seed(&self, slot: usize, names: &[String]) -> Result<scenario::Scenario, String>;
    fn prepare_unused(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String>;
    fn activate(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String>;
    fn observe(&mut self, play: &crate::Play, names: &[String]) -> Result<Observation, String>;
    fn stop(&mut self, play: &crate::Play, names: &[String]) -> Result<(), String>;
    fn evidence(&self, names: &[String]) -> serde_json::Value;
}

#[cfg(test)]
mod tests {
    use super::Kind;

    #[test]
    fn native_trade_accepts_its_real_paired_pilot_but_not_unpaired_fleets() {
        for n in [2, 10, 16, 32, 50, 128] {
            assert!(Kind::NativePartnerTrade.supports_count(n));
        }
        for n in [0, 1, 3, 9] {
            assert!(!Kind::NativePartnerTrade.supports_count(n));
        }
    }

    #[test]
    fn other_features_keep_the_existing_measured_fleet_choices() {
        for kind in [
            Kind::PriestJournal,
            Kind::CompatibilityObjectCache,
            Kind::ActiveRouteNavigation,
        ] {
            for n in [1, 10, 16, 32, 50, 128] {
                assert!(kind.supports_count(n));
            }
            for n in [0, 2, 3, 9] {
                assert!(!kind.supports_count(n));
            }
        }
    }

    #[test]
    fn native_api_accepts_only_its_independent_tree_fixture_counts() {
        for n in [1, 10] {
            assert!(Kind::MergedNativeApi.supports_count(n));
        }
        for n in [0, 2, 3, 9, 16, 32, 50, 128] {
            assert!(!Kind::MergedNativeApi.supports_count(n));
        }
    }

    #[test]
    fn native_npc_combat_accepts_only_its_independent_npc_fixture_counts() {
        for n in [1, 10] {
            assert!(Kind::NativeNpcCombat.supports_count(n));
        }
        for n in [0, 2, 3, 9, 16, 32, 50, 128] {
            assert!(!Kind::NativeNpcCombat.supports_count(n));
        }
    }
}
