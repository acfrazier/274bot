//! Frontend-owned walk-map vocabulary and narrow projections over host map data.
//!
//! Map identity, selection and cache lifetimes remain owned by `host-play`;
//! these wrappers let renderers and operator sessions use them without naming
//! adapter types or retaining host-private facts.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use api::snapshot::{GameSnapshot, WorldTile};
use nav_planner::map::formats::{ClientPois, ServiceIdentity};
use nav_planner::map::identity::{CatalogueIdentity, Digest};
use nav_planner::map::poi::{Eligibility, PoiKind, PoiRecord};
use nav_planner::map::{MapError, Text};
use nav_planner::router::{FindOptions, Route};
use nav_planner::tile::Tile;
use nav_planner::transport::TransportEdge;
use nav_planner::world::NavWorld;

use crate::launch::BoundProfile;

/// The map command binding. Focus is a host token, not a UI identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkMapContext {
    focus: Option<host_play::walk_map::FocusToken>,
    nav: Digest,
    overlay: Option<Digest>,
    generation: u64,
}

impl WalkMapContext {
    pub(crate) fn from_host(context: host_play::walk_map::MapContext) -> Self {
        Self {
            focus: context.focus,
            nav: context.nav,
            overlay: context.overlay,
            generation: context.generation,
        }
    }

    pub(crate) fn to_host(self) -> host_play::walk_map::MapContext {
        host_play::walk_map::MapContext {
            focus: self.focus,
            nav: self.nav,
            overlay: self.overlay,
            generation: self.generation,
        }
    }

    /// Bind a map view without a slot focus, for catalogue-only browsing.
    pub fn browsing(nav: Digest, overlay: Option<Digest>, generation: u64) -> Self {
        Self {
            focus: None,
            nav,
            overlay,
            generation,
        }
    }

    /// Navigation identity bound to this map.
    pub fn nav_identity(self) -> Digest {
        self.nav
    }

    /// POI catalogue identity bound to this map, if available.
    pub fn overlay_identity(self) -> Option<Digest> {
        self.overlay
    }

    /// Update the catalogue identity without changing focus or navigation.
    pub fn with_overlay(mut self, overlay: Option<Digest>) -> Self {
        self.overlay = overlay;
        self
    }

    /// Profile binding generation.
    pub fn generation(self) -> u64 {
        self.generation
    }

    /// Whether this binding contains a live focused slot token.
    pub fn has_focus(self) -> bool {
        self.focus.is_some()
    }

    /// Destination identity ignores the slot focus, as the host map does.
    pub fn same_destination(self, other: Self) -> bool {
        self.to_host().dest_eq(&other.to_host())
    }
}

/// Stable categories for map interaction refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkActionErrorKind {
    NoSelection,
    Blocked,
    NoOrigin,
    OriginNotStandable,
    NoFocus,
    Stale,
    NoNavigation,
    NoPath,
    BlockedByZones,
    InsufficientItems,
    RiskRefused,
    MembersOnly,
    Unauthorized,
    WrongAction,
    InvalidCoordinates,
    RunningScript,
}

/// Typed error projection for a map command. Details remain formatted by the
/// host so wording and route diagnostics stay identical across frontends.
#[derive(Debug)]
pub struct WalkActionError(host_play::walk_map::ActionError);

impl WalkActionError {
    pub(crate) fn from_host(error: host_play::walk_map::ActionError) -> Self {
        Self(error)
    }

    pub(crate) fn into_host(self) -> host_play::walk_map::ActionError {
        self.0
    }

    /// Stable reason code for UI branching.
    pub fn kind(&self) -> WalkActionErrorKind {
        use host_play::walk_map::ActionError as E;
        match self.0 {
            E::NoSelection => WalkActionErrorKind::NoSelection,
            E::Blocked => WalkActionErrorKind::Blocked,
            E::NoOrigin => WalkActionErrorKind::NoOrigin,
            E::OriginNotStandable => WalkActionErrorKind::OriginNotStandable,
            E::NoFocus => WalkActionErrorKind::NoFocus,
            E::Stale => WalkActionErrorKind::Stale,
            E::NoNavigation => WalkActionErrorKind::NoNavigation,
            E::NoPath => WalkActionErrorKind::NoPath,
            E::BlockedByZones { .. } => WalkActionErrorKind::BlockedByZones,
            E::InsufficientItems { .. } => WalkActionErrorKind::InsufficientItems,
            E::RiskRefused { .. } => WalkActionErrorKind::RiskRefused,
            E::MembersOnly => WalkActionErrorKind::MembersOnly,
            E::Unauthorized => WalkActionErrorKind::Unauthorized,
            E::WrongAction => WalkActionErrorKind::WrongAction,
            E::InvalidCoordinates => WalkActionErrorKind::InvalidCoordinates,
            E::RunningScript => WalkActionErrorKind::RunningScript,
        }
    }

