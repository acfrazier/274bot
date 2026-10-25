//! Session fixtures for front-end tests (see the parent module).
//!
//! Panel and TUI session tests build their vaults here so they never name
//! the crates below the operator seam. The vault adapter values stay private
//! to `frontend-core`; tests use [`FixtureProfile`] builders plus the free
//! functions in this module.

use std::time::Instant;

use super::{FixtureProfile, VaultFixture};
use crate::{OperatorSession, QueuePlace};

/// One vault row's script assignment, as the forms show it.
pub struct AssignmentSnapshot {
    source_kind: String,
    identity: String,
    display_name: String,
    unavailable: Option<String>,
}

impl AssignmentSnapshot {
    /// The assignment's source kind (`catalog`, `compiled`, ...).
    pub fn source_kind(&self) -> &str {
        &self.source_kind
    }

    /// The card identity Start keys on.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The picker display name.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Why a missing card is unavailable, if it is.
    pub fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }
}

/// The stored assignment of `username`, if it has one.
pub fn vault_assignment(fixture: &VaultFixture, username: &str) -> Option<AssignmentSnapshot> {
    let assignment = fixture
        .vault
        .get(username)?
        .settings
        .script_assignment
        .clone()?;
    Some(AssignmentSnapshot {
        source_kind: assignment.source_kind,
        identity: assignment.identity,
        display_name: assignment.display_name,
        unavailable: assignment.unavailable,
    })
}

/// The stored per-card settings bag `key` of `username`, if present.
pub fn vault_script_bag(
    fixture: &VaultFixture,
    username: &str,
    key: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    fixture
        .vault
        .get(username)?
        .settings
        .script_settings
        .get(key)
        .cloned()
}

/// Every stored username, in vault order.
pub fn vault_usernames(fixture: &VaultFixture) -> Vec<String> {
    fixture.usernames()
}

/// Re-write `username`'s row unchanged (a current-writer round trip).
/// Panics when the row is missing: a fixture bug.
pub fn vault_touch(fixture: &mut VaultFixture, username: &str) {
    let row = fixture
        .vault
        .get(username)
        .expect("touch a stored fixture profile")
        .clone();
    fixture.vault.upsert(row).expect("rewrite fixture profile");
}

/// Move `fixture`'s vault into `core` (replacing any unlocked vault).
pub fn install_vault<Io>(core: &mut OperatorSession<Io>, fixture: VaultFixture) {
    core.set_vault(Some(fixture.vault));
}

/// Insert or replace `profile` in `core`'s unlocked vault.
/// Panics when no vault is unlocked: a fixture bug.
pub fn vault_upsert<Io>(core: &mut OperatorSession<Io>, profile: FixtureProfile) {
    core.vault_mut()
        .expect("an unlocked vault")
        .upsert(profile.profile)
        .expect("write fixture profile");
}

/// Stage a script assignment on `profile`.
pub fn with_assignment(
    mut profile: FixtureProfile,
    source_kind: &str,
    identity: &str,
    display_name: &str,
    unavailable: Option<&str>,
) -> FixtureProfile {
    profile.profile.settings.script_assignment = Some(vault::ScriptAssignment {
        source_kind: source_kind.to_string(),
        identity: identity.to_string(),
        display_name: display_name.to_string(),
        unavailable: unavailable.map(str::to_string),
    });
    profile
}

/// Stage one per-card settings bag `key` on `profile`.
pub fn with_settings_bag(
    mut profile: FixtureProfile,
    key: &str,
    bag: serde_json::Map<String, serde_json::Value>,
) -> FixtureProfile {
    profile
        .profile
        .settings
        .script_settings
        .insert(key.to_string(), bag);
    profile
}

impl FixtureProfile {
    /// Spawn this row on `play` with no surface devices (tests only).
    pub fn spawn_on(
        self,
        play: &mut host_play::Play,
        arm: Option<std::sync::Arc<host_play::SlotArm>>,
    ) {
        play.spawn_slot(self.profile, None, None, arm);
    }
}

/// Stage a cached TutSkip result on `profile`.
pub fn with_tutorial_skipped(mut profile: FixtureProfile, skipped: Option<bool>) -> FixtureProfile {
    profile.profile.settings.tutorial_skipped = skipped;
    profile
}

