use std::mem::size_of;
use std::sync::Arc;

use crate::map_cache::ReadyCatalogue;
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::map::formats::{ClientPois, Coverage, ServiceIdentity, ServicePois, MAX_POIS};
use nav::map::identity::{CatalogueIdentity, Digest};
use nav::map::poi::{CapabilityEvidence, Eligibility, EntityKind, PoiKind, PoiRecord, SourceSpace};
use nav::map::{MapError, Text};
use nav::tile::Tile;
use nav::transport::{TransportEdge, TransportKind};
use nav::world::NavWorld;

pub const BANK_API_NOTE: &str = "Map-discovered bank. Frozen bank API roster is separate: nearestBank may omit map discoveries, including Canifis. Walking here does not change that roster or prove banking eligibility.";
const MAX_ENTRIES: usize = 16384;
const INDEX_BUDGET: usize = 1024 * 1024;

/// Construction requires the nav owner's expected identity and whole-file digest;
/// callers cannot promote an unchecked ServicePois to authenticated facts.
pub struct AuthenticatedServices {
    document: Arc<ServicePois>,
    digest: Digest,
}
impl AuthenticatedServices {
    pub fn decode(
        bytes: &[u8],
        expected: ServiceIdentity,
        digest: Digest,
    ) -> Result<Self, MapError> {
        Ok(Self {
            document: Arc::new(ServicePois::decode_navpois(bytes, expected, digest)?),
            digest,
        })
    }
    pub fn document(&self) -> &ServicePois {
        &self.document
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meaning {
    /// A physical entity, never the stand tile itself.
    Access,
    Annotation,
    PlaceLabel,
    Transport,
    TeleportLanding,
}

#[derive(Clone, Copy)]
enum Origin {
    Records {
        client: Option<u16>,
        service: Option<u16>,
    },
    /// Range in the compact sorted edge-index array, not cloned transport data.
    Edges {
        start: u32,
        count: u32,
        teleport: bool,
    },
}
#[derive(Clone, Copy)]
enum NameRef {
    Source,
    Entry(u32),
    Spell(u32),
}
struct Indexed {
    origin: Origin,
    name: NameRef,
    target: Option<Tile>,
    search: Box<str>,
}

enum ClientInput {
    Fixture(Arc<ClientPois>),
    Ready(Arc<ReadyCatalogue>),
}
impl std::ops::Deref for ClientInput {
    type Target = ClientPois;
    fn deref(&self) -> &ClientPois {
        match self {
            Self::Fixture(pois) => pois,
            Self::Ready(ready) => ready.pois(),
        }
    }
}

/// One shared instance per open map, holding Arcs to inputs and tiny reference
/// indices. Closing the surface drops it; no process-static or slot-owned copy.
pub struct Catalogue {
    world: Arc<NavWorld>,
    identity: CatalogueIdentity,
    nav: Digest,
    key: Digest,
    client: Option<ClientInput>,
    services: Option<AuthenticatedServices>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    entries: Vec<Indexed>,
    edges: Vec<u32>,
}
impl Catalogue {
    pub fn new(
        world: Arc<NavWorld>,
        identity: CatalogueIdentity,
        nav: Digest,
        client: Option<Arc<ClientPois>>,
        services: Option<AuthenticatedServices>,
    ) -> Result<Self, MapError> {
        Self::build(
            world,
            identity,
            nav,
            client.map(ClientInput::Fixture),
            services,
        )
    }

    /// Retain D's ready owner rather than cloning its decoded POI payload.
    pub fn from_ready(
        world: Arc<NavWorld>,
        identity: CatalogueIdentity,
        nav: Digest,
        client: Option<Arc<ReadyCatalogue>>,
        services: Option<AuthenticatedServices>,
    ) -> Result<Self, MapError> {
        Self::build(
            world,
            identity,
            nav,
            client.map(ClientInput::Ready),
            services,
        )
    }

    fn build(
        world: Arc<NavWorld>,
        identity: CatalogueIdentity,
        nav: Digest,
        client: Option<ClientInput>,
        services: Option<AuthenticatedServices>,
    ) -> Result<Self, MapError> {
        identity.key()?;
        if let Some(client) = &client {
            client.validate(identity)?;
        }
        if let Some(services) = &services {
            let bound = services.document.identity;
            if bound.revision != identity.revision
                || bound.content != identity.content
                || bound.nav_sha256 != nav
            {
                return Err(MapError::Identity);
            }
        }
        let count = world
            .graph
            .edges
            .len()
            .checked_add(world.graph.teleports.len())
            .ok_or(MapError::Limit("nav overlay count"))?;
        if count > MAX_ENTRIES {
            return Err(MapError::Limit("nav overlay count"));
        }
        let merged = identity.merged_key(nav, services.as_ref().map(|s| s.digest))?;
        // Availability is a runtime generation too: a before-ready selection
        // must not survive the later publication of client records.
        let mut generation = [0u8; 33];
        generation[..32].copy_from_slice(&merged.0);
        generation[32] = u8::from(client.is_some());
        let mut result = Self {
            world,
            identity,
            nav,
            key: Digest::of(&generation),
            client,
            services,
            game_data: None,
            entries: Vec::new(),
            edges: Vec::with_capacity(count),
        };
        result.index_records()?;
        result.index_edges(false)?;
        result.index_edges(true)?;
        result.entries.shrink_to_fit();
        result.edges.shrink_to_fit();
        if result.index_bytes() > INDEX_BUDGET {
            return Err(MapError::Limit("map search/index bytes"));
        }
        Ok(result)
    }
    /// Optional display names from the already bound selected game facts. No
    /// spell/item name or destination roster is invented when these are absent.
    pub fn with_game_data(
        mut self,
        data: Arc<api::game_data::SelectedGameData>,
    ) -> Result<Self, MapError> {
        if data.revision() != i32::from(self.identity.revision)
            || data.content_id().and_then(|id| Digest::from_hex(id).ok())
                != Some(self.identity.content)
        {
            return Err(MapError::Identity);
        }
        for indexed in &mut self.entries {
            let Origin::Edges {
                start,
                count,
                teleport: true,
            } = indexed.origin
            else {
                continue;
            };
            let edges = &self.edges[start as usize..(start + count) as usize];
            if let Some((i, spell)) = data.teleports().iter().enumerate().find(|(_, spell)| {
                edges.iter().any(|&j| {
                    let edge = &self.world.graph.teleports[j as usize];
                    edge.loc_id == 0
                        && edge.to
                            == WorldTile {
                                x: spell.x,
                                z: spell.z,
                                level: spell.plane,
                            }
                })
            }) {
                indexed.name = NameRef::Spell(i as u32);
                let mut search = normalize(&spell.name);
                search.push(' ');
                search.push_str(&indexed.search);
                indexed.search = search.into_boxed_str();
            }
        }
        self.game_data = Some(data);
        let mut generation = [0u8; 33];
        generation[..32].copy_from_slice(&self.key.0);
        generation[32] = 1;
        self.key = Digest::of(&generation);
        if self.index_bytes() > INDEX_BUDGET {
            return Err(MapError::Limit("map search/index bytes"));
        }
        Ok(self)
    }
    pub fn key(&self) -> Digest {
        self.key
    }
    pub fn nav_identity(&self) -> Digest {
        self.nav
    }
    pub fn identity(&self) -> CatalogueIdentity {
        self.identity
    }
    pub fn world(&self) -> &Arc<NavWorld> {
        &self.world
    }
    pub fn client_status(&self) -> SourceStatus {
        if self.client.is_some() {
            SourceStatus::Available
        } else {
            SourceStatus::Unavailable
        }
    }
    pub fn service_status(&self) -> SourceStatus {
        if self.services.is_some() {
            SourceStatus::Available
        } else {
            SourceStatus::Unavailable
        }
    }
    pub fn client_coverage(&self) -> Option<&Coverage> {
        self.client.as_ref().map(|p| &p.coverage)
    }
    pub fn service_coverage(&self) -> Option<&Coverage> {
        self.services.as_ref().map(|p| &p.document.coverage)
    }
    pub fn coverage_messages(&self) -> impl Iterator<Item = &'static str> {
        [
            self.client
                .is_none()
                .then_some("Client POIs unavailable / finish profile cache preparation"),
            self.services
                .is_none()
                .then_some("Tellers and place labels unavailable without navpois"),
        ]
        .into_iter()
        .flatten()
    }
    pub fn entries(&self) -> impl ExactSizeIterator<Item = Entry<'_>> {
        (0..self.entries.len()).map(|index| Entry {
            catalogue: self,
            index,
        })
    }
    pub fn entry(&self, index: usize) -> Option<Entry<'_>> {
        self.entries.get(index).map(|_| Entry {
            catalogue: self,
            index,
        })
    }
    /// Exact owned index/search payload, excluding shared input documents/world.
    pub fn index_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Indexed>()
            + self.edges.capacity() * size_of::<u32>()
            + self.entries.iter().map(|e| e.search.len()).sum::<usize>()
    }
    fn records(&self, origin: Origin) -> [Option<&PoiRecord>; 2] {
        match origin {
            Origin::Records { client, service } => [
                client.map(|i| &self.client.as_ref().unwrap().records.as_slice()[i as usize]),
                service.map(|i| {
                    &self.services.as_ref().unwrap().document.records.as_slice()[i as usize]
                }),
            ],
            _ => [None, None],
        }
    }
    fn index_records(&mut self) -> Result<(), MapError> {
        // At most 8192 small references, never all world LOC placements.
        let clients = self
            .client
            .as_ref()
            .map_or(&[][..], |p| p.records.as_slice());
        let services = self
            .services
            .as_ref()
            .map_or(&[][..], |p| p.document.records.as_slice());
        let mut rows: Vec<(bool, u16)> = (0..clients.len())
            .map(|i| (false, i as u16))
            .chain((0..services.len()).map(|i| (true, i as u16)))
            .collect();
        let record = |(service, i): (bool, u16)| {
            if service {
                &services[i as usize]
            } else {
                &clients[i as usize]
            }
        };
        rows.sort_unstable_by_key(|&i| placement(record(i)));
        let mut i = 0;
        while i < rows.len() {
            let first = record(rows[i]);
            let mut client = None;
            let mut service = None;
            let mut end = i;
            while end < rows.len() && placement(record(rows[end])) == placement(first) {
                let other = record(rows[end]);
                if first.footprint != other.footprint || first.key.source != other.key.source {
                    return Err(MapError::Invalid("conflicting placement provenance"));
                }
                let (is_service, index) = rows[end];
                let slot = if is_service {
                    &mut service
                } else {
                    &mut client
                };
                if slot.replace(index).is_some() {
                    return Err(MapError::Duplicate("normalized placement"));
                }
                end += 1;
            }
            let origin = Origin::Records { client, service };
            let target = if matches!(first.key.entity, EntityKind::Loc | EntityKind::Npc) {
                let supplied = self
                    .records(origin)
                    .into_iter()
                    .flatten()
                    .find_map(|r| r.walk_target);
                match supplied {
                    Some(t)
                        if t.nav_sha256 == self.nav
                            && safe_standable(&self.world, t.tile.into()) =>
                    {
                        Some(tile(t.tile.into()))
                    }
                    Some(_) => return Err(MapError::Invalid("unwalkable POI stand")),
                    None => adjacent_stand(&self.world, first),
                }
            } else {
                let anchor = anchor_tile(first);
                safe_standable(&self.world, world_tile(anchor)).then_some(anchor)
            };
            let mut search = String::new();
            for r in self.records(origin).into_iter().flatten() {
                normalize_into(&mut search, r.name.as_str());
            }
            self.entries.push(Indexed {
                origin,
                name: NameRef::Source,
                target,
                search: search.into_boxed_str(),
            });
            i = end;
        }
        Ok(())
    }
    fn index_edges(&mut self, teleport: bool) -> Result<(), MapError> {
        let source = if teleport {
            &self.world.graph.teleports
        } else {
            &self.world.graph.edges
        };
        let begin = self.edges.len();
        self.edges.extend(
            (0..source.len())
                .filter(|&i| source[i].kind != TransportKind::EssenceExit)
                .map(|i| i as u32),
        );
        self.edges[begin..].sort_unstable_by_key(|&i| edge_key(&source[i as usize], teleport));
        let mut at = begin;
        while at < self.edges.len() {
            let edge = &source[self.edges[at] as usize];
            let mut end = at + 1;
            while end < self.edges.len()
                && edge_key(&source[self.edges[end] as usize], teleport) == edge_key(edge, teleport)
            {
                end += 1;
            }
            if self.entries.len() == MAX_ENTRIES {
                return Err(MapError::Limit("merged POI count"));
            }
            let anchor = if teleport { edge.to } else { edge.at };
            let label = transport_name(edge.kind);
            // Cache/service names are matched by actual definition+placement,
            // not a town-radius join. Unknown nav labels remain honest kinds/IDs.
            let matched = self.entries.iter().position(|e| {
                self.records(e.origin).into_iter().flatten().any(|r| {
                    r.key.id as i64 == i64::from(edge.loc_id)
                        && r.key.x == anchor.x
                        && r.key.z == anchor.z
                        && i32::from(r.effective_plane) == anchor.level
                })
            });
            let name = matched
                .and_then(|i| self.entry(i))
                .map_or(label, |e| e.name());
            let search = normalize(&format!(
                "{name} {label} {} {} {} {}",
                edge.loc_id, anchor.x, anchor.z, anchor.level
            ))
            .into_boxed_str();
            self.entries.push(Indexed {
                name: matched.map_or(NameRef::Source, |i| NameRef::Entry(i as u32)),
                origin: Origin::Edges {
                    start: at as u32,
                    count: (end - at) as u32,
                    teleport,
                },
                target: safe_standable(&self.world, anchor).then_some(tile(anchor)),
                search,
            });
            at = end;
        }
        Ok(())
    }
}

