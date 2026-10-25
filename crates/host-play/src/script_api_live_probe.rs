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
