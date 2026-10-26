//! The operator's consent before a local WalkTo terrain bake.
//!
//! Terrain imagery is served from the map cache when a ready artefact matches
//! the bound client cache's image identity (baked earlier), or installed from
//! the terrain the release ships when that matches the identity exactly.
//! Otherwise opening the map would bake it locally, which costs CPU for about
//! 15 s and up to ~15 MiB once, so [`MapBakeGate`] asks first. Declining keeps
//! the map catalogue-only (POIs, search, grid); the operator can accept later.
//! Catalogue-only demand (the TUI's) and warm opens never ask.
//!
//! The remembered choice is the `map_bake` key of `panel-ui.json`, shared by
//! both front ends; an absent or unknown value means [`MapBakeChoice::Ask`].

use std::error::Error;
use std::fmt;
#[cfg(any(test, feature = "test-support"))]
use std::io;
use std::path::Path;

use host_play::map_cache::{MapDemand, MapDemandManager, MapProfileDescriptor};

use crate::launch::BoundProfile;
use crate::walk_map::{WalkMapCacheError, WalkMapDemand, WalkMapDemandHandle, WalkMapStage};

/// Error while building the release-shipped map bundle.
#[derive(Debug)]
pub struct WalkMapBakeError(String);

impl WalkMapBakeError {
    fn from_error(error: impl Error + 'static) -> Self {
        Self(error.to_string())
    }
}

impl fmt::Display for WalkMapBakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for WalkMapBakeError {}

/// Bake the release-shipped map bundle and return its JSON manifest.
pub fn bake_shipped_map_bundle(
    revision: u16,
    cache: &Path,
    unpack: &Path,
    out: &Path,
    mut notify: impl FnMut(WalkMapStage, &str),
) -> Result<String, WalkMapBakeError> {
    let shipped = host_play::bake_shipped_map_bundle(revision, cache, unpack, out, |status| {
        if let host_play::map_cache::MapJobStatus::Running(progress) = status {
            notify(WalkMapStage::from_host(progress.stage), &progress.message);
        }
    })
    .map_err(WalkMapBakeError::from_error)?;
    serde_json::to_string(&shipped).map_err(WalkMapBakeError::from_error)
}
/// `panel-ui.json` key of the remembered [`MapBakeChoice`].
pub const MAP_BAKE_KEY: &str = "map_bake";

/// Heading of the bake warning, identical in both front ends.
pub const MAP_BAKE_TITLE: &str = "Bake WalkTo map terrain?";

/// Body of the bake warning: what it costs and what declining keeps.
pub const MAP_BAKE_WARNING: &str = "Map terrain for this client cache is not on disk yet. \
Baking it runs once on this machine: CPU for about 15 s and up to ~15 MiB of memory; \
later opens reuse the result. Not now keeps the map catalogue-only (POIs, search, grid) \
and you can bake later.";

/// Remembered answer to the bake warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapBakeChoice {
    /// Warn before every local bake (default, and the value when absent).
    #[default]
    Ask,
    /// Bake without asking.
    Always,
}

impl MapBakeChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Always => "always",
        }
    }

    /// Lossy parse: anything but `always` (e.g. a value written by a newer
    /// release) is [`Self::Ask`], so an upgrade never silently skips the
    /// warning.
    pub fn parse(text: &str) -> Self {
        if text == "always" {
            Self::Always
        } else {
            Self::Ask
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Ask => Self::Always,
            Self::Always => Self::Ask,
        }
    }
}

impl serde::Serialize for MapBakeChoice {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Never fails: a malformed value must not reset the rest of the prefs file.
impl<'de> serde::Deserialize<'de> for MapBakeChoice {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().map(Self::parse).unwrap_or_default())
    }
}

/// The remembered choice from the shared preferences (read once per
/// front-end start).
pub fn load_map_bake_choice(preferences: &crate::prefs::UiPreferences) -> MapBakeChoice {
    preferences
        .document()
        .map(|document| choice_in(&document))
        .unwrap_or_default()
}

/// The remembered choice a preference document holds ([`MapBakeChoice::Ask`]
/// when absent).
pub(crate) fn choice_in(document: &serde_json::Map<String, serde_json::Value>) -> MapBakeChoice {
    document
        .get(MAP_BAKE_KEY)
        .and_then(|value| value.as_str().map(MapBakeChoice::parse))
        .unwrap_or_default()
}

/// Fixture seam: [`persist_map_bake_choice`] for integration tests,
/// durable when it returns.
#[cfg(any(test, feature = "test-support"))]
pub fn persist_map_bake_choice_for_test(
    preferences: &crate::prefs::UiPreferences,
    choice: MapBakeChoice,
) -> io::Result<()> {
    crate::prefs::settled_for_test(persist_map_bake_choice(preferences, choice))
}

