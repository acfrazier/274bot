use api::snapshot::GameSnapshot;
use nav::map::poi::{classify_definition, CapabilityEvidence, Definition, EntityKind, PoiKind};
use nav::tile::Tile;

use super::{ActionError, MapContext};

pub const MAX_OBSERVED_SERVICES: usize = 128;

/// An actual observed NPC service, not a persistent global spawn. Account
/// eligibility remains unknown; evidence preserves the real operation slot.
#[derive(Debug, Clone)]
pub struct ObservedService {
    pub npc_index: usize,
    pub kind: PoiKind,
    pub tile: Tile,
    pub evidence: CapabilityEvidence,
    pub context: MapContext,
}

/// Extract the first bounded service records in snapshot order. Borrow the
/// frontend's existing snapshot under its lock only for this call: no snapshot,
/// NPC strings or client tables are cloned or retained. Call on open-map refresh,
/// not every draw, and discard the result on close or a binding change.
///
/// `source` is the binding captured alongside the snapshot; `current` is the
/// current focused slot/world/nav binding. Old observations never become facts
/// about a replacement slot with the same username.
/// Only observed operation slots are classified, which is revision-neutral;
/// this path never interprets revision-specific mapfunction symbols or IDs.
pub fn observed_services(
    snapshot: &GameSnapshot,
    source: MapContext,
    current: MapContext,
) -> Result<Vec<ObservedService>, ActionError> {
    if source != current || source.focus.is_none() {
        return Err(ActionError::Stale);
    }
    if !snapshot.ingame() || snapshot.tile().is_none() {
        return Err(ActionError::NoOrigin);
    }
    let mut records = Vec::new();
    for npc in snapshot.npcs() {
        if records.len() == MAX_OBSERVED_SERVICES {
            break;
        }
        if !(0..4).contains(&npc.tile.level) || npc.r#type.is_none() {
            continue;
        }
        let definition = Definition {
            entity: EntityKind::Npc,
            name: npc.name.as_deref().unwrap_or("Unnamed NPC"),
            operations: std::array::from_fn(|i| npc.actions.get(i).and_then(|op| op.as_deref())),
            active: true,
            mapfunction: None,
        };
        classify_definition(0, &definition, |kind, evidence| {
            if records.len() < MAX_OBSERVED_SERVICES {
                if records.is_empty() {
                    records.reserve_exact(
                        snapshot
                            .npcs()
                            .len()
                            .saturating_mul(definition.operations.len())
                            .min(MAX_OBSERVED_SERVICES),
                    );
                }
                records.push(ObservedService {
                    npc_index: npc.index,
                    kind,
                    tile: Tile {
                        x: npc.tile.x,
                        z: npc.tile.z,
                        level: npc.tile.level,
                    },
                    evidence,
                    context: source,
                });
            }
        });
    }
    Ok(records)
}
