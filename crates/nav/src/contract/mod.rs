//! Semantic nav facade (F0 item 4: contract §23 §1.1, D11).
//!
//! Views and leases wrap the current backend (`arrival`, `collision`,
//! `router`, `transport`, `zones`) without exposing storage layouts.
//! A re-export of [`crate::collision::WorldCollision`], [`crate::router::Route`],
//! [`crate::transport::TransportEdge`] or [`crate::zones::ZoneTable`] is not
//! this surface. Production import migration is T5.1 alone.
//!
//! Small semantic values stay stable and are re-exported: [`ArrivalKind`],
//! [`ZoneKey`], [`ZoneClass`], [`TransportKind`], [`DoorDir`], [`AvoidRect`],
//! [`MAX_ZONE_KEYS`], [`AVOID_CATALOG_IDS`]. Zone exemptions are pack-bound
//! [`BoundZoneExempt`] values; raw [`crate::zones::ZoneExempt`] is not part
//! of this surface. Coordinates are [`api::snapshot::WorldTile`].

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::world::NavWorld;

mod arrival;
mod collision;
mod envelope;
mod route;
mod transport;
mod zones;

pub use arrival::ArrivalSpec;
pub use collision::CollisionView;
pub use envelope::{
    BoundRouteRequest, RouteFamilyError, RouteFamilyResult, RoutePermissions, RoutePhase,
    RouteRequest, RouteService,
};
pub use route::{LegView, RoutePlanLease, RouteView};
pub use transport::TransportView;
pub use zones::{
    BoundZoneExempt, BoundZoneKey, WildernessView, ZoneKindView, ZoneTableView, ZoneView,
};

pub use crate::arrival::ArrivalKind;
pub use crate::pack::{FORMAT_ID, VERSION};
pub use crate::router::AvoidRect;
pub use crate::transport::{DoorDir, TransportKind};
pub use crate::zones::{ZoneClass, ZoneKey, AVOID_CATALOG_IDS, MAX_ZONE_KEYS};

/// SHA-256 digest of pack content. Two leases of the same bytes share this
/// identity and still have distinct [`PackId`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackDigest([u8; 32]);

impl PackDigest {
    /// Wrap an already-computed 32-byte digest (the owner-boundary
    /// `nav_sha256` / `nav::pack` hash).
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The 32 content bytes.
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    /// Digest `bytes` with SHA-256.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }
}

/// Opaque identity of one immutable pack lease. A numeric zone or route
/// index is meaningless without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PackId(NonZeroU64);

impl PackId {
    fn mint() -> Result<Self, ContractError> {
        static LAST: AtomicU64 = AtomicU64::new(0);
        match LAST.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        }) {
            Ok(previous) => NonZeroU64::new(previous + 1)
                .map(Self)
                .ok_or(ContractError::PackIdExhausted),
            Err(_) => Err(ContractError::PackIdExhausted),
        }
    }

    /// The raw lease id. Not a table index.
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Nav revision/content identity carried by every view and plan lease:
/// the lease id, the content digest, and the pack format/version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavIdentity {
    pack: PackId,
    content: PackDigest,
    format: &'static str,
    version: u8,
}

impl NavIdentity {
    /// Mint a fresh lease identity for `content`. One per loaded world
    /// (`crate::producer`) or per [`NavLease::wrap`] call.
    pub(crate) fn mint(content: PackDigest) -> Result<Self, ContractError> {
        Ok(Self {
            pack: PackId::mint()?,
            content,
            format: FORMAT_ID,
            version: VERSION,
        })
    }

    /// The originating pack lease. Distinct even when content matches.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Content digest shared by every lease of the same bytes.
    pub const fn content(self) -> PackDigest {
        self.content
    }

    /// Pack format identity (`274V16` today).
    pub const fn format(self) -> &'static str {
        self.format
    }

    /// Pack format version byte (`16` today).
    pub const fn version(self) -> u8 {
        self.version
    }
}