/// Stage the lamp reward skill on `profile`.
pub fn with_lamp_skill(mut profile: FixtureProfile, skill: &str) -> FixtureProfile {
    profile.profile.settings.lamp_skill = skill.to_string();
    profile
}

/// Stage lamp auto-use on `profile`.
pub fn with_lamp_auto(mut profile: FixtureProfile, auto: bool) -> FixtureProfile {
    profile.profile.settings.lamp_auto = auto;
    profile
}

/// Stage incoming random events on `profile`.
pub fn with_random_events(mut profile: FixtureProfile, enabled: bool) -> FixtureProfile {
    profile.profile.settings.random_events = enabled;
    profile
}

/// The settings-bag key for (`source_kind`, `identity`).
pub fn assignment_key(source_kind: &str, identity: &str) -> String {
    vault::assignment_key(source_kind, identity)
}

/// The shared walk-map decode service's admitted capacity (tests).
pub fn decode_service_capacity() -> api::work::WorkCapacity {
    script::work_admission::WorkService::shared()
        .expect("shared work service")
        .lock()
        .capacity()
}

/// A queued place at the head whose wait is another bot logging in.
pub fn queue_head_handshake(position: u32, total: u32) -> QueuePlace {
    handshake_place(position, total)
}

#[cfg(feature = "js")]
/// A script-settings file a test reads and writes (upgrade compat).
pub struct SettingsFixture {
    store: script::ScriptSettingsStore,
}

#[cfg(feature = "js")]
impl SettingsFixture {
    /// Open the settings at `path`.
    pub fn at(path: std::path::PathBuf) -> Self {
        Self {
            store: script::ScriptSettingsStore::at(path),
        }
    }

    /// The stored overrides for (`source`, `name`).
    pub fn overrides(
        &self,
        source: crate::scripts::CardSource,
        name: &str,
    ) -> serde_json::Map<String, serde_json::Value> {
        self.store.overrides(source.to_script(), name)
    }

    /// Stage one override value.
    pub fn set_value(
        &mut self,
        source: crate::scripts::CardSource,
        name: &str,
        id: &str,
        value: serde_json::Value,
    ) {
        self.store.set_value(source.to_script(), name, id, value);
    }

    /// Write staged values.
    pub fn save(&mut self) -> Result<(), String> {
        self.store.save()
    }
}

/// Deterministic debug rows of every stored vault profile, in vault order:
/// a rewrite that preserves every field keeps this vector.
pub fn vault_debug_rows(fixture: &VaultFixture) -> Vec<String> {
    fixture
        .usernames()
        .iter()
        .map(|username| {
            format!(
                "{:?}",
                fixture
                    .vault
                    .get(username)
                    .expect("a stored fixture profile")
            )
        })
        .collect()
}

/// Deterministic debug rows of `core`'s unlocked vault, in vault order.
pub fn core_vault_debug_rows<Io>(core: &OperatorSession<Io>) -> Vec<String> {
    let vault = core.vault().expect("an unlocked vault");
    vault.profiles().map(|row| format!("{row:?}")).collect()
}

/// Lock the vault shut without naming it (tests only).
pub fn clear_vault<Io>(core: &mut OperatorSession<Io>) {
    core.set_vault(None);
}

/// A queued place at the head held by the server until `until`.
pub fn queue_head_server_hold_at(position: u32, total: u32, until: Instant) -> QueuePlace {
    server_hold_place(position, total, until)
}

fn handshake_place(position: u32, total: u32) -> QueuePlace {
    let status = host_play::SlotStatus {
        queue_position: i32::try_from(position).unwrap_or(i32::MAX),
        queue_total: i32::try_from(total).unwrap_or(i32::MAX),
        queue_blocker: Some(host::login_queue::AdmitBlocker::Handshake),
        ..Default::default()
    };
    QueuePlace::of(&status).expect("a valid queue place")
}

fn server_hold_place(position: u32, total: u32, until: Instant) -> QueuePlace {
    let status = host_play::SlotStatus {
        queue_position: i32::try_from(position).unwrap_or(i32::MAX),
        queue_total: i32::try_from(total).unwrap_or(i32::MAX),
        queue_blocker: Some(host::login_queue::AdmitBlocker::ServerHold { until }),
        ..Default::default()
    };
    QueuePlace::of(&status).expect("a valid queue place")
}