/// Write the remembered choice into the shared preferences, keeping every
/// other one. A write still queued when the bounded wait ends comes back
/// [`host_play::local_state::Update::Queued`].
pub(crate) fn persist_map_bake_choice(
    preferences: &crate::prefs::UiPreferences,
    choice: MapBakeChoice,
) -> Result<host_play::local_state::Update, crate::prefs::PreferenceError> {
    preferences.begin_set(MAP_BAKE_KEY, choice.as_str().into())
}

/// Whether the front end shows the bake warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapBakePrompt {
    /// Nothing to show: terrain is ready, baking with consent, or not wanted.
    /// An admitted probe can still be checking disk readiness.
    #[default]
    None,
    /// Show [`MAP_BAKE_WARNING`] with Bake / Always bake / Not now.
    Asking,
    /// The operator said Not now; the map stays catalogue-only and offers
    /// a Bake control.
    Declined,
}

/// Per-process bake consent for one front end. Open every WalkTo map
/// demand through [`Self::open`]; it starts a local terrain bake only with
/// consent (this session's accept, or [`MapBakeChoice::Always`]).
#[derive(Debug, Default)]
pub struct MapBakeGate {
    choice: MapBakeChoice,
    accepted: bool,
    prompt: MapBakePrompt,
}

impl MapBakeGate {
    pub fn new(choice: MapBakeChoice) -> Self {
        Self {
            choice,
            ..Self::default()
        }
    }

    pub fn choice(&self) -> MapBakeChoice {
        self.choice
    }

    /// Update the remembered choice (the caller persists it).
    pub(crate) fn set_choice(&mut self, choice: MapBakeChoice) {
        self.choice = choice;
    }

    pub fn prompt(&self) -> MapBakePrompt {
        self.prompt
    }

    /// Consent to the local bake for the rest of this process; reopen the
    /// demand through [`Self::open`] to start it.
    pub(crate) fn accept(&mut self) {
        self.accepted = true;
        self.prompt = MapBakePrompt::None;
    }

    /// Not now: keep the catalogue-only demand. Later opens stay quiet until
    /// the operator accepts.
    pub(crate) fn decline(&mut self) {
        if self.prompt == MapBakePrompt::Asking {
            self.prompt = MapBakePrompt::Declined;
        }
    }

    /// Fixture seam: [`Self::accept`] for integration tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn accept_for_test(&mut self) {
        self.accept();
    }

    /// Fixture seam: [`Self::decline`] for integration tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn decline_for_test(&mut self) {
        self.decline();
    }

    /// [`Self::open`] against the process map cache for the bound profile.
    pub(crate) fn open_profile(
        &mut self,
        profile: &BoundProfile,
        demand: WalkMapDemand,
    ) -> Result<WalkMapDemandHandle, WalkMapCacheError> {
        let manager = host_play::map_demand_manager().map_err(WalkMapCacheError::from_host)?;
        let descriptor = host_play::map_profile_descriptor(profile.host().as_ref())
            .map_err(WalkMapCacheError::from_host)?;
        self.open(manager, descriptor, demand.to_host())
            .map(WalkMapDemandHandle::from_host)
            .map_err(WalkMapCacheError::from_host)
    }

    /// Open `demand`. With no consent, a known terrain lease is adopted or an
    /// admitted adopt-only probe checks the disk cache. Neither path bakes
    /// terrain. A missing catalogue is derived silently. Once the probe
    /// settles without terrain, `note_ready_without_terrain` raises the prompt.
    pub(crate) fn open(
        &mut self,
        manager: &MapDemandManager,
        descriptor: MapProfileDescriptor,
        demand: host_play::map_cache::MapDemand,
    ) -> Result<host_play::map_cache::MapDemandHandle, host_play::map_cache::MapCacheError> {
        let consent = self.accepted || self.choice == MapBakeChoice::Always;
        if demand != MapDemand::Images || consent {
            if demand == MapDemand::Images {
                self.prompt = MapBakePrompt::None;
            }
            return host_play::open_map_demand(manager, descriptor, demand);
        }
        if let Some(handle) = host_play::open_map_ready_terrain(manager, descriptor.clone())? {
            self.prompt = MapBakePrompt::None;
            return Ok(handle);
        }
        if self.prompt != MapBakePrompt::Declined {
            self.prompt = MapBakePrompt::None;
        }
        host_play::open_map_demand(manager, descriptor, MapDemand::ReadyImages)
    }

    /// Open with a caller-owned fixture cache in test-support builds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_for_test(
        &mut self,
        manager: &MapDemandManager,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<host_play::map_cache::MapDemandHandle, host_play::map_cache::MapCacheError> {
        self.open(manager, descriptor, demand)
    }

    /// An adopt-only demand settled without usable terrain. Raise the prompt
    /// instead of starting a local bake. Cheap enough for every frame.
    pub fn note_ready_without_terrain(&mut self, demand: WalkMapDemand) {
        if demand == WalkMapDemand::ReadyImages {
            self.raise();
        }
    }

    fn raise(&mut self) {
        if self.prompt == MapBakePrompt::None {
            self.prompt = MapBakePrompt::Asking;
        }
    }
}