    /// Short reason used in group walk reports.
    pub fn short(&self) -> &'static str {
        self.0.short()
    }

    /// Host-supplied diagnostic detail for route refusals, if any.
    pub fn detail(&self) -> Option<&str> {
        use host_play::walk_map::ActionError as E;
        match &self.0 {
            E::BlockedByZones { detail } => detail.as_deref(),
            E::InsufficientItems { detail } | E::RiskRefused { detail, .. } => Some(detail),
            _ => None,
        }
    }
}

impl fmt::Display for WalkActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for WalkActionError {}

/// Walk confirmation or teleport action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkActionKind {
    Walk,
    Teleport,
}

impl WalkActionKind {
    pub(crate) fn to_host(self) -> host_play::walk_map::ActionKind {
        match self {
            Self::Walk => host_play::walk_map::ActionKind::Walk,
            Self::Teleport => host_play::walk_map::ActionKind::Teleport,
        }
    }
}

/// One pending map destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkSelection {
    requested: Tile,
    target: Option<Tile>,
    poi_index: Option<usize>,
}

impl WalkSelection {
    fn from_host(selection: host_play::walk_map::Selection) -> Self {
        Self {
            requested: selection.requested,
            target: selection.target,
            poi_index: selection.poi,
        }
    }

    /// Exact tile selected by the operator.
    pub fn requested(self) -> Tile {
        self.requested
    }

    /// Walkable snapped tile, or `None` when the requested tile is blocked.
    pub fn walk_target(self) -> Option<Tile> {
        self.target
    }

    /// Catalogue row that originated this selection, if any.
    pub fn poi_index(self) -> Option<usize> {
        self.poi_index
    }

    /// The selected destination for a walk receipt, using the requested tile
    /// when a blocked selection will be refused by confirmation.
    pub fn reported_destination(self) -> Tile {
        self.target.unwrap_or(self.requested)
    }
}

/// Map layer toggles exposed to a frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkMapLayers {
    terrain: bool,
    banks: bool,
    transports: bool,
    teleports: bool,
    labels: bool,
    other_pois: bool,
    doors: bool,
    dots: bool,
    collision: bool,
    reach: bool,
}

impl WalkMapLayers {
    fn from_host(layers: host_play::walk_map::Layers) -> Self {
        Self {
            terrain: layers.terrain,
            banks: layers.banks,
            transports: layers.transports,
            teleports: layers.teleports,
            labels: layers.labels,
            other_pois: layers.other_pois,
            doors: layers.doors,
            dots: layers.dots,
            collision: layers.collision,
            reach: layers.reach,
        }
    }

    fn to_host(self) -> host_play::walk_map::Layers {
        host_play::walk_map::Layers {
            terrain: self.terrain,
            banks: self.banks,
            transports: self.transports,
            teleports: self.teleports,
            labels: self.labels,
            other_pois: self.other_pois,
            doors: self.doors,
            dots: self.dots,
            collision: self.collision,
            reach: self.reach,
        }
    }

    pub fn terrain(self) -> bool {
        self.terrain
    }
    pub fn banks(self) -> bool {
        self.banks
    }
    pub fn transports(self) -> bool {
        self.transports
    }
    pub fn teleports(self) -> bool {
        self.teleports
    }
    pub fn labels(self) -> bool {
        self.labels
    }
    pub fn other_pois(self) -> bool {
        self.other_pois
    }
    pub fn doors(self) -> bool {
        self.doors
    }
    pub fn dots(self) -> bool {
        self.dots
    }
    pub fn collision(self) -> bool {
        self.collision
    }
    pub fn reach(self) -> bool {
        self.reach
    }

    pub fn with_terrain(mut self, enabled: bool) -> Self {
        self.terrain = enabled;
        self
    }
    pub fn with_banks(mut self, enabled: bool) -> Self {
        self.banks = enabled;
        self
    }
    pub fn with_transports(mut self, enabled: bool) -> Self {
        self.transports = enabled;
        self
    }
    pub fn with_teleports(mut self, enabled: bool) -> Self {
        self.teleports = enabled;
        self
    }
    pub fn with_labels(mut self, enabled: bool) -> Self {
        self.labels = enabled;
        self
    }
    pub fn with_other_pois(mut self, enabled: bool) -> Self {
        self.other_pois = enabled;
        self
    }
    pub fn with_doors(mut self, enabled: bool) -> Self {
        self.doors = enabled;
        self
    }
    pub fn with_dots(mut self, enabled: bool) -> Self {
        self.dots = enabled;
        self
    }
    pub fn with_collision(mut self, enabled: bool) -> Self {
        self.collision = enabled;
        self
    }
    pub fn with_reach(mut self, enabled: bool) -> Self {
        self.reach = enabled;
        self
    }
}

