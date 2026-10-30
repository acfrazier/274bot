//! Test-only host receipts and coordination for the ignored Script API C/D cells.
use crate::{script_runtime::script_slot, Play};

impl Play {
    #[doc(hidden)]
    pub fn script_api_live_probe(&self, name: &str) -> serde_json::Value {
        script_slot(&self.scripts, name)
            .and_then(|cell| cell.lock().ok().map(|mut slot| slot.api_live_test_probe()))
            .unwrap_or(serde_json::Value::Null)
    }

    /// The actual slot-owned seat, read without modifying its custody.
    #[doc(hidden)]
    pub fn script_seat_probe(&self, name: &str) -> Option<script::slot::SeatProbe> {
        script_slot(&self.scripts, name)
            .and_then(|cell| cell.lock().ok().and_then(|slot| slot.seat_probe()))
    }
    /// The logical outer compat walk, or its ordinary live follow before handover.
    #[doc(hidden)]
    pub fn script_live_walk_request_id(&self, name: &str) -> Option<u64> {
        if let Some(request_id) = script_slot(&self.scripts, name).and_then(|cell| {
            cell.lock()
                .ok()
                .and_then(|slot| slot.api_live_compat_walk_request_id())
        }) {
            return Some(request_id);
        }
        self.script_live_follow_request_id(name)
    }

    /// The physical follow id, which can differ from its logical outer walk.
    #[doc(hidden)]
    pub fn script_live_follow_request_id(&self, name: &str) -> Option<u64> {
        self.navs.lock().ok()?.get(name).and_then(|bot| {
            (bot.walk_request_id != 0 && bot.walk_work_live()).then_some(bot.walk_request_id)
        })
    }

    /// Dispatch the existing compat `AbortWalk` row for one exact logical walk.
    #[doc(hidden)]
    pub fn script_api_live_abort_walk(&self, name: &str, request_id: u64) -> Result<(), String> {
        if self.script_live_walk_request_id(name) != Some(request_id) {
            return Err("walk request id is not the current logical compat walk".into());
        }
        let cell = script_slot(&self.scripts, name).ok_or("script slot missing")?;
        let slot = cell.lock().map_err(|_| "script slot lock poisoned")?;
        let expression = format!(
            "__rs2b0t_seat_host().interact.push({{op:'abort-walk',request_id:{request_id}}}); true"
        );
        match slot
            .probe(&expression)
            .map_err(|error| error.to_string())?
            .as_bool()
        {
            Some(true) => Ok(()),
            _ => Err("compat AbortWalk did not enqueue".into()),
        }
    }

    /// Signal the fixture consumer after the host observes the real stage-40 cheat
    /// acknowledgment. This does not modify snapshots or perform a game action.
    #[doc(hidden)]
    pub fn script_api_live_read_again(&self, name: &str) -> Result<(), String> {
        let cell = script_slot(&self.scripts, name).ok_or("script slot missing")?;
        let slot = cell.lock().map_err(|_| "script slot lock poisoned")?;
        slot.probe("globalThis.__script_api_read_again = true")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Queue one game row in the real isolate while the guardian owns the frame.
    /// The seat must count and discard it before it can reach the host.
    #[doc(hidden)]
    pub fn script_api_live_queue_held_gameplay(&self, name: &str) -> Result<(), String> {
        let cell = script_slot(&self.scripts, name).ok_or("script slot missing")?;
        let slot = cell.lock().map_err(|_| "script slot lock poisoned")?;
        slot.probe("__rs2b0t_seat_host().interact.push({op:'walk-to',x:0,z:0,level:0}); true")
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