/// Borrowed view; evidence, bank corroboration and requirements are not copied.
#[derive(Clone, Copy)]
pub struct Entry<'a> {
    catalogue: &'a Catalogue,
    index: usize,
}
impl<'a> Entry<'a> {
    pub fn index(self) -> usize {
        self.index
    }
    pub fn records(self) -> impl Iterator<Item = &'a PoiRecord> {
        self.catalogue
            .records(self.data().origin)
            .into_iter()
            .flatten()
    }
    pub fn name(self) -> &'a str {
        match self.data().name {
            NameRef::Entry(i) => return self.catalogue.entry(i as usize).unwrap().name(),
            NameRef::Spell(i) => {
                return &self.catalogue.game_data.as_ref().unwrap().teleports()[i as usize].name
            }
            NameRef::Source => {}
        }
        self.records()
            .last()
            .map(|r| r.name.as_str())
            .unwrap_or_else(|| transport_name(self.transports().next().unwrap().kind))
    }
    pub fn kind(self) -> PoiKind {
        self.records().last().map(|r| r.kind).unwrap_or_else(|| {
            if self.meaning() == Meaning::TeleportLanding {
                PoiKind::Teleport
            } else {
                PoiKind::Transport
            }
        })
    }
    pub fn meaning(self) -> Meaning {
        match self.data().origin {
            Origin::Edges { teleport: true, .. } => Meaning::TeleportLanding,
            Origin::Edges { .. } => Meaning::Transport,
            _ => match self.records().next().unwrap().key.entity {
                EntityKind::Label => Meaning::PlaceLabel,
                EntityKind::MapFunction => Meaning::Annotation,
                _ => Meaning::Access,
            },
        }
    }
    pub fn anchor(self) -> Tile {
        self.records().next().map(anchor_tile).unwrap_or_else(|| {
            let edge = self.transports().next().unwrap();
            tile(if self.meaning() == Meaning::TeleportLanding {
                edge.to
            } else {
                edge.at
            })
        })
    }
    pub fn walk_target(self) -> Option<Tile> {
        self.data().target
    }
    pub fn eligibility(self) -> Eligibility {
        self.records()
            .flat_map(|r| r.evidence.as_slice())
            .filter_map(|e| match e {
                CapabilityEvidence::SourceService { eligibility, .. } => Some(*eligibility),
                _ => None,
            })
            .max_by_key(|e| match e {
                Eligibility::Unknown => 0,
                Eligibility::Conditional => 1,
                Eligibility::Restricted => 2,
            })
            .unwrap_or(Eligibility::Unknown)
    }
    /// A cardinal, collision-valid tile is not proof of a permitted interaction
    /// side. A v1 POI does not carry LocType.forceapproach. WalkTo only walks.
    pub fn approach_known(self) -> bool {
        self.records().any(|r| r.walk_target.is_some())
    }
    pub fn provenance(self) -> &'static str {
        match self.data().origin {
            Origin::Records {
                client: Some(_),
                service: Some(_),
            } => "Client cache + navpois",
            Origin::Records {
                client: Some(_), ..
            } => "Client cache",
            Origin::Records { .. } => "navpois",
            Origin::Edges { .. } => "Navigation pack",
        }
    }
    pub fn bank_note(self) -> Option<&'static str> {
        matches!(self.kind(), PoiKind::Bank).then_some(BANK_API_NOTE)
    }
    /// Legacy packed anchors are corroboration only. Compare their raw plane to
    /// the raw source LOC, not its normalized bridge plane. They never add rows.
    pub fn packed_banks(self) -> impl Iterator<Item = &'a nav::pack::BankStand> {
        self.catalogue.world.banks().iter().filter(move |bank| {
            self.records().any(|r| {
                let raw = match r.key.source {
                    SourceSpace::ClientVisual { plane, .. }
                    | SourceSpace::ServerGame { plane }
                    | SourceSpace::Game { plane } => plane,
                };
                matches!(r.kind, PoiKind::Bank)
                    && bank.tile.x == r.key.x
                    && bank.tile.z == r.key.z
                    && bank.tile.level == i32::from(raw)
                    && matches!(
                        (&bank.access, r.key.entity),
                        (nav::pack::BankAccess::Booth { .. }, EntityKind::Loc)
                            | (nav::pack::BankAccess::Npc { .. }, EntityKind::Npc)
                    )
            })
        })
    }
    pub fn transports(self) -> impl Iterator<Item = &'a TransportEdge> {
        let (indices, source) = match self.data().origin {
            Origin::Edges {
                start,
                count,
                teleport,
            } => (
                &self.catalogue.edges[start as usize..(start + count) as usize],
                if teleport {
                    &self.catalogue.world.graph.teleports
                } else {
                    &self.catalogue.world.graph.edges
                },
            ),
            _ => (&[][..], &self.catalogue.world.graph.edges),
        };
        indices.iter().map(move |&i| &source[i as usize])
    }
    fn data(self) -> &'a Indexed {
        &self.catalogue.entries[self.index]
    }
}