/// The user-visible map model. Host model state is retained privately.
pub struct WalkMap(host_play::walk_map::MapModel);

impl Default for WalkMap {
    fn default() -> Self {
        Self::new()
    }
}

impl WalkMap {
    pub fn new() -> Self {
        Self(host_play::walk_map::MapModel::default())
    }

    pub fn center(&self) -> [f64; 2] {
        self.0.center
    }
    pub fn set_center(&mut self, center: [f64; 2]) {
        self.0.center = center;
    }
    pub fn scale(&self) -> f64 {
        self.0.scale
    }
    pub fn set_scale(&mut self, scale: f64) {
        self.0.scale = scale;
    }
    pub fn plane(&self) -> u8 {
        self.0.plane
    }
    pub fn layers(&self) -> WalkMapLayers {
        WalkMapLayers::from_host(self.0.layers)
    }
    pub fn set_layers(&mut self, layers: WalkMapLayers) {
        self.0.layers = layers.to_host();
    }

    pub fn bind(&mut self, context: WalkMapContext) -> bool {
        self.0.bind(context.to_host())
    }

    pub fn context(&self) -> Option<WalkMapContext> {
        self.0.context().map(WalkMapContext::from_host)
    }

    pub fn pending(&self) -> Option<WalkSelection> {
        self.0.pending().copied().map(WalkSelection::from_host)
    }

    pub fn clear_selection(&mut self) {
        self.0.clear_selection();
    }
    pub fn close(&mut self) {
        self.0.close();
    }
    pub fn set_plane(&mut self, plane: u8) -> bool {
        self.0.set_plane(plane)
    }
    pub fn recenter(&mut self, observed: Option<Tile>) {
        self.0.recenter(observed);
    }

    pub fn select_tile(&mut self, world: &NavWorld, requested: Tile) -> Option<Tile> {
        self.0.select_tile(world, requested)
    }

    pub fn parse_coordinates(input: &str) -> Result<Tile, WalkActionError> {
        host_play::walk_map::MapModel::parse_coordinates(input).map_err(WalkActionError::from_host)
    }

    pub fn select_poi(
        &mut self,
        catalogue: &WalkCatalogue,
        index: usize,
    ) -> Result<(), WalkActionError> {
        self.0
            .select_poi(&catalogue.0, index)
            .map_err(WalkActionError::from_host)
    }

    pub fn availability(
        &self,
        kind: WalkActionKind,
        current: WalkMapContext,
        origin: Option<Tile>,
    ) -> Result<Tile, WalkActionError> {
        self.0
            .availability(kind.to_host(), &current.to_host(), origin)
            .map_err(WalkActionError::from_host)
    }

    pub(crate) fn confirm(
        &mut self,
        kind: WalkActionKind,
        current: WalkMapContext,
        origin: Option<Tile>,
        options: FindOptions,
    ) -> Result<host_play::walk_map::MapCommand, WalkActionError> {
        self.0
            .confirm(kind.to_host(), &current.to_host(), origin, options)
            .map_err(WalkActionError::from_host)
    }

    pub(crate) fn confirm_walk_plan(
        &mut self,
        current: WalkMapContext,
        options: FindOptions,
    ) -> Result<host_play::walk_map::MapWalkPlan, WalkActionError> {
        self.0
            .confirm_walk_plan(&current.to_host(), options)
            .map_err(WalkActionError::from_host)
    }
}

/// Authenticated navpois input. Bytes are decoded and identity checked before
/// a catalogue can retain the document.
pub struct WalkServices(host_play::walk_map::AuthenticatedServices);

impl WalkServices {
    pub fn decode(
        bytes: &[u8],
        expected: ServiceIdentity,
        digest: Digest,
    ) -> Result<Self, MapError> {
        host_play::walk_map::AuthenticatedServices::decode(bytes, expected, digest).map(Self)
    }
}

/// A shared indexed map catalogue. The host-owned world and authenticated
/// source documents remain shared rather than being copied into the UI.
pub struct WalkCatalogue(host_play::walk_map::Catalogue);

impl WalkCatalogue {
    /// Build a catalogue from explicit, already-decoded map inputs.
    pub fn new(
        world: Arc<NavWorld>,
        identity: CatalogueIdentity,
        nav: Digest,
        client: Option<Arc<ClientPois>>,
        services: Option<WalkServices>,
    ) -> Result<Self, MapError> {
        host_play::walk_map::Catalogue::new(
            world,
            identity,
            nav,
            client,
            services.map(|services| services.0),
        )
        .map(Self)
    }