/// Why a facade query was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// A view, plan or bound key was used against another pack.
    #[error("nav view or index belongs to another pack")]
    PackMismatch,
    /// The zone or group key is not present in this pack.
    #[error("zone or group key is unknown in this pack")]
    UnknownKey,
    /// The requested leg does not exist on this plan.
    #[error("route leg index is out of range")]
    NoSuchLeg,
    /// Pack lease ids exhausted; never reuse.
    #[error("nav pack lease identity space exhausted")]
    PackIdExhausted,
    /// Area arrival cannot name a loc origin (`native/walk.rs`: `(Area, Some(loc))`).
    #[error("area arrival cannot carry a loc identity")]
    AreaWithLoc,
    /// A Reach that names a loc is answered by the loaded scene
    /// (`walk_loc_arrived`), never by the packed bake.
    #[error("loc arrival needs the loaded scene, not the bake")]
    LocArrivalNeedsScene,
    /// More than [`MAX_ZONE_KEYS`] named exemptions.
    #[error("more than {MAX_ZONE_KEYS} zone exemptions")]
    TooManyExemptions,
}

/// O(1) graph counts needed to reserve search scratch.
///
/// Computed once when a lease is prepared and copied with each lease handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SearchGraphEnvelope {
    pub(crate) carry_rows: usize,
    pub(crate) return_rows: usize,
    pub(crate) worn_rows: usize,
    pub(crate) max_worn_ids_per_edge: usize,
    pub(crate) max_quest_gates: usize,
    representable: bool,
}

impl Default for SearchGraphEnvelope {
    fn default() -> Self {
        Self {
            carry_rows: 0,
            return_rows: 0,
            worn_rows: 0,
            max_worn_ids_per_edge: 0,
            max_quest_gates: 0,
            representable: true,
        }
    }
}

impl SearchGraphEnvelope {
    pub(crate) fn from_graph(graph: &crate::transport::TransportGraph) -> Self {
        let mut envelope = Self::default();
        for edge in graph.edges.iter().chain(&graph.teleports) {
            if envelope
                .include_edge(
                    edge.item_req.len(),
                    edge.consumed_req.len(),
                    edge.item_returns.len(),
                    edge.worn_req.len(),
                    edge.quest_gates
                        .as_ref()
                        .map_or(0, |gates| gates.gates().len()),
                )
                .is_none()
            {
                envelope.representable = false;
                break;
            }
        }
        envelope
    }

    pub(crate) const fn is_representable(self) -> bool {
        self.representable
    }

    fn include_edge(
        &mut self,
        item_req_rows: usize,
        consumed_req_rows: usize,
        return_rows: usize,
        worn_ids: usize,
        quest_gates: usize,
    ) -> Option<()> {
        if !self.representable {
            return None;
        }
        let carry_rows = self
            .carry_rows
            .checked_add(item_req_rows)?
            .checked_add(consumed_req_rows)?
            .checked_add(return_rows)?;
        let return_rows_total = self.return_rows.checked_add(return_rows)?;
        let worn_rows = self.worn_rows.checked_add(worn_ids)?;
        let max_quest_gates = self.max_quest_gates.checked_add(quest_gates)?;

        self.carry_rows = carry_rows;
        self.return_rows = return_rows_total;
        self.worn_rows = worn_rows;
        self.max_worn_ids_per_edge = self.max_worn_ids_per_edge.max(worn_ids);
        self.max_quest_gates = max_quest_gates;
        Some(())
    }
}

/// Immutable pack lease. Concrete views borrow it; they cannot outlive it
/// or index a different pack. Not a per-tick clone and not a heap-allocated
/// trait object.
///
/// [`Clone`] clones the `Arc` handle and copies the checked scalar graph
/// envelope; the bake is not copied. The production owner reuses one prepared
/// lease across a cooperative cursor's phases.
#[derive(Clone)]
pub struct NavLease {
    identity: NavIdentity,
    search_envelope: SearchGraphEnvelope,
    world: Arc<NavWorld>,
}

impl NavLease {
    /// Wrap a loaded world as an immutable lease with its content digest.
    /// Every call mints a distinct lease identity; the host's one shared
    /// identity per loaded world is [`crate::producer::lease`].
    pub fn wrap(world: Arc<NavWorld>, content: PackDigest) -> Result<Self, ContractError> {
        let identity = NavIdentity::mint(content)?;
        let search_envelope = SearchGraphEnvelope::from_graph(&world.graph);
        Ok(Self {
            identity,
            search_envelope,
            world,
        })
    }

    /// A lease over `world` under an identity the owner boundary already
    /// minted for it (`crate::producer`).
    pub(crate) fn with_identity(world: Arc<NavWorld>, identity: NavIdentity) -> Self {
        let search_envelope = world.search_graph_envelope();
        Self {
            identity,
            search_envelope,
            world,
        }
    }

