//! Read-only state exposed for the persistent manual-click LIVE proofs.

use crate::{script_runtime::script_slot, Play};

impl Play {
    /// Snapshot the script walk and its owner state for the manual-click LIVE harness.
    #[doc(hidden)]
    pub fn manual_click_live_probe(&self, name: &str) -> serde_json::Value {
        let script = script_slot(&self.scripts, name).map(|cell| {
            let slot = cell.lock().unwrap();
            serde_json::json!({
                "runtime_generation": slot.runtime_generation(),
                "run_state": format!("{:?}", slot.state()),
                "walk_result": slot
                    .probe("globalThis.__manual_walk_result")
                    .unwrap_or(serde_json::Value::Null),
                "terminal_count": slot
                    .probe("globalThis.__manual_terminal_count || 0")
                    .unwrap_or(serde_json::Value::Null),
                "error": slot.last_error(),
            })
        });
        let navs = self.navs.lock().unwrap();
        let nav = navs.get(name).map(|bot| {
            serde_json::json!({
                "armed": bot.script_walk_armed(),
                "route": bot.route.is_some(),
                "worker": bot.route_worker.is_some(),
                "pending": bot.pending_route.is_some(),
                "requested": bot.requested_route.is_some(),
                "bank_fetch": bot.bank_fetch.is_some(),
                "carry": bot.carried_walk.is_some(),
                "route_generation": bot.route_generation,
                "request_id": bot.walk_request_id,
                "route_request_id": bot.route_request_id,
                "route_destination": bot.route.as_ref().map(|route| [
                    route.dest.x, route.dest.z, route.dest.level,
                ]),
                "follow_aim": bot.traveller.current_aim().map(|aim| [
                    aim.x, aim.z, aim.level,
                ]),
                "outcome_seq": bot.walk_outcome_seq,
                "outcome_generation": bot.walk_outcome_generation,
                "outcome_request_id": bot.walk_outcome_request_id,
                "failed": bot.walk_outcome_failed,
                "blocked": bot.walk_outcome_blocked,
                "reason": format!("{:?}", bot.walk_outcome_cancel_reason),
                "user_move_intent_seq": bot.user_move_intent_seq,
                "takeover_watermark": bot.manual_takeover_watermark,
                "destination": [bot.walk_outcome_x, bot.walk_outcome_z, bot.walk_outcome_level],
            })
        });
        serde_json::json!({ "script": script, "nav": nav })
    }
}