    pub(crate) fn from_ready(
        world: Arc<NavWorld>,
        profile: &BoundProfile,
        ready: Arc<WalkReadyCatalogue>,
    ) -> Result<(Self, bool), WalkMapCacheError> {
        let identity = ready.identity();
        let nav = profile
            .host()
            .nav_identity()
            .and_then(|identity| Digest::from_hex(&identity.nav_sha256).ok())
            .ok_or_else(|| WalkMapCacheError::message("map catalogue has no nav identity"))?;
        let services = host_play::load_navpois(profile.host(), identity.content, nav);
        let catalogue = host_play::walk_map::Catalogue::from_ready(
            world,
            identity,
            nav,
            Some(Arc::clone(&ready.0)),
            services,
        )
        .map(Self)
        .map_err(WalkMapCacheError::map_error)?;
        match profile.game_data() {
            Some(data) => catalogue
                .0
                .with_game_data(data)
                .map(Self)
                .map(|catalogue| (catalogue, true))
                .map_err(WalkMapCacheError::map_error),
            None => Ok((catalogue, false)),
        }
    }

    pub fn with_game_data(
        self,
        data: Arc<api::game_data::SelectedGameData>,
    ) -> Result<Self, MapError> {
        self.0.with_game_data(data).map(Self)
    }

    pub fn world(&self) -> &Arc<NavWorld> {
        self.0.world()
    }
    pub fn identity(&self) -> CatalogueIdentity {
        self.0.identity()
    }
    pub fn nav_identity(&self) -> Digest {
        self.0.nav_identity()
    }
    pub fn key(&self) -> Digest {
        self.0.key()
    }
    pub fn client_status(&self) -> WalkSourceStatus {
        WalkSourceStatus::from_host(self.0.client_status())
    }
    pub fn service_status(&self) -> WalkSourceStatus {
        WalkSourceStatus::from_host(self.0.service_status())
    }
    pub fn coverage_messages(&self) -> impl Iterator<Item = &'static str> {
        self.0.coverage_messages()
    }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = WalkMapEntry<'_>> {
        self.0.entries().map(WalkMapEntry)
    }
    pub fn entry(&self, index: usize) -> Option<WalkMapEntry<'_>> {
        self.0.entry(index).map(WalkMapEntry)
    }
}

/// One borrowed map catalogue row.
#[derive(Clone, Copy)]
pub struct WalkMapEntry<'a>(host_play::walk_map::Entry<'a>);

impl<'a> WalkMapEntry<'a> {
    pub fn index(self) -> usize {
        self.0.index()
    }
    pub fn records(self) -> impl Iterator<Item = &'a PoiRecord> + 'a {
        self.0.records()
    }
    pub fn name(self) -> &'a str {
        self.0.name()
    }
    pub fn display_name(self) -> WalkDisplayName<'a> {
        WalkDisplayName(self.0.name())
    }
    pub fn kind(self) -> PoiKind {
        self.0.kind()
    }
    pub fn meaning(self) -> WalkMeaning {
        WalkMeaning::from_host(self.0.meaning())
    }
    pub fn anchor(self) -> Tile {
        self.0.anchor()
    }
    pub fn walk_target(self) -> Option<Tile> {
        self.0.walk_target()
    }
    pub fn eligibility(self) -> Eligibility {
        self.0.eligibility()
    }
    pub fn approach_known(self) -> bool {
        self.0.approach_known()
    }
    pub fn provenance(self) -> &'static str {
        self.0.provenance()
    }
    pub fn bank_note(self) -> Option<&'static str> {
        self.0.bank_note()
    }
    pub fn packed_banks(self) -> impl Iterator<Item = &'a nav_planner::pack::BankStand> + 'a {
        self.0.packed_banks()
    }
    pub fn transports(self) -> impl Iterator<Item = &'a TransportEdge> + 'a {
        self.0.transports()
    }
}

/// POI meaning independent of its navigation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkMeaning {
    Access,
    Annotation,
    PlaceLabel,
    Transport,
    TeleportLanding,
}

impl WalkMeaning {
    fn from_host(meaning: host_play::walk_map::Meaning) -> Self {
        match meaning {
            host_play::walk_map::Meaning::Access => Self::Access,
            host_play::walk_map::Meaning::Annotation => Self::Annotation,
            host_play::walk_map::Meaning::PlaceLabel => Self::PlaceLabel,
            host_play::walk_map::Meaning::Transport => Self::Transport,
            host_play::walk_map::Meaning::TeleportLanding => Self::TeleportLanding,
        }
    }
}

/// Whether one catalogue input was available at bind time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkSourceStatus {
    Available,
    Unavailable,
}