    /// Pack/format/content identity.
    pub const fn identity(&self) -> NavIdentity {
        self.identity
    }
    pub(crate) fn graph(&self) -> &crate::transport::TransportGraph {
        &self.world.graph
    }
    pub(crate) const fn search_envelope(&self) -> SearchGraphEnvelope {
        self.search_envelope
    }
    /// Route cursors borrow the same collision kernel as synchronous searches.
    pub(crate) fn collision_world(&self) -> &crate::collision::WorldCollision {
        &self.world.collision
    }

    /// Borrowed collision view over this lease's packed bake.
    pub fn collision(&self) -> CollisionView<'_> {
        CollisionView::new(self.identity.pack, &self.world.collision)
    }

    /// Borrowed zone table, or `None` on a legacy/synthetic world with no
    /// packed zones (unknown/refusal).
    pub fn zones(&self) -> Option<ZoneTableView<'_>> {
        self.world
            .graph
            .zones
            .as_ref()
            .map(|table| ZoneTableView::new(self.identity.pack, table))
    }

    /// Borrowed wilderness membership/level facts.
    pub fn wilderness(&self) -> WildernessView<'_> {
        WildernessView::new(&self.world.graph.wilderness)
    }
}

impl std::fmt::Debug for NavLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavLease")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;

#[cfg(test)]
mod search_graph_envelope_tests {
    use super::SearchGraphEnvelope;

    #[test]
    fn envelope_uses_checked_totals_and_per_edge_maxima() {
        let mut envelope = SearchGraphEnvelope::default();
        envelope.include_edge(2, 3, 5, 7, 11).unwrap();
        envelope.include_edge(1, 2, 3, 9, 13).unwrap();
        assert_eq!(
            envelope,
            SearchGraphEnvelope {
                carry_rows: 16,
                return_rows: 8,
                worn_rows: 16,
                max_worn_ids_per_edge: 9,
                max_quest_gates: 24,
                representable: true,
            }
        );
    }

    #[test]
    fn envelope_rejects_overflow_without_partial_updates() {
        let mut envelope = SearchGraphEnvelope::default();
        envelope
            .include_edge(usize::MAX, 0, 0, usize::MAX, usize::MAX)
            .unwrap();
        let before = envelope;
        assert!(envelope.include_edge(1, 0, 0, 0, 1).is_none());
        assert_eq!(envelope, before);
    }
    #[test]
    fn envelope_rejects_worn_total_overflow_without_partial_updates() {
        let mut envelope = SearchGraphEnvelope::default();
        envelope.include_edge(0, 0, 0, usize::MAX, 0).unwrap();
        let before = envelope;
        assert!(envelope.include_edge(0, 0, 0, 1, 0).is_none());
        assert_eq!(envelope, before);
    }

    #[test]
    fn envelope_counts_graph_edges_and_teleports() {
        let mut graph = crate::transport::TransportGraph::default();
        graph.edges.push(crate::transport::TransportEdge { takeoff: None, worn_all_req: Vec::new(), forbidden_held: Vec::new(), forbidden_worn: Vec::new(), kind: crate::transport::TransportKind::Door,
        player_delta: None,
        at: api::snapshot::WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: api::snapshot::WorldTile {
            x: 1,
            z: 0,
            level: 0,
        },
        loc_id: 0,
        option: 0,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: Vec::new(),
        item_req: Vec::new(),
        consumed_req: Vec::new(),
        item_returns: Vec::new(),
        quest_req: Vec::new(),
        varp_req: Vec::new(),
        worn_req: vec![10, 11],
        members_req: false,
        wildy_cap: None,
        quest_gates: None, });
        let mut teleport = graph.edges[0].clone();
        teleport.kind = crate::transport::TransportKind::Teleport;
        teleport.worn_req = vec![11, 12, 13];
        graph.teleports.push(teleport);
        let envelope = SearchGraphEnvelope::from_graph(&graph);
        assert_eq!(envelope.worn_rows, 5);
        assert_eq!(envelope.max_worn_ids_per_edge, 3);
    }

    #[test]
    fn empty_graph_has_zero_envelope() {
        assert_eq!(
            SearchGraphEnvelope::from_graph(&crate::transport::TransportGraph::default()),
            SearchGraphEnvelope::default()
        );
    }
}