#[derive(Default)]
pub struct Search {
    key: Option<Digest>,
    query: String,
    results: Vec<usize>,
}
impl Search {
    /// Called on edit/publication, not each paint; unchanged input does no work.
    pub fn update(&mut self, catalogue: &Catalogue, query: &str) -> Result<bool, MapError> {
        if query.len() > Text::MAX_BYTES {
            return Err(MapError::Limit("search text"));
        }
        if self.key == Some(catalogue.key) && self.query == query {
            return Ok(false);
        }
        self.query.clear();
        self.query.push_str(query);
        self.key = Some(catalogue.key);
        let needle = normalize(query);
        self.results.clear();
        self.results.extend(
            catalogue
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| e.search.contains(&needle).then_some(i)),
        );
        Ok(true)
    }
    pub fn results(&self) -> &[usize] {
        &self.results
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}
fn normalize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    normalize_into(&mut result, s);
    result
}
fn normalize_into(result: &mut String, s: &str) {
    for word in s.split('/').flat_map(str::split_whitespace) {
        if !result.is_empty() {
            result.push(' ');
        }
        result.extend(word.chars().flat_map(char::to_lowercase));
    }
}
fn placement(r: &PoiRecord) -> (EntityKind, u32, i32, i32, u8, u8, u8) {
    (
        r.key.entity,
        r.key.id,
        r.key.x,
        r.key.z,
        r.effective_plane,
        r.key.shape,
        r.key.rotation,
    )
}
fn edge_key(e: &TransportEdge, teleport: bool) -> (i32, i32, i32, u8, i32) {
    let t = if teleport { e.to } else { e.at };
    (
        t.level,
        t.x,
        t.z,
        if teleport { 0 } else { e.kind as u8 },
        if teleport { 0 } else { e.loc_id },
    )
}
fn transport_name(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Door => "Door",
        TransportKind::Ladder => "Ladder",
        TransportKind::Stairs => "Stairs",
        TransportKind::Boat => "Boat",
        TransportKind::Teleport => "Teleport landing",
        TransportKind::AgilityShortcut => "Agility shortcut",
        TransportKind::Glider => "Gnome glider",
        TransportKind::SpiritTree => "Spirit tree",
        TransportKind::Npc => "NPC transport",
        TransportKind::EssenceExit => "Observed essence exit",
    }
}
pub(super) fn tile(t: WorldTile) -> Tile {
    Tile {
        x: t.x,
        z: t.z,
        level: t.level,
    }
}
pub(super) fn world_tile(t: Tile) -> WorldTile {
    WorldTile {
        x: t.x,
        z: t.z,
        level: t.level,
    }
}
fn anchor_tile(r: &PoiRecord) -> Tile {
    Tile {
        x: r.display.x.floor() as i32,
        z: r.display.z.floor() as i32,
        level: i32::from(r.effective_plane),
    }
}
pub(super) fn safe_standable(world: &NavWorld, t: WorldTile) -> bool {
    // WorldCollision's old APIs subtract i32 coordinates; bound externally
    // supplied map clicks before entering them, including i32::MIN/MAX.
    let dx = i64::from(t.x) - i64::from(world.collision.origin.x);
    let dz = i64::from(t.z) - i64::from(world.collision.origin.z);
    if !(0..4).contains(&t.level)
        || dx < 0
        || dz < 0
        || dx >= world.collision.width as i64
        || dz >= world.collision.height as i64
    {
        return false;
    }
    let index = t.level as usize * world.collision.width * world.collision.height
        + dz as usize * world.collision.width
        + dx as usize;
    index < world.collision.walk.len()
        && index / 64 < world.collision.blocked.len()
        && world.collision.standable(t)
}
fn adjacent_stand(world: &NavWorld, r: &PoiRecord) -> Option<Tile> {
    let x = i64::from(r.key.x);
    let z = i64::from(r.key.z);
    let east = x + i64::from(r.footprint.width);
    let north = z + i64::from(r.footprint.length);
    let candidates = (z..north)
        .flat_map(|z| {
            [
                (x - 1, z, CollisionFlag::W_E as u32),
                (east, z, CollisionFlag::W_W as u32),
            ]
        })
        .chain((x..east).flat_map(|x| {
            [
                (x, z - 1, CollisionFlag::W_N as u32),
                (x, north, CollisionFlag::W_S as u32),
            ]
        }));
    candidates
        .filter_map(|(x, z, face)| {
            let t = Tile {
                x: i32::try_from(x).ok()?,
                z: i32::try_from(z).ok()?,
                level: i32::from(r.effective_plane),
            };
            (safe_standable(world, world_tile(t))
                && world.collision.walkable_word(t.x, t.z, t.level) & face == 0)
                .then_some(t)
        })
        .min_by_key(|t| {
            let dx = (i64::from(t.x) - i64::from(r.key.x)).abs();
            let dz = (i64::from(t.z) - i64::from(r.key.z)).abs();
            (dx.max(dz), dx + dz, t.x, t.z)
        })
}

const _: () = assert!(MAX_POIS <= u16::MAX as usize);