impl WalkSourceStatus {
    fn from_host(status: host_play::walk_map::SourceStatus) -> Self {
        match status {
            host_play::walk_map::SourceStatus::Available => Self::Available,
            host_play::walk_map::SourceStatus::Unavailable => Self::Unavailable,
        }
    }
}

/// Catalogue query state. Updating is done on edit/publication, not in paint.
#[derive(Default)]
pub struct WalkSearch(host_play::walk_map::Search);

impl WalkSearch {
    pub fn update(&mut self, catalogue: &WalkCatalogue, query: &str) -> Result<bool, MapError> {
        self.0.update(&catalogue.0, query)
    }
    pub fn results(&self) -> &[usize] {
        self.0.results()
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Stored slash-separated map name formatted for a list or canvas.
#[derive(Clone, Copy, Debug)]
pub struct WalkDisplayName<'a>(&'a str);

impl<'a> WalkDisplayName<'a> {
    pub fn raw(self) -> &'a str {
        self.0
    }
}

impl fmt::Display for WalkDisplayName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&host_play::walk_map::DisplayName::new(self.0), f)
    }
}

/// Append a display form into the caller's reused buffer (`/` becomes space).
pub fn display_walk_name_into(raw: &str, out: &mut String) {
    host_play::walk_map::display_name_into(raw, out);
}

/// One observed NPC service from the focused slot's published snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkObservedService {
    npc_index: usize,
    kind: PoiKind,
    tile: Tile,
}

impl WalkObservedService {
    pub(crate) fn from_host(service: host_play::walk_map::ObservedService) -> Self {
        Self {
            npc_index: service.npc_index,
            kind: service.kind,
            tile: service.tile,
        }
    }
    pub fn npc_index(self) -> usize {
        self.npc_index
    }
    pub fn kind(self) -> PoiKind {
        self.kind
    }
    pub fn tile(self) -> Tile {
        self.tile
    }
}

/// Slot eligibility used by a walk-send checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkSlotStatus {
    origin: Option<Tile>,
    exclusion: Option<WalkExclude>,
}

impl WalkSlotStatus {
    pub(crate) fn from_host(status: host_play::walk_map::WalkSlotStatus) -> Self {
        match status {
            host_play::walk_map::WalkSlotStatus::Eligible(ready) => Self {
                origin: Some(ready.origin),
                exclusion: None,
            },
            host_play::walk_map::WalkSlotStatus::Excluded(reason) => Self {
                origin: None,
                exclusion: Some(WalkExclude::from_host(reason)),
            },
        }
    }
    pub fn eligible(origin: Tile) -> Self {
        Self {
            origin: Some(origin),
            exclusion: None,
        }
    }
    pub fn excluded(reason: WalkExclude) -> Self {
        Self {
            origin: None,
            exclusion: Some(reason),
        }
    }
    pub fn is_eligible(self) -> bool {
        self.origin.is_some()
    }
    pub fn origin(self) -> Option<Tile> {
        self.origin
    }
    pub fn exclusion(self) -> Option<WalkExclude> {
        self.exclusion
    }
}

/// Why a slot cannot take a group Walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkExclude {
    NotLoggedIn,
    NoPosition,
    RunningScript,
}

impl WalkExclude {
    fn from_host(reason: host_play::walk_map::WalkExclude) -> Self {
        match reason {
            host_play::walk_map::WalkExclude::NotLoggedIn => Self::NotLoggedIn,
            host_play::walk_map::WalkExclude::NoPosition => Self::NoPosition,
            host_play::walk_map::WalkExclude::RunningScript => Self::RunningScript,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotLoggedIn => "not logged in",
            Self::NoPosition => "no position yet",
            Self::RunningScript => "running a script (stop the script to include it)",
        }
    }
}

/// Map demand kind requested by a frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkMapDemand {
    CatalogueOnly,
    Images,
    ReadyImages,
}

impl WalkMapDemand {
    pub(crate) fn to_host(self) -> host_play::map_cache::MapDemand {
        match self {
            Self::CatalogueOnly => host_play::map_cache::MapDemand::CatalogueOnly,
            Self::Images => host_play::map_cache::MapDemand::Images,
            Self::ReadyImages => host_play::map_cache::MapDemand::ReadyImages,
        }
    }
    fn from_host(demand: host_play::map_cache::MapDemand) -> Self {
        match demand {
            host_play::map_cache::MapDemand::CatalogueOnly => Self::CatalogueOnly,
            host_play::map_cache::MapDemand::Images => Self::Images,
            host_play::map_cache::MapDemand::ReadyImages => Self::ReadyImages,
        }
    }
}

/// Map-cache error with the original host wording.
#[derive(Debug)]
pub struct WalkMapCacheError(Box<host_play::map_cache::MapCacheError>);

