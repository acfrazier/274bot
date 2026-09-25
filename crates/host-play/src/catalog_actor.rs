use super::*;
pub const ACTOR_OBSERVATION_V2_STOP: &str = "actor observation qualification complete";
pub const ACTOR_RECEIPT_PREFIX: &str = "actor-receipt:";
/// File and Core share this rule: min `(distance, index)` among `size >= 1`.
/// After the script posts a receipt, look that index up so headed join verifies
/// the observed row instead of independently picking another NPC.
pub(super) fn choose_actor_observation_npc<'a>(
    npcs: &'a [NpcView],
    receipt: Option<&ActorObservationScriptReceipt>,
) -> Option<&'a NpcView> {
    if let Some(index) = receipt.map(|row| row.npc.index) {
        return npcs
            .iter()
            .find(|row| row.size >= 1 && row.index as i32 == index);
    }
    npcs.iter()
        .filter(|row| row.size >= 1)
        .min_by_key(|row| (row.distance, row.index))
}

pub(super) fn packed_self_target(snapshot: &GameSnapshot) -> (i32, i32) {
    match snapshot
        .local_player()
        .and_then(|player| player.player.actor.target)
    {
        None => (0, -1),
        Some(target) => match target.kind {
            ActorKind::Npc => (1, target.index as i32),
            ActorKind::Player => (2, target.index as i32),
        },
    }
}

pub(super) fn parse_actor_receipt_from_paint(
    paint: &script::shim::ScriptPaint,
) -> Option<ActorObservationScriptReceipt> {
    paint.lines.iter().find_map(|line| {
        line.strip_prefix(ACTOR_RECEIPT_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationPoint {
    pub x: i32,
    pub z: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationNpcFact {
    pub index: i32,
    pub name: Option<String>,
    pub size: i32,
    pub tile: ActorObservationPoint,
    pub network: ActorObservationPoint,
    pub level: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationPacked {
    pub size: i32,
    pub nx: i32,
    pub nz: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationSelfTarget {
    pub kind: i32,
    pub index: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationLos {
    pub v2: bool,
    pub v1: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActorObservationScriptReceipt {
    pub identity: LineOfSightIdentity,
    pub here: LineOfSightTile,
    pub npc: ActorObservationNpcFact,
    pub packed: ActorObservationPacked,
    pub rendered: ActorObservationPoint,
    pub self_target: ActorObservationSelfTarget,
    pub los: ActorObservationLos,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ActorObservationNpc {
    pub index: i32,
    pub name: Option<String>,
    pub size: i32,
    pub tile_x: i32,
    pub tile_z: i32,
    pub nx: i32,
    pub nz: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActorObservation {
    pub available: bool,
    pub identity: LineOfSightIdentity,
    pub here: Option<LineOfSightTile>,
    pub npc: Option<ActorObservationNpc>,
    pub host_los: Option<bool>,
    pub self_target_kind: i32,
    pub self_target_index: i32,
    pub receipt: Option<ActorObservationScriptReceipt>,
}

impl Default for ActorObservation {
    fn default() -> Self {
        Self {
            available: false,
            identity: LineOfSightIdentity::default(),
            here: None,
            npc: None,
            host_los: None,
            self_target_kind: 0,
            self_target_index: -1,
            receipt: None,
        }
    }
}

pub fn actor_observation_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.actor.available
        && baseline.actor.here.is_some()
}

fn actor_receipt_joined(now: &ActorObservation) -> bool {
    let (Some(npc), Some(here), Some(host_los), Some(receipt)) = (
        now.npc.as_ref(),
        now.here,
        now.host_los,
        now.receipt.as_ref(),
    ) else {
        return false;
    };
    now.available
        && npc.size >= 1
        && receipt.identity == now.identity
        && receipt.here == here
        && receipt.npc.index == npc.index
        && receipt.npc.name == npc.name
        && receipt.npc.size == npc.size
        && receipt.npc.level == npc.level
        && receipt.npc.tile.x == npc.tile_x
        && receipt.npc.tile.z == npc.tile_z
        && receipt.npc.network.x == npc.nx
        && receipt.npc.network.z == npc.nz
        && receipt.packed.size == npc.size
        && receipt.packed.nx == npc.nx
        && receipt.packed.nz == npc.nz
        && receipt.rendered.x == npc.tile_x
        && receipt.rendered.z == npc.tile_z
        && receipt.self_target.kind == now.self_target_kind
        && receipt.self_target.index == now.self_target_index
        && receipt.los.v2 == host_los
        && receipt.los.v1 == host_los
}

/// Post-Start witness: one packed size>=1 NPC, joined script receipt, named stop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ActorObservationDeliveryCycle {
    pub identity: Option<LineOfSightIdentity>,
    pub npc: Option<ActorObservationNpc>,
    pub host_los: Option<bool>,
    pub receipt: Option<ActorObservationScriptReceipt>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl ActorObservationDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if self.receipt.is_some() {
            return;
        }
        if now.actor.available {
            if let Some(npc) = now.actor.npc.clone() {
                if npc.size >= 1 {
                    self.identity = Some(now.actor.identity);
                    self.npc = Some(npc);
                    self.host_los = now.actor.host_los;
                }
            }
        }
        if actor_receipt_joined(&now.actor) {
            self.identity = Some(now.actor.identity);
            self.npc = now.actor.npc.clone();
            self.host_los = now.actor.host_los;
            self.receipt = now.actor.receipt.clone();
        }
    }

    pub fn observe_script_lifecycle(
        &mut self,
        receipt: script::ScriptLifecycleReceipt,
        expected: &str,
    ) {
        if self.receipt.is_some()
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && receipt.reason == expected
        {
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        if self.stopped.is_none() {
            return false;
        }
        let (Some(identity), Some(npc), Some(host_los), Some(receipt)) = (
            self.identity,
            self.npc.as_ref(),
            self.host_los,
            self.receipt.as_ref(),
        ) else {
            return false;
        };
        identity == receipt.identity
            && npc.size >= 1
            && npc.index == receipt.npc.index
            && npc.name == receipt.npc.name
            && npc.size == receipt.npc.size
            && npc.level == receipt.npc.level
            && npc.tile_x == receipt.npc.tile.x
            && npc.tile_z == receipt.npc.tile.z
            && npc.nx == receipt.npc.network.x
            && npc.nz == receipt.npc.network.z
            && npc.size == receipt.packed.size
            && npc.nx == receipt.packed.nx
            && npc.nz == receipt.packed.nz
            && npc.tile_x == receipt.rendered.x
            && npc.tile_z == receipt.rendered.z
            && receipt.los.v2 == host_los
            && receipt.los.v1 == host_los
    }
}