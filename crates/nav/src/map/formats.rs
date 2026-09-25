//! V1 disk schemas. Unknown fields, trailing data and duplicate JSON fields fail.
//! Lists are canonical/sorted: consumers validate without auxiliary world-sized sets.
use super::identity::{
    CatalogueIdentity, CatalogueKey, Digest, ImageIdentity, ImageKey, CATALOGUE_SCHEMA,
    IMAGE_SCHEMA,
};
use super::poi::{CapabilityEvidence, EntityKind, PoiRecord, SourceSpace};
use super::spatial::{
    TileKey, WorldBounds, MAX_LOD, TILE_GUTTER, TILE_INTERIOR, TILE_PIXELS, TILE_RGBA_BYTES,
};
use super::{MapError, Rows, Text};
use serde::{Deserialize, Serialize};

pub const MAX_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_POIS: usize = 4096;
pub const MAX_IMAGE_TILES: usize = 8192;
pub const MAX_PNG_BYTES: u32 = 512 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;
pub const NAVPOIS_MAGIC: &[u8; 4] = b"274P";
pub const NAVPOIS_VERSION: u16 = 1;
pub const NAVPOIS_HEADER_BYTES: usize = 77;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadReceipt {
    pub bytes: u32,
    pub sha256: Digest,
}
impl PayloadReceipt {
    pub fn validate(self, limit: u32) -> Result<(), MapError> {
        if self.bytes == 0 {
            return Err(MapError::Invalid("empty payload"));
        }
        if self.bytes > limit {
            return Err(MapError::Limit("payload bytes"));
        }
        Ok(())
    }
    pub fn verify(self, bytes: &[u8], limit: u32) -> Result<(), MapError> {
        self.validate(limit)?;
        if bytes.len() != self.bytes as usize {
            return Err(MapError::Invalid("payload length"));
        }
        if Digest::of(bytes) != self.sha256 {
            return Err(MapError::Digest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageLevel {
    Complete,
    Limited,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageReason {
    MissingNpcPlacements,
    MissingLabels,
    UnresolvedHandler,
    ConditionalService,
    UnknownMapFunction,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageIssue {
    pub reason: CoverageReason,
    pub reference: Text,
    pub count: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub npc_placements: CoverageLevel,
    pub bank_services: CoverageLevel,
    pub place_labels: CoverageLevel,
    pub unresolved: Rows<CoverageIssue, 128>,
}

/// client-pois.json: client-only physical candidates, no account observations,
/// server-only capabilities, route stands or terrain-policy dependency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientPois {
    pub schema: u16,
    pub identity: CatalogueIdentity,
    pub coverage: Coverage,
    pub records: Rows<PoiRecord, MAX_POIS>,
}
impl ClientPois {
    pub fn decode(bytes: &[u8], expected: CatalogueIdentity) -> Result<Self, MapError> {
        super::preflight(
            bytes,
            MAX_JSON_BYTES,
            expected,
            "client-pois",
            CATALOGUE_SCHEMA,
        )?;
        let document: Self = super::json(bytes, MAX_JSON_BYTES)?;
        document.validate(expected)?;
        Ok(document)
    }
    pub fn validate(&self, expected: CatalogueIdentity) -> Result<(), MapError> {
        super::version("client-pois", self.schema, CATALOGUE_SCHEMA)?;
        self.identity.key()?;
        if self.identity != expected {
            return Err(MapError::Identity);
        }
        if self.coverage.npc_placements != CoverageLevel::Unavailable
            || self.coverage.place_labels != CoverageLevel::Unavailable
            || self.coverage.bank_services == CoverageLevel::Complete
        {
            return Err(MapError::Invalid("client-only coverage"));
        }
        validate_records(self.records.as_slice())?;
        for record in self.records.as_slice() {
            if !matches!(record.key.entity, EntityKind::Loc | EntityKind::MapFunction)
                || !matches!(record.key.source, SourceSpace::ClientVisual { .. })
                || record.walk_target.is_some()
                || record.evidence.as_slice().iter().any(|e| {
                    matches!(
                        e,
                        CapabilityEvidence::SourceService { .. }
                            | CapabilityEvidence::SourceLabel { .. }
                    )
                })
            {
                return Err(MapError::Invalid("non-client POI in client catalogue"));
            }
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, MapError> {
        self.validate(self.identity)?;
        encode_json(self)
    }
}

/// manifest.json beside client-pois.json; independently publishable before art.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueManifest {
    pub schema: u16,
    pub identity: CatalogueIdentity,
    pub key: CatalogueKey,
    pub record_count: u32,
    pub payload: PayloadReceipt,
}
impl CatalogueManifest {
    pub fn decode(bytes: &[u8], expected: CatalogueIdentity) -> Result<Self, MapError> {
        let manifest: Self = super::json(bytes, MAX_JSON_BYTES)?;
        super::version("catalogue manifest", manifest.schema, CATALOGUE_SCHEMA)?;
        if manifest.identity != expected || manifest.key != expected.key()? {
            return Err(MapError::Identity);
        }
        if manifest.record_count as usize > MAX_POIS {
            return Err(MapError::Limit("POI count"));
        }
        manifest.payload.validate(MAX_JSON_BYTES as u32)?;
        Ok(manifest)
    }
    pub fn read_payload(&self, bytes: &[u8]) -> Result<ClientPois, MapError> {
        self.payload.verify(bytes, MAX_JSON_BYTES as u32)?;
        let pois = ClientPois::decode(bytes, self.identity)?;
        if pois.records.as_slice().len() != self.record_count as usize {
            return Err(MapError::Invalid("POI receipt count"));
        }
        Ok(pois)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaneBounds {
    pub plane: u8,
    pub bounds: WorldBounds,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorFormat {
    Rgba8Unorm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileReceipt {
    pub key: TileKey,
    pub payload: PayloadReceipt,
}
impl TileReceipt {
    /// Verify compressed bytes and IHDR BEFORE a PNG decoder allocates pixels.
    /// The decoder must still verify PNG chunks/CRC and impose this output size.
    pub fn verify_png(&self, bytes: &[u8]) -> Result<usize, MapError> {
        self.key.bounds()?;
        self.payload.verify(bytes, MAX_PNG_BYTES)?;
        if bytes.len() < 33 {
            return Err(MapError::Truncated);
        }
        if &bytes[..8] != b"\x89PNG\r\n\x1a\n"
            || bytes[8..12] != 13u32.to_be_bytes()
            || &bytes[12..16] != b"IHDR"
        {
            return Err(MapError::Invalid("PNG header"));
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        if width != TILE_PIXELS || height != TILE_PIXELS || bytes[24..29] != [8, 6, 0, 0, 0] {
            return Err(MapError::Invalid(
                "PNG dimensions/format (258x258 RGBA8 noninterlaced)",
            ));
        }
        Ok(TILE_RGBA_BYTES)
    }
}

/// manifest.json for imagery. Bounds and tile lists describe sparse presence;
/// an unlisted group is no art, a missing listed PNG is damage, not collision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageManifest {
    pub schema: u16,
    pub identity: ImageIdentity,
    pub key: ImageKey,
    pub extent: WorldBounds,
    pub planes: Rows<PlaneBounds, 4>,
    pub max_lod: u8,
    pub interior: u32,
    pub gutter: u32,
    pub color: ColorFormat,
    pub tiles: Rows<TileReceipt, MAX_IMAGE_TILES>,
}
impl ImageManifest {
    pub fn decode(bytes: &[u8], expected: ImageIdentity) -> Result<Self, MapError> {
        // Header-only pass: no tile vector or pixel buffer is allocated.
        #[derive(Deserialize)]
        struct Header {
            schema: u16,
            identity: ImageIdentity,
            key: ImageKey,
            max_lod: u8,
            interior: u32,
            gutter: u32,
        }
        let header: Header = super::json(bytes, MAX_JSON_BYTES)?;
        super::version("image manifest", header.schema, IMAGE_SCHEMA)?;
        if header.identity != expected || header.key != expected.key()? {
            return Err(MapError::Identity);
        }
        if header.max_lod > MAX_LOD
            || header.interior != TILE_INTERIOR
            || header.gutter != TILE_GUTTER
        {
            return Err(MapError::Invalid("raster dimensions/LOD"));
        }
        let manifest: Self = super::json(bytes, MAX_JSON_BYTES)?;
        manifest.validate(expected)?;
        Ok(manifest)
    }
    pub fn validate(&self, expected: ImageIdentity) -> Result<(), MapError> {
        super::version("image manifest", self.schema, IMAGE_SCHEMA)?;
        if self.identity != expected || self.key != expected.key()? {
            return Err(MapError::Identity);
        }
        self.extent.validate()?;
        if self.max_lod > MAX_LOD || self.interior != TILE_INTERIOR || self.gutter != TILE_GUTTER {
            return Err(MapError::Invalid("raster dimensions/LOD"));
        }
        let mut previous_plane = None;
        for plane in self.planes.as_slice() {
            plane.bounds.validate()?;
            if plane.plane >= 4 || !self.extent.contains(plane.bounds) {
                return Err(MapError::Invalid("plane bounds"));
            }
            ordered(previous_plane, plane.plane, "plane")?;
            previous_plane = Some(plane.plane);
        }
        if self.planes.as_slice().is_empty() || self.tiles.as_slice().is_empty() {
            return Err(MapError::Invalid("empty imagery"));
        }
        let mut previous = None;
        let mut total = 0u64;
        let mut seen_lods = [0u32; 4];
        for tile in self.tiles.as_slice() {
            ordered(previous, tile.key, "tile key")?;
            previous = Some(tile.key);
            tile.payload.validate(MAX_PNG_BYTES)?;
            total += u64::from(tile.payload.bytes);
            if total > MAX_IMAGE_BYTES {
                return Err(MapError::Limit("image key disk bytes"));
            }
            if tile.key.lod > self.max_lod {
                return Err(MapError::Invalid("tile LOD"));
            }
            let plane = self
                .planes
                .as_slice()
                .iter()
                .find(|p| p.plane == tile.key.plane)
                .ok_or(MapError::Invalid("tile plane"))?;
            if !plane.bounds.intersects(tile.key)? {
                return Err(MapError::Invalid("tile outside plane bounds"));
            }
            seen_lods[usize::from(tile.key.plane)] |= 1 << tile.key.lod;
        }
        // Every represented plane has a base and every coarser level. Sparse
        // holes are legal, but a silently omitted plane/LOD is not publication.
        for plane in self.planes.as_slice() {
            if seen_lods[usize::from(plane.plane)] != (1u32 << (self.max_lod + 1)) - 1 {
                return Err(MapError::Invalid("missing plane/LOD"));
            }
        }
        for tile in self.tiles.as_slice() {
            if tile.key.lod < self.max_lod {
                let parent = TileKey {
                    lod: tile.key.lod + 1,
                    x: tile.key.x.div_euclid(2),
                    z: tile.key.z.div_euclid(2),
                    ..tile.key
                };
                if self.tile(parent).is_none() {
                    return Err(MapError::Invalid("missing parent tile"));
                }
            }
        }
        Ok(())
    }
    pub fn tile(&self, key: TileKey) -> Option<&TileReceipt> {
        self.tiles
            .as_slice()
            .binary_search_by_key(&key, |t| t.key)
            .ok()
            .map(|index| &self.tiles.as_slice()[index])
    }
    pub fn encode(&self) -> Result<Vec<u8>, MapError> {
        self.validate(self.identity)?;
        encode_json(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceIdentity {
    pub revision: u16,
    pub content: Digest,
    pub nav_sha256: Digest,
    pub source_sha256: Digest,
    pub generator_sha256: Digest,
    pub policy: Digest,
}
impl ServiceIdentity {
    /// SHA256("274bot.map.services\0" || version:u16be || revision:u16be ||
    /// content || nav || source || generator || policy), each digest raw 32 bytes.
    pub fn binding(self) -> Result<Digest, MapError> {
        use sha2::{Digest as _, Sha256};
        super::revision(self.revision)?;
        let mut h = Sha256::new();
        h.update(b"274bot.map.services\0");
        h.update(NAVPOIS_VERSION.to_be_bytes());
        h.update(self.revision.to_be_bytes());
        for digest in [
            self.content,
            self.nav_sha256,
            self.source_sha256,
            self.generator_sha256,
            self.policy,
        ] {
            h.update(digest.0);
        }
        Ok(Digest(h.finalize().into()))
    }
}

/// Data-only service supplement. Binary framing deliberately wraps the same
/// typed POI JSON records, not Rust enum discriminants or imagery.
/// Wire: "274P" (4), version:u8 (1), JSON bytes:u32le (4), count:u32le (4),
/// identity-binding (32), JSON SHA256 (32), UTF-8 ServicePois JSON. No trailing data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePois {
    pub schema: u16,
    pub identity: ServiceIdentity,
    pub coverage: Coverage,
    pub records: Rows<PoiRecord, MAX_POIS>,
}
impl ServicePois {
    pub fn validate(&self, expected: ServiceIdentity) -> Result<(), MapError> {
        super::version("navpois", self.schema, NAVPOIS_VERSION)?;
        self.identity.binding()?;
        if self.identity != expected {
            return Err(MapError::Identity);
        }
        validate_records(self.records.as_slice())?;
        for record in self.records.as_slice() {
            match record.key.entity {
                EntityKind::Npc | EntityKind::Label
                    if matches!(record.key.source, SourceSpace::ServerGame { .. }) => {}
                EntityKind::Loc
                    if matches!(record.key.source, SourceSpace::ClientVisual { .. }) => {}
                _ => return Err(MapError::Invalid("service source space")),
            }
            if record
                .walk_target
                .is_some_and(|t| t.nav_sha256 != expected.nav_sha256)
            {
                return Err(MapError::Identity);
            }
        }
        Ok(())
    }
    pub fn encode_navpois(&self) -> Result<Vec<u8>, MapError> {
        self.validate(self.identity)?;
        let body = encode_json(self)?;
        let mut out = Vec::with_capacity(NAVPOIS_HEADER_BYTES + body.len());
        out.extend_from_slice(NAVPOIS_MAGIC);
        out.push(NAVPOIS_VERSION as u8);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.records.as_slice().len() as u32).to_le_bytes());
        out.extend_from_slice(&self.identity.binding()?.0);
        out.extend_from_slice(&Digest::of(&body).0);
        out.extend_from_slice(&body);
        Ok(out)
    }
    /// expected_file_sha256 comes from the authenticated nav identity/manifest,
    /// not from this sidecar's own assertions. Header checks precede POI allocation.
    pub fn decode_navpois(
        bytes: &[u8],
        expected: ServiceIdentity,
        expected_file_sha256: Digest,
    ) -> Result<Self, MapError> {
        if bytes.len() < NAVPOIS_HEADER_BYTES {
            return Err(MapError::Truncated);
        }
        if &bytes[..4] != NAVPOIS_MAGIC {
            return Err(MapError::Invalid("navpois magic"));
        }
        super::version("navpois", u16::from(bytes[4]), NAVPOIS_VERSION)?;
        let length = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(bytes[9..13].try_into().unwrap()) as usize;
        if length > MAX_JSON_BYTES || count > MAX_POIS {
            return Err(MapError::Limit("navpois header"));
        }
        if bytes.len() != NAVPOIS_HEADER_BYTES + length {
            return Err(MapError::Invalid("navpois length"));
        }
        if bytes[13..45] != expected.binding()?.0 {
            return Err(MapError::Identity);
        }
        if Digest::of(bytes) != expected_file_sha256 || bytes[45..77] != Digest::of(&bytes[77..]).0
        {
            return Err(MapError::Digest);
        }
        super::preflight(
            &bytes[77..],
            MAX_JSON_BYTES,
            expected,
            "navpois",
            NAVPOIS_VERSION,
        )?;
        let document: Self = super::json(&bytes[77..], MAX_JSON_BYTES)?;
        document.validate(expected)?;
        if document.records.as_slice().len() != count {
            return Err(MapError::Invalid("navpois count"));
        }
        Ok(document)
    }
}

pub(crate) fn ordered<T: Ord>(
    previous: Option<T>,
    current: T,
    what: &'static str,
) -> Result<(), MapError> {
    if let Some(previous) = previous {
        if previous == current {
            return Err(MapError::Duplicate(what));
        }
        if previous > current {
            return Err(MapError::Invalid("record ordering"));
        }
    }
    Ok(())
}
fn validate_records(records: &[PoiRecord]) -> Result<(), MapError> {
    let mut previous = None;
    for record in records {
        ordered(previous, record.key, "POI key")?;
        previous = Some(record.key);
        record.validate()?;
    }
    Ok(())
}

/// The writer cannot grow beyond the same limit enforced by readers.
pub(crate) fn encode_json(value: &impl Serialize) -> Result<Vec<u8>, MapError> {
    use std::io::{self, Write};
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_JSON_BYTES - self.0.len() {
                return Err(io::Error::other("map JSON byte limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(&mut output, value).map_err(|e| {
        if e.is_io() {
            MapError::Limit("JSON bytes")
        } else {
            MapError::Schema(e)
        }
    })?;
    Ok(output.0)
}