impl WalkMapCacheError {
    pub(crate) fn from_host(error: host_play::map_cache::MapCacheError) -> Self {
        Self(Box::new(error))
    }
    pub(crate) fn map_error(error: MapError) -> Self {
        Self(Box::new(host_play::map_cache::MapCacheError::Map(error)))
    }
    pub(crate) fn message(message: &'static str) -> Self {
        Self(Box::new(host_play::map_cache::MapCacheError::Message(
            message.into(),
        )))
    }
}

impl fmt::Display for WalkMapCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.as_ref(), f)
    }
}
impl std::error::Error for WalkMapCacheError {}

/// Published catalogue lease with the ready directory's pruning lock.
pub struct WalkReadyCatalogue(Arc<host_play::map_cache::ReadyCatalogue>);

impl WalkReadyCatalogue {
    fn from_host(ready: Arc<host_play::map_cache::ReadyCatalogue>) -> Self {
        Self(ready)
    }
    pub fn identity(&self) -> CatalogueIdentity {
        self.0.manifest().identity
    }
}

/// Published terrain images with the ready directory's pruning lock.
pub struct WalkReadyImages(Arc<host_play::map_cache::ReadyImages>);

impl WalkReadyImages {
    fn from_host(ready: Arc<host_play::map_cache::ReadyImages>) -> Self {
        Self(ready)
    }
    /// Wrap an already-opened image fixture for front-end renderer tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_test_fixture(ready: Arc<host_play::map_cache::ReadyImages>) -> Self {
        Self::from_host(ready)
    }
    pub fn image_identity(&self) -> nav_planner::map::identity::ImageIdentity {
        self.0.manifest().identity
    }
    pub fn max_lod(&self) -> u8 {
        self.0.manifest().max_lod
    }
    pub fn read_tile_arc(
        &self,
        key: nav_planner::map::spatial::TileKey,
    ) -> Result<Option<Arc<[u8]>>, WalkMapCacheError> {
        self.0
            .read_tile_arc(key)
            .map_err(WalkMapCacheError::from_host)
    }
}

/// A lazily decoded, identity-verified reach grid from a bound profile.
pub struct WalkReachSource(Arc<host_play::profile::DeferredReach>);

impl WalkReachSource {
    pub(crate) fn from_host(source: Arc<host_play::profile::DeferredReach>) -> Self {
        Self(source)
    }

    /// Decode and verify on first paint demand, then share cached bits.
    pub fn get(&self) -> Option<Arc<[u64]>> {
        self.0.get()
    }
}

/// Stable status variant for a map demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkMapJobState {
    Queued,
    Running,
    Ready,
    Paused,
    Cancelled,
    Failed,
}

/// Worker stage currently reported by a map demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkMapStage {
    ReadingCache,
    DerivingPois,
    BakingPlane,
    BuildingZoomLevels,
    Publishing,
}

/// Non-owning read of the progress attached to one status value.
pub struct WalkMapProgressView<'a> {
    stage: WalkMapStage,
    completed: u32,
    total: u32,
    message: &'a str,
}

impl WalkMapProgressView<'_> {
    pub fn stage(&self) -> WalkMapStage {
        self.stage
    }
    pub fn completed(&self) -> u32 {
        self.completed
    }
    pub fn total(&self) -> u32 {
        self.total
    }
    pub fn message(&self) -> &str {
        self.message
    }
}

/// Current map demand state. Error/progress strings are private and borrowed
/// through accessors so the renderer cannot retain host worker state.
pub struct WalkMapJobStatus {
    state: WalkMapJobState,
    progress: Option<(WalkMapStage, u32, u32, String)>,
    failure: Option<String>,
}

impl WalkMapJobStatus {
    fn from_host(status: host_play::map_cache::MapJobStatus) -> Self {
        use host_play::map_cache::MapJobStatus as H;
        match status {
            H::Queued => Self {
                state: WalkMapJobState::Queued,
                progress: None,
                failure: None,
            },
            H::Running(progress) => Self {
                state: WalkMapJobState::Running,
                progress: Some((
                    WalkMapStage::from_host(progress.stage),
                    progress.completed,
                    progress.total,
                    progress.message,
                )),
                failure: None,
            },
            H::Ready => Self {
                state: WalkMapJobState::Ready,
                progress: None,
                failure: None,
            },
            H::Paused => Self {
                state: WalkMapJobState::Paused,
                progress: None,
                failure: None,
            },
            H::Cancelled => Self {
                state: WalkMapJobState::Cancelled,
                progress: None,
                failure: None,
            },
            H::Failed(error) => Self {
                state: WalkMapJobState::Failed,
                progress: None,
                failure: Some(error),
            },
        }
    }
    pub fn state(&self) -> WalkMapJobState {
        self.state
    }
    pub fn progress(&self) -> Option<WalkMapProgressView<'_>> {
        self.progress
            .as_ref()
            .map(|(stage, completed, total, message)| WalkMapProgressView {
                stage: *stage,
                completed: *completed,
                total: *total,
                message,
            })
    }
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }
}

