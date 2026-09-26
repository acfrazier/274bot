//! Shared paired full-cycle witness used by the existing paired harness and
//! the opt-in headed pair-watch bridge.
//!
//! This is proof infrastructure, not a second gameplay engine. Headed pair
//! cells are Air, Mule, Flax, and Duel. Duel counterpart identity is native
//! witness-owned; Duel settings carry schema target stats and have no partner
//! key.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::game_data::{DuelControls, SelectedGameData};
use api::snapshot::{GameSnapshot, ItemView, WidgetView};
use client::util::JString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[path = "pair_case.rs"]
mod case;
pub use case::*;
#[path = "pair_runecraft.rs"]
mod runecraft;
pub use runecraft::*;
#[path = "pair_air.rs"]
mod air;
pub use air::*;
#[path = "pair_mule.rs"]
mod mule;
pub use mule::*;
#[path = "pair_flax.rs"]
mod flax;
pub use flax::*;
#[path = "pair_duel.rs"]
mod duel;
pub use duel::*;
#[path = "pair_watch.rs"]
mod watch;
pub use watch::*;

pub fn pair_settings(
    case: PairCase,
    schema: &[script::SettingDef],
    slot_index: usize,
    _self_name: &str,
    partner: &str,
) -> Result<Map<String, Value>, String> {
    let partner = JString::to_screen_name(partner);
    let partner = partner.as_str();
    match (case, slot_index) {
        (PairCase::Air, 0) => Ok(air_settings(schema, AirRole::Master, partner)),
        (PairCase::Air, 1) => Ok(air_settings(schema, AirRole::Runner, partner)),
        (PairCase::Mule, 0) => Ok(mule_settings(schema, MuleRole::Crafter, partner)),
        (PairCase::Mule, 1) => Ok(mule_settings(schema, MuleRole::Mule, partner)),
        (PairCase::Flax, 0) => Ok(flax_settings(schema, FlaxRole::Runner, partner)),
        (PairCase::Flax, 1) => Ok(flax_settings(schema, FlaxRole::Spinner, partner)),
        (PairCase::Duel, _) => Ok(duel_settings(schema)),
        _ => Err(format!(
            "pair settings require two headed slots for {case:?}, got index {slot_index}"
        )),
    }
}
