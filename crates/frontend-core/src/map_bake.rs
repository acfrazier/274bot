//! The operator's consent before a local WalkTo terrain bake.
//!
//! Terrain imagery is served from the map cache when a ready artefact matches
//! the bound client cache's image identity (baked earlier, or pre-installed).
//! Otherwise opening the map would bake it locally, which costs CPU for about
//! 15 s and up to ~15 MiB once, so [`MapBakeGate`] asks first. Declining keeps
//! the map catalogue-only (POIs, search, grid); the operator can accept later.
//! Catalogue-only demand (the TUI's) and warm opens never ask.
//!
//! The remembered choice is the `map_bake` key of `panel-ui.json`, shared by
//! both front ends; an absent or unknown value means [`MapBakeChoice::Ask`].

use std::io;

use host_play::map_cache::{
    MapCacheError, MapDemand, MapDemandHandle, MapDemandManager, MapProfileDescriptor,
};
use host_play::ServerProfile;

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

/// The remembered choice from `panel-ui.json` (read once per front-end start).
pub fn load_map_bake_choice() -> MapBakeChoice {
    host_play::panel_ui_value(MAP_BAKE_KEY)
        .and_then(|value| value.as_str().map(MapBakeChoice::parse))
        .unwrap_or_default()
}

/// Write the remembered choice into `panel-ui.json`, keeping every other key.
pub fn persist_map_bake_choice(choice: MapBakeChoice) -> io::Result<()> {
    host_play::persist_panel_ui_value(MAP_BAKE_KEY, choice.as_str().into())
}

/// Whether the front end shows the bake warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapBakePrompt {
    /// Nothing to show: terrain is ready, baking with consent, or not wanted.
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
#[derive(Debug, Clone, Default)]
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
    pub fn set_choice(&mut self, choice: MapBakeChoice) {
        self.choice = choice;
    }

    pub fn prompt(&self) -> MapBakePrompt {
        self.prompt
    }

    /// Consent to the local bake for the rest of this process; reopen the
    /// demand through [`Self::open`] to start it.
    pub fn accept(&mut self) {
        self.accepted = true;
        self.prompt = MapBakePrompt::None;
    }

    /// Not now: keep the catalogue-only demand. Later opens stay quiet until
    /// the operator accepts.
    pub fn decline(&mut self) {
        if self.prompt == MapBakePrompt::Asking {
            self.prompt = MapBakePrompt::Declined;
        }
    }

    /// [`Self::open`] against the process map cache for the bound profile.
    pub fn open_profile(
        &mut self,
        profile: &ServerProfile,
        demand: MapDemand,
    ) -> Result<MapDemandHandle, MapCacheError> {
        let manager = host_play::map_demand_manager()?;
        let descriptor = host_play::map_profile_descriptor(profile)?;
        self.open(manager, descriptor, demand)
    }

    /// Open `demand`. Only terrain readiness decides the prompt: ready
    /// terrain opens the images demand (a missing or stale catalogue is
    /// derived silently, it is cheap); terrain that would need a local bake
    /// without consent returns a catalogue-only handle and raises the prompt.
    pub fn open(
        &mut self,
        manager: &MapDemandManager,
        descriptor: MapProfileDescriptor,
        demand: MapDemand,
    ) -> Result<MapDemandHandle, MapCacheError> {
        let consent = self.accepted || self.choice == MapBakeChoice::Always;
        if demand == MapDemand::Images && !consent && !manager.images_ready(&descriptor)? {
            if self.prompt == MapBakePrompt::None {
                self.prompt = MapBakePrompt::Asking;
            }
            return host_play::open_map_demand(manager, descriptor, MapDemand::CatalogueOnly);
        }
        if demand == MapDemand::Images {
            self.prompt = MapBakePrompt::None;
        }
        host_play::open_map_demand(manager, descriptor, demand)
    }
}