impl WalkMapStage {
    pub(crate) fn from_host(stage: host_play::map_cache::MapStage) -> Self {
        match stage {
            host_play::map_cache::MapStage::ReadingCache => Self::ReadingCache,
            host_play::map_cache::MapStage::DerivingPois => Self::DerivingPois,
            host_play::map_cache::MapStage::BakingPlane => Self::BakingPlane,
            host_play::map_cache::MapStage::BuildingZoomLevels => Self::BuildingZoomLevels,
            host_play::map_cache::MapStage::Publishing => Self::Publishing,
        }
    }
}

/// A demand lease. Dropping it releases this frontend's demand; the shared
/// worker remains governed by host admission and other consumers.
pub struct WalkMapDemandHandle(Box<host_play::map_cache::MapDemandHandle>);

impl WalkMapDemandHandle {
    pub(crate) fn from_host(handle: host_play::map_cache::MapDemandHandle) -> Self {
        Self(Box::new(handle))
    }
    pub fn duplicate(&self) -> Self {
        Self(self.0.clone())
    }
    pub fn demand(&self) -> WalkMapDemand {
        WalkMapDemand::from_host(self.0.demand())
    }
    pub fn generation(&self) -> Option<u64> {
        self.0.generation()
    }
    pub fn is_ready(&self) -> bool {
        self.0.is_ready()
    }
    pub fn status(&self) -> WalkMapJobStatus {
        WalkMapJobStatus::from_host(self.0.status())
    }
    pub fn cancel(&self) {
        self.0.cancel();
    }
    pub fn ready_catalogue(&self) -> Result<Arc<WalkReadyCatalogue>, WalkMapCacheError> {
        host_play::map_ready_catalogue(&self.0)
            .map(WalkReadyCatalogue::from_host)
            .map(Arc::new)
            .map_err(WalkMapCacheError::from_host)
    }
    pub fn ready_images(&self) -> Result<Option<Arc<WalkReadyImages>>, WalkMapCacheError> {
        host_play::map_ready_images(&self.0)
            .map(|ready| ready.map(WalkReadyImages::from_host).map(Arc::new))
            .map_err(WalkMapCacheError::from_host)
    }
}

/// Current observed services from a slot's published snapshot, bound to the
/// map identity that is open now.
pub fn observed_walk_services(
    snapshot: &GameSnapshot,
    context: WalkMapContext,
) -> Result<Vec<WalkObservedService>, WalkActionError> {
    host_play::walk_map::observed_services(snapshot, context.to_host(), context.to_host())
        .map(|rows| {
            rows.into_iter()
                .map(WalkObservedService::from_host)
                .collect()
        })
        .map_err(WalkActionError::from_host)
}

/// Select the route owner used for map and in-game painting.
pub fn select_walk_route_source(live: bool, script: bool, manual: bool) -> Option<WalkRouteSource> {
    host_play::walk_map::select_route_source(live, script, manual).map(WalkRouteSource::from_host)
}

/// Owner precedence for the route shown by a map or game-frame projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkRouteSource {
    Manual,
    Script,
    Live,
}

impl WalkRouteSource {
    fn from_host(source: host_play::walk_map::RouteSource) -> Self {
        match source {
            host_play::walk_map::RouteSource::Manual => Self::Manual,
            host_play::walk_map::RouteSource::Script => Self::Script,
            host_play::walk_map::RouteSource::Live => Self::Live,
        }
    }

    fn to_host(self) -> host_play::walk_map::RouteSource {
        match self {
            Self::Manual => host_play::walk_map::RouteSource::Manual,
            Self::Script => host_play::walk_map::RouteSource::Script,
            Self::Live => host_play::walk_map::RouteSource::Live,
        }
    }
}

/// Route identity for paint-cache invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkRouteStamp {
    source: WalkRouteSource,
    generation: u64,
    destination_x: i32,
    destination_z: i32,
    destination_level: i32,
}

