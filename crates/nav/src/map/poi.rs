//! Discovery evidence is not route/account eligibility and never an action.
use super::identity::Digest;
use super::spatial::GameTile;
use super::{MapError, Rows, Text};
use serde::{Deserialize, Serialize};

/// Source tagging prevents a server NPC or an already-normalized route anchor
/// from accidentally receiving the raw visual LOC shift a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "space", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceSpace {
    ClientVisual { plane: u8, link_below: bool },
    ServerGame { plane: u8 },
    Game { plane: u8 },
}
impl SourceSpace {
    pub fn game_plane(self) -> Result<Option<u8>, MapError> {
        let plane = match self {
            Self::ClientVisual { plane, .. }
            | Self::ServerGame { plane }
            | Self::Game { plane } => plane,
        };
        if plane >= 4 {
            return Err(MapError::Invalid("source plane"));
        }
        Ok(match self {
            Self::ClientVisual { link_below, .. } => {
                crate::collision::game_plane(i32::from(plane), link_below).map(|p| p as u8)
            }
            Self::ServerGame { .. } | Self::Game { .. } => Some(plane),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Loc,
    Npc,
    MapFunction,
    Label,
    Transport,
    Teleport,
}

/// Document identity + this key identifies a physical source placement. Shape
/// and rotation distinguish colocated LOC placements. NPCs/labels use zeroes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoiKey {
    pub entity: EntityKind,
    pub id: u32,
    pub x: i32,
    pub z: i32,
    pub source: SourceSpace,
    pub shape: u8,
    pub rotation: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PoiKind {
    Bank,
    Shop,
    Altar,
    MapSymbol { symbol: u16 },
    Label { priority: u8 },
    Transport,
    Teleport,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Bank,
    Trade,
    Pray,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Eligibility {
    Unknown,
    Conditional,
    Restricted,
}

/// Real one-based cache operation slot, never a guessed default for banks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct OperationSlot(u8);
impl OperationSlot {
    pub fn new(slot: u8) -> Result<Self, MapError> {
        if !(1..=5).contains(&slot) {
            return Err(MapError::Invalid("operation slot"));
        }
        Ok(Self(slot))
    }
    pub fn get(self) -> u8 {
        self.0
    }
}
impl TryFrom<u8> for OperationSlot {
    type Error = MapError;
    fn try_from(v: u8) -> Result<Self, MapError> {
        Self::new(v)
    }
}
impl From<OperationSlot> for u8 {
    fn from(v: OperationSlot) -> u8 {
        v.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "trigger", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceTrigger {
    Operation { slot: OperationSlot },
    Dialogue,
    LocVariant { definition: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "evidence", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityEvidence {
    ClientOperation {
        capability: Capability,
        slot: OperationSlot,
    },
    MapFunction {
        symbol: u16,
    },
    /// Display/access candidate, NOT proof that Use-quickly opens a bank.
    ActiveQuickBooth,
    SourceService {
        capability: Capability,
        trigger: ServiceTrigger,
        eligibility: Eligibility,
        /// Content-root-relative source reference (may include a line/label).
        reference: Text,
        source_sha256: Digest,
    },
    SourceLabel {
        reference: Text,
        source_sha256: Digest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub width: u8,
    pub length: u8,
}
impl Footprint {
    pub fn validate(self) -> Result<(), MapError> {
        if self.width == 0 || self.length == 0 || self.width > 64 || self.length > 64 {
            return Err(MapError::Invalid("footprint"));
        }
        Ok(())
    }
    pub fn rotated(self, rotation: u8) -> Result<Self, MapError> {
        self.validate()?;
        if rotation > 3 {
            return Err(MapError::Invalid("rotation"));
        }
        Ok(if rotation & 1 != 0 {
            Self {
                width: self.length,
                length: self.width,
            }
        } else {
            self
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayAnchor {
    pub x: f64,
    pub z: f64,
    pub plane: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkTarget {
    pub tile: GameTile,
    pub nav_sha256: Digest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoiRecord {
    pub key: PoiKey,
    pub name: Text,
    pub kind: PoiKind,
    pub effective_plane: u8,
    /// Already rotated by the producer; physical placement remains key.x/z.
    pub footprint: Footprint,
    pub display: DisplayAnchor,
    pub evidence: Rows<CapabilityEvidence, 8>,
    /// Separate collision/approach-proven stand; never the display anchor.
    pub walk_target: Option<WalkTarget>,
}
impl PoiRecord {
    pub fn validate(&self) -> Result<(), MapError> {
        if self.key.source.game_plane()? != Some(self.effective_plane)
            || self.display.plane != self.effective_plane
        {
            return Err(MapError::Invalid("POI plane conversion"));
        }
        if matches!(self.key.entity, EntityKind::Npc | EntityKind::Label)
            && matches!(self.key.source, SourceSpace::ClientVisual { .. })
        {
            return Err(MapError::Invalid("non-visual entity source space"));
        }
        if self.key.rotation > 3 || self.key.shape > 22 {
            return Err(MapError::Invalid("placement shape/rotation"));
        }
        if !matches!(self.key.entity, EntityKind::Loc | EntityKind::MapFunction)
            && (self.key.rotation != 0 || self.key.shape != 0)
        {
            return Err(MapError::Invalid("non-LOC shape/rotation"));
        }
        self.footprint.validate()?;
        if !self.display.x.is_finite()
            || !self.display.z.is_finite()
            || self.display.x < f64::from(i32::MIN)
            || self.display.x > f64::from(i32::MAX)
            || self.display.z < f64::from(i32::MIN)
            || self.display.z > f64::from(i32::MAX)
        {
            return Err(MapError::Invalid("display anchor"));
        }
        if self.evidence.as_slice().is_empty() {
            return Err(MapError::Invalid("missing POI evidence"));
        }
        if let Some(target) = self.walk_target {
            target.tile.validate()?;
            if target.tile.plane != self.effective_plane {
                return Err(MapError::Invalid("stand plane"));
            }
        }
        Ok(())
    }
}

/// Borrowed metadata only. Call while streaming placements and retain candidates,
/// not every LOC. Five slots preserve cache indices even when earlier ops are absent.
pub struct Definition<'a> {
    pub entity: EntityKind,
    pub name: &'a str,
    pub operations: [Option<&'a str>; 5],
    pub active: bool,
    pub mapfunction: Option<u16>,
}

/// Allocation-free conservative classifier shared by local and supplement producers.
/// The observed 289 bank symbol is 5; unfamiliar revision/category pairs stay generic.
pub fn classify_definition(
    revision: u16,
    definition: &Definition<'_>,
    mut emit: impl FnMut(PoiKind, CapabilityEvidence),
) {
    if matches!(definition.entity, EntityKind::Loc | EntityKind::Npc) {
        for (index, op) in definition.operations.iter().enumerate() {
            let Some(op) = op else {
                continue;
            };
            let op = op.trim();
            let service = if op.eq_ignore_ascii_case("Bank") {
                Some((PoiKind::Bank, Capability::Bank))
            } else if op.eq_ignore_ascii_case("Trade") {
                Some((PoiKind::Shop, Capability::Trade))
            } else if op.eq_ignore_ascii_case("Pray-at") {
                Some((PoiKind::Altar, Capability::Pray))
            } else {
                None
            };
            if let Some((kind, capability)) = service {
                emit(
                    kind,
                    CapabilityEvidence::ClientOperation {
                        capability,
                        slot: OperationSlot(index as u8 + 1),
                    },
                );
            }
        }
        if definition.entity == EntityKind::Loc
            && definition.active
            && definition.name.trim().eq_ignore_ascii_case("Bank booth")
            && definition
                .operations
                .iter()
                .flatten()
                .any(|op| op.trim().eq_ignore_ascii_case("Use-quickly"))
        {
            emit(PoiKind::Bank, CapabilityEvidence::ActiveQuickBooth);
        }
    }
    if let Some(symbol) = definition.mapfunction {
        let kind = if revision == 289 && symbol == 5 {
            PoiKind::Bank
        } else {
            PoiKind::MapSymbol { symbol }
        };
        emit(kind, CapabilityEvidence::MapFunction { symbol });
    }
}
