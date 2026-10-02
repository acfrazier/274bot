//! Test-only host receipts and coordination for the ignored Script API C/D cells.
use crate::{script_runtime::script_slot, Play};

impl Play {
    #[doc(hidden)]
    pub fn script_api_live_probe(&self, name: &str) -> serde_json::Value {
        script_slot(&self.scripts, name)
            .and_then(|cell| cell.lock().ok().map(|mut slot| slot.api_live_test_probe()))
            .unwrap_or(serde_json::Value::Null)
    }

    /// Signal the fixture consumer after the host observes the real stage-40 cheat
    /// acknowledgment. This does not modify snapshots or perform a game action.
    #[doc(hidden)]
    pub fn script_api_live_read_again(&self, name: &str) -> Result<(), String> {
        let cell = script_slot(&self.scripts, name).ok_or("script slot missing")?;
        let slot = cell.lock().map_err(|_| "script slot lock poisoned")?;
        slot.probe("globalThis.__script_api_read_again = true")?;
        Ok(())
    }
}
