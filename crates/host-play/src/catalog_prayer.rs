use super::*;
pub const PRAYER_V2_STOP: &str = "prayer v2 qualification complete";
pub const PRAYER_V1_STOP: &str = "prayer v1 qualification complete";

fn protect_from_melee() -> &'static api::game_data::PrayerFact {
    super::ids::selected_catalog_game_data()
        .prayer_by_name("Protect from Melee")
        .expect("selected R289 data lacks Protect from Melee")
}

pub fn prayer_varp_indexes() -> impl Iterator<Item = i32> {
    super::ids::selected_catalog_game_data()
        .prayers()
        .iter()
        .map(|prayer| prayer.varp)
}
pub fn is_prayer_varp(index: i32) -> bool {
    prayer_varp_indexes().any(|varp| varp == index)
}

/// All selected overlay keys present and zero. Missing keys are not off.
pub fn prayer_varps_all_present_off(observation: &Observation) -> bool {
    prayer_varp_indexes().all(|index| observation.varps.get(&index) == Some(&0))
}

pub fn prayer_delivery_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.level("prayer") >= protect_from_melee().level
        && baseline.effective_level("prayer") > 0
        && prayer_varps_all_present_off(baseline)
}

/// Ordered witness: seeded all-off, then a latched Protect from Melee ON
/// (the selected prayer row's varp is 1), then all selected overlays present
/// and off, then the exact File-card stop reason.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PrayerDeliveryCycle {
    pub saw_on: bool,
    pub later_all_off: bool,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl PrayerDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if now.varps.get(&protect_from_melee().varp) == Some(&1) {
            self.saw_on = true;
        }
        if self.saw_on && prayer_varps_all_present_off(now) {
            self.later_all_off = true;
        }
    }

    pub fn observe_script_lifecycle(
        &mut self,
        receipt: script::ScriptLifecycleReceipt,
        expected: &str,
    ) {
        if self.later_all_off
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && receipt.reason == expected
        {
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        self.saw_on && self.later_all_off && self.stopped.is_some()
    }
}