impl WalkRouteStamp {
    fn from_host(stamp: host_play::walk_map::RouteStamp) -> Self {
        let destination = stamp.destination;
        Self {
            source: WalkRouteSource::from_host(stamp.source),
            generation: stamp.generation,
            destination_x: destination.x,
            destination_z: destination.z,
            destination_level: destination.level,
        }
    }
    pub fn source(self) -> WalkRouteSource {
        self.source
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn destination(self) -> WorldTile {
        WorldTile {
            x: self.destination_x,
            z: self.destination_z,
            level: self.destination_level,
        }
    }
}

/// Borrowed path storage; compact script routes stay compact.
#[derive(Clone, Copy)]
pub enum WalkRoutePath<'a> {
    Legacy(&'a Route),
    Compact(nav_planner::contract::RouteView<'a>),
}

impl WalkRoutePath<'_> {
    pub fn destination(self) -> WorldTile {
        match self {
            Self::Legacy(route) => route.dest,
            Self::Compact(view) => view.destination(),
        }
    }
}

/// Route projection borrowed only for the duration of a frame callback.
#[derive(Clone, Copy)]
pub struct RouteProjectionView<'a> {
    stamp: WalkRouteStamp,
    route: WalkRoutePath<'a>,
    aim: Option<(i32, i32, i32)>,
}

impl<'a> RouteProjectionView<'a> {
    pub(crate) fn from_host(projection: host_play::walk_map::RouteProjection<'a>) -> Self {
        Self {
            stamp: WalkRouteStamp::from_host(projection.stamp),
            route: match projection.route {
                host_play::walk_map::RouteProjectionPath::Legacy(route) => {
                    WalkRoutePath::Legacy(route)
                }
                host_play::walk_map::RouteProjectionPath::Compact(view) => {
                    WalkRoutePath::Compact(view)
                }
            },
            aim: projection.aim.map(|tile| (tile.x, tile.z, tile.level)),
        }
    }
    pub(crate) fn to_host(self) -> host_play::walk_map::RouteProjection<'a> {
        host_play::walk_map::RouteProjection {
            stamp: host_play::walk_map::RouteStamp {
                source: self.stamp.source.to_host(),
                generation: self.stamp.generation,
                destination: self.stamp.destination(),
            },
            route: match self.route {
                WalkRoutePath::Legacy(route) => {
                    host_play::walk_map::RouteProjectionPath::Legacy(route)
                }
                WalkRoutePath::Compact(view) => {
                    host_play::walk_map::RouteProjectionPath::Compact(view)
                }
            },
            aim: self.aim(),
        }
    }
    fn new(
        route: WalkRoutePath<'a>,
        source: WalkRouteSource,
        generation: u64,
        aim: Option<WorldTile>,
    ) -> Self {
        let destination = route.destination();
        Self {
            stamp: WalkRouteStamp {
                source,
                generation,
                destination_x: destination.x,
                destination_z: destination.z,
                destination_level: destination.level,
            },
            route,
            aim: aim.map(|tile| (tile.x, tile.z, tile.level)),
        }
    }
    pub fn stamp(self) -> WalkRouteStamp {
        self.stamp
    }
    pub fn route(self) -> WalkRoutePath<'a> {
        self.route
    }
    pub fn aim(self) -> Option<WorldTile> {
        self.aim.map(|(x, z, level)| WorldTile { x, z, level })
    }
    /// Borrow a compact route under the supplied owner stamp.
    #[cfg(any(test, feature = "test-support"))]
    pub fn compact(
        view: nav_planner::contract::RouteView<'a>,
        source: WalkRouteSource,
        generation: u64,
        aim: Option<WorldTile>,
    ) -> RouteProjectionView<'a> {
        Self::new(WalkRoutePath::Compact(view), source, generation, aim)
    }
    /// Borrow a legacy route under the supplied owner stamp.
    pub fn legacy(
        route: &'a Route,
        source: WalkRouteSource,
        generation: u64,
        aim: Option<WorldTile>,
    ) -> Self {
        Self::new(WalkRoutePath::Legacy(route), source, generation, aim)
    }
}

/// Whether Debug Teleport is authorized for a local loopback launch.
pub fn walk_teleport_authorized(class: crate::launch::ProfileClass, host: &str) -> bool {
    crate::launch::debug_teleport_authorized(class, host)
}

/// Validate that a search query stays within the map text boundary.
pub fn validate_walk_search(query: &str) -> Result<(), MapError> {
    if query.len() > Text::MAX_BYTES {
        Err(MapError::Limit("search text"))
    } else {
        Ok(())
    }
}

/// Peek at a profile's already-published catalogue without starting demand.
pub fn peek_walk_catalogue(profile: &BoundProfile) -> Option<Arc<WalkReadyCatalogue>> {
    host_play::peek_map_catalogue(profile.host())
        .map(WalkReadyCatalogue::from_host)
        .map(Arc::new)
}

/// Default sibling path for the profile's map-flags sidecar.
pub fn default_walk_nav_flags_path() -> PathBuf {
    host_play::default_pack_path().with_extension("navflags")
}

/// Reap cache workers after all map-demand leases have been released.
pub fn reap_walk_map_demand() {
    host_play::reap_map_demand();
}
