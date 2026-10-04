//! State and clock controls for the feature-gated manual-click LIVE proofs.

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
                "live_walking_operation": slot.live_walking_operation(),
                "walk_result": slot
                    .probe("globalThis.__manual_walk_result")
                    .unwrap_or(serde_json::Value::Null),
                "v2_reason": slot
                    .probe("globalThis.__manual_v2_reason ?? null")
                    .unwrap_or(serde_json::Value::Null),
                "watchdog_state": format!("{:?}", slot.watchdog().state()),
                "recovering_anchor": slot.watchdog().recovering_anchor().map(|tile| [
                    tile.x, tile.z, tile.level,
                ]),
                "rearm_pending": slot.watchdog().rearm_pending(),
                "idle_ms": slot.progress(std::time::Instant::now()).and_then(|p| p.idle_for)
                    .map(|elapsed| elapsed.as_millis() as u64),
                "terminal_count": slot
                    .probe("globalThis.__manual_terminal_count || 0")
                    .unwrap_or(serde_json::Value::Null),
                "npc_boxes": slot
                    .probe("globalThis.__rs2b0t_host.snapshot.npc_boxes")
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
                "arrival": format!("{:?}", bot.route_arrival),
                "requested_radius": bot.requested_route.as_ref().map(|(_, radius, ..)| *radius),
                "requested_destination": bot.requested_route.as_ref().map(|(tile, ..)| [
                    tile.x, tile.z, tile.level,
                ]),
                "outcome_radius": bot.walk_outcome_radius,
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

    /// Advance the gameplay idle clock for a LIVE recovery trial without
    /// replacing the production watchdog, anchor callback, route or input path.
    #[doc(hidden)]
    pub fn manual_click_live_age_gameplay(
        &self,
        name: &str,
        here: api::snapshot::WorldTile,
    ) -> Result<(), String> {
        if let Some(cell) = script_slot(&self.scripts, name) {
            let slot = cell.lock().unwrap();
            if !matches!(slot.watchdog().state(), script::WatchdogState::Armed) {
                return Ok(());
            }
            let xp: Vec<i32> = serde_json::from_value(
                slot.probe("globalThis.__rs2b0t_host.snapshot.stats.map(row => row.xp)")
                    .map_err(|error| {
                        format!(
                            "{error}; owner={:?}; watchdog={:?}; last_error={:?}",
                            slot.state(),
                            slot.watchdog().state(),
                            slot.last_error(),
                        )
                    })?,
            )
            .map_err(|error| error.to_string())?;
            drop(slot);
            self.manual_click_live_age_gameplay_with_xp(name, here, &xp)?;
        }
        Ok(())
    }

    /// Age the same production watchdog using observed native XP when no
    /// JavaScript isolate exists (compiled cards).
    #[doc(hidden)]
    pub fn manual_click_live_age_gameplay_with_xp(
        &self,
        name: &str,
        here: api::snapshot::WorldTile,
        xp: &[i32],
    ) -> Result<(), String> {
        if let Some(cell) = script_slot(&self.scripts, name) {
            let mut slot = cell.lock().unwrap();
            if !matches!(slot.watchdog().state(), script::WatchdogState::Armed) {
                return Ok(());
            }
            slot.feed_watchdog(
                std::time::Instant::now()
                    - script::watchdog::WEDGE
                    - std::time::Duration::from_secs(1),
                Some((here.x, here.z, here.level)),
                xp,
                false,
                true,
                &[script::shim::InteractReq::NoteProgress],
            );
            // Evaluate the aged clock before live movement can restamp it.
            // Sample the real isolate callback; production still owns ArmWalk.
            let action = slot.feed_watchdog(
                std::time::Instant::now(),
                Some((here.x, here.z, here.level)),
                xp,
                false,
                true,
                &[script::shim::InteractReq::LoopSettled],
            );
            if action == script::WatchdogAction::RequestAnchor {
                slot.request_recovery_anchor();
            }
        }
        Ok(())
    }
}
