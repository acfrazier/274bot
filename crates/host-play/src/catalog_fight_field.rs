use super::*;
pub const FIGHT_FIELD_V2_STOP: &str = "fight field qualification complete";
pub const FIGHT_FIELD_RECEIPT_PREFIX: &str = "fight-field-receipt:";
/// File and Core share this rule: min `(distance, index)` among `size >= 1`.
/// After the script posts a receipt, look that index up so headed join verifies
/// the observed row instead of independently picking another NPC.
pub(super) fn choose_fight_field_npc<'a>(
    npcs: &'a [NpcView],
    receipt: Option<&FightFieldScriptReceipt>,
) -> Option<&'a NpcView> {
    if let Some(index) = receipt.map(|row| row.index) {
        return npcs
            .iter()
            .find(|row| row.size >= 1 && row.index as i32 == index);
    }
    npcs.iter()
        .filter(|row| row.size >= 1)
        .min_by_key(|row| (row.distance, row.index))
}

pub(super) fn parse_fight_field_receipt_from_paint(
    paint: &script::shim::ScriptPaint,
) -> Option<FightFieldScriptReceipt> {
    paint.lines.iter().find_map(|line| {
        line.strip_prefix(FIGHT_FIELD_RECEIPT_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

fn fight_field_effect_is_attack(receipt: &FightFieldScriptReceipt) -> bool {
    fn is_attack(value: &str) -> bool {
        value.eq_ignore_ascii_case("npc") || value.eq_ignore_ascii_case("attack")
    }
    receipt.kind.as_deref().is_some_and(is_attack)
        || receipt.effect.as_deref().is_some_and(is_attack)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FightFieldScriptReceipt {
    pub index: i32,
    pub size: i32,
    pub tile: ActorObservationPoint,
    pub network_origin: ActorObservationPoint,
    pub los_network: bool,
    pub los_tile: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FightFieldNpc {
    pub index: i32,
    pub size: i32,
    pub tile_x: i32,
    pub tile_z: i32,
    pub nx: i32,
    pub nz: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FightFieldObservation {
    pub available: bool,
    pub identity: LineOfSightIdentity,
    pub here: Option<LineOfSightTile>,
    pub npc: Option<FightFieldNpc>,
    pub host_los_network: Option<bool>,
    pub host_los_tile: Option<bool>,
    pub receipt: Option<FightFieldScriptReceipt>,
}

pub fn fight_field_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.fight.available
        && baseline.fight.here.is_some()
}

fn fight_field_receipt_joined(now: &FightFieldObservation) -> bool {
    let (Some(npc), Some(here), Some(host_los_network), Some(host_los_tile), Some(receipt)) = (
        now.npc.as_ref(),
        now.here,
        now.host_los_network,
        now.host_los_tile,
        now.receipt.as_ref(),
    ) else {
        return false;
    };
    now.available
        && npc.size >= 1
        && here.level == npc.level
        && receipt.index == npc.index
        && receipt.size == npc.size
        && receipt.tile.x == npc.tile_x
        && receipt.tile.z == npc.tile_z
        && receipt.network_origin.x == npc.nx
        && receipt.network_origin.z == npc.nz
        && receipt.los_network == host_los_network
        && receipt.los_tile == host_los_tile
        && !fight_field_effect_is_attack(receipt)
}

/// Post-Start witness: one packed size>=1 NPC, joined script receipt, named stop, no Attack.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FightFieldDeliveryCycle {
    pub identity: Option<LineOfSightIdentity>,
    pub here: Option<LineOfSightTile>,
    pub npc: Option<FightFieldNpc>,
    pub host_los_network: Option<bool>,
    pub host_los_tile: Option<bool>,
    pub receipt: Option<FightFieldScriptReceipt>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl FightFieldDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if self.receipt.is_some() {
            return;
        }
        if now.fight.available {
            if let Some(npc) = now.fight.npc.clone() {
                if npc.size >= 1 {
                    self.identity = Some(now.fight.identity);
                    self.here = now.fight.here;
                    self.npc = Some(npc);
                    self.host_los_network = now.fight.host_los_network;
                    self.host_los_tile = now.fight.host_los_tile;
                }
            }
        }
        if fight_field_receipt_joined(&now.fight) {
            self.identity = Some(now.fight.identity);
            self.here = now.fight.here;
            self.npc = now.fight.npc.clone();
            self.host_los_network = now.fight.host_los_network;
            self.host_los_tile = now.fight.host_los_tile;
            self.receipt = now.fight.receipt.clone();
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
        let (
            Some(identity),
            Some(here),
            Some(npc),
            Some(host_los_network),
            Some(host_los_tile),
            Some(receipt),
        ) = (
            self.identity,
            self.here,
            self.npc.as_ref(),
            self.host_los_network,
            self.host_los_tile,
            self.receipt.as_ref(),
        )
        else {
            return false;
        };
        identity.level == here.level
            && npc.size >= 1
            && npc.index == receipt.index
            && npc.size == receipt.size
            && npc.tile_x == receipt.tile.x
            && npc.tile_z == receipt.tile.z
            && npc.nx == receipt.network_origin.x
            && npc.nz == receipt.network_origin.z
            && receipt.los_network == host_los_network
            && receipt.los_tile == host_los_tile
            && !fight_field_effect_is_attack(receipt)
    }
}