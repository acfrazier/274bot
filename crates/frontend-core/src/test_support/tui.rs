//! Front-end test fixtures (see the parent module): the TUI's session,
//! script and live tests.
//!
//! These tests drive a `host-play` [`Play`] and its slot arms next to script,
//! vault and registry facts. The functions here read and write those facts
//! through the front-end vocabulary, so the TUI tests never name the crates
//! below the operator seam.

use std::collections::BTreeMap;
use std::sync::Arc;

use host_play::{Play, SlotArm};
use serde_json::{Map, Value};

use super::VaultFixture;
use crate::frames::FrameHook;
use crate::launch::PlayLaunch;
use crate::scripts::{CardSource, LoadShape, ScriptRunState, Scripts};
use crate::{GuardianPolicy, OperatorSession};

/// Start the play `launch` describes, with `hook` as every slot's per-frame
/// client work, and adopt it with `vault`.
pub fn start_play_with_vault<Io>(
    core: &mut OperatorSession<Io>,
    vault: VaultFixture,
    launch: PlayLaunch,
    hook: Arc<dyn FrameHook>,
) -> Result<(), String> {
    core.start_play(vault.vault, launch, hook)
}

/// `name`'s script lifecycle on a bare `play`; `Idle` without a script.
pub fn play_script_state(play: &Play, name: &str) -> ScriptRunState {
    ScriptRunState::from_script(play.script_state(name))
}

/// Start `source` as a `shape` bot on `name`'s slot, with no settings bag
/// and no sibling modules: the load path below the script coordinator.
pub fn start_script_source(
    play: &Play,
    name: &str,
    source: String,
    shape: LoadShape,
) -> Result<(), String> {
    play.script_start_load(name, source, shape.to_script(), None, Vec::new())
}

/// The response `arm` applies to the hostile random `event`. Panics when the
/// arm holds none for it: a fixture bug.
pub fn arm_random_event_policy(arm: &SlotArm, event: &str) -> GuardianPolicy {
    let policies = arm.random_event_policies.lock().unwrap();
    GuardianPolicy::from_vault(policies[event])
}

/// The saved per-card settings bag `key` of `username`; `None` without a bag
/// for the card. Panics without an unlocked vault or the profile: a fixture
/// bug.
pub fn saved_script_bag<Io>(
    core: &OperatorSession<Io>,
    username: &str,
    key: &str,
) -> Option<Map<String, Value>> {
    core.vault()
        .expect("an unlocked vault")
        .get(username)
        .expect("a saved profile")
        .settings
        .script_settings
        .get(key)
        .cloned()
}

/// One profile's saved settings per card, by card identity key.
type CardSettings = BTreeMap<String, Map<String, Value>>;

/// Every saved profile's per-card settings maps, by username in vault order.
/// Panics without an unlocked vault: a fixture bug.
pub fn saved_script_settings<Io>(core: &OperatorSession<Io>) -> Vec<(String, CardSettings)> {
    core.vault()
        .expect("an unlocked vault")
        .profiles()
        .map(|row| (row.username.clone(), row.settings.script_settings.clone()))
        .collect()
}

/// Delete `username` from the unlocked vault without a command, so the
/// session's bound row is gone. Panics without an unlocked vault or when the
/// vault cannot be rewritten: a fixture bug.
pub fn remove_saved_profile<Io>(core: &mut OperatorSession<Io>, username: &str) {
    core.vault_mut()
        .expect("an unlocked vault")
        .remove(username)
        .expect("rewrite the vault");
}

/// The API family (`unversioned`, `v1`, `v2`) the library classified the card
/// `lookup` against; `None` when no such card is loaded.
pub fn card_api_family(scripts: &Scripts, source: CardSource, lookup: &str) -> Option<String> {
    scripts
        .library()
        .get(source.to_script(), lookup)
        .map(|card| card.api_family.as_str().to_owned())
}

/// Ids of the process-wide Quester Path registry's rows read from the
/// configured folder.
pub fn quester_folder_path_ids() -> Vec<String> {
    quester_path_ids(script::quester::registry::PathSource::Folder)
}

/// Ids of the process-wide Quester Path registry's draft rows.
pub fn quester_draft_path_ids() -> Vec<String> {
    quester_path_ids(script::quester::registry::PathSource::Draft)
}

fn quester_path_ids(source: script::quester::registry::PathSource) -> Vec<String> {
    script::quester::registry::snapshot()
        .rows()
        .iter()
        .filter(|row| row.source == source)
        .map(|row| row.id.clone())
        .collect()
}
