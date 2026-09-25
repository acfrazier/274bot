use super::*;
pub const PRAYER_V2_STOP: &str = "prayer v2 qualification complete";
pub const PRAYER_V1_STOP: &str = "prayer v1 qualification complete";
pub const PRAYER_BASE_MIN: i32 = 43;
pub const PROTECT_FROM_MELEE_VARP: i32 = 97;

pub fn prayer_varp_indexes() -> impl Iterator<Item = i32> {
    let start = api::prayer::PRAYER_VARP0;
    (0..api::prayer::PRAYER_COUNT as i32).map(move |i| start + i)
}

/// All 15 overlay keys present and zero. Missing keys are not off.
pub fn prayer_varps_all_present_off(observation: &Observation) -> bool {
    prayer_varp_indexes().all(|index| observation.varps.get(&index) == Some(&0))
}

pub fn prayer_delivery_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.level("prayer") >= PRAYER_BASE_MIN
        && baseline.effective_level("prayer") > 0
        && prayer_varps_all_present_off(baseline)
}

/// Ordered witness: seeded all-off, then a latched Protect from Melee ON
/// (varp 97==1, even if later cleared the same/later tick), then all 15
/// present-and-off, then the exact File-card stop reason.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PrayerDeliveryCycle {
    pub saw_on: bool,
    pub later_all_off: bool,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl PrayerDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if now.varps.get(&PROTECT_FROM_MELEE_VARP) == Some(&1) {
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