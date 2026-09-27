use super::*;
pub const LOS_V2_STOP: &str = "line of sight qualification complete";
pub const LOS_RECEIPT_PREFIX: &str = "los-receipt:";
pub const LOS_WALK_SCENERY: i32 = 0x100;
pub const LOS_V_N: i32 = 0x400;
pub const LOS_V_E: i32 = 0x1000;
pub const LOS_V_S: i32 = 0x4000;
pub const LOS_V_W: i32 = 0x10000;
pub const LOS_VIS_SCENERY: i32 = 0x20000;
pub const LOS_PAIR_RADIUS: i32 = 8;
const LOS_DIRS: [(i32, i32, i32); 4] = [
    (1, 0, LOS_V_W),
    (-1, 0, LOS_V_E),
    (0, 1, LOS_V_S),
    (0, -1, LOS_V_N),
];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightIdentity {
    pub base_x: i32,
    pub base_z: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightPair {
    pub from: LineOfSightTile,
    pub to: LineOfSightTile,
    pub src: i32,
    pub dst: i32,
    pub mask: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightPairResult {
    pub from: LineOfSightTile,
    pub to: LineOfSightTile,
    pub src: i32,
    pub dst: i32,
    pub mask: i32,
    pub v2: bool,
    pub v1: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightHere {
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub flag: i32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightScriptReceipt {
    pub identity: LineOfSightIdentity,
    pub here: LineOfSightHere,
    pub open: LineOfSightPairResult,
    pub blocked: LineOfSightPairResult,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LineOfSightObservation {
    pub available: bool,
    pub identity: LineOfSightIdentity,
    pub here: Option<LineOfSightTile>,
    pub here_flag: Option<i32>,
    pub host_open: Option<LineOfSightPair>,
    pub host_blocked: Option<LineOfSightPair>,
    pub fixture_failure: Option<String>,
    pub receipt: Option<LineOfSightScriptReceipt>,
}

impl LineOfSightPairResult {
    pub fn pair(self) -> LineOfSightPair {
        LineOfSightPair {
            from: self.from,
            to: self.to,
            src: self.src,
            dst: self.dst,
            mask: self.mask,
        }
    }
}

pub(super) fn collision_flag_at(scene: &SceneView, tile: LineOfSightTile) -> Option<i32> {
    if !scene.available || tile.level != scene.level {
        return None;
    }
    let lx = tile.x - scene.base_x;
    let lz = tile.z - scene.base_z;
    if lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height {
        return None;
    }
    scene
        .collision_flags
        .get((lx * scene.height + lz) as usize)
        .copied()
}

pub fn los_entering_mask(dx: i32, dz: i32) -> Option<i32> {
    match (dx, dz) {
        (1, 0) => Some(LOS_V_W),
        (-1, 0) => Some(LOS_V_E),
        (0, 1) => Some(LOS_V_S),
        (0, -1) => Some(LOS_V_N),
        _ => None,
    }
}

pub fn line_of_sight_pair_is_open(pair: &LineOfSightPair) -> bool {
    let dx = pair.to.x - pair.from.x;
    let dz = pair.to.z - pair.from.z;
    pair.from.level == pair.to.level
        && los_entering_mask(dx, dz) == Some(pair.mask)
        && pair.src & LOS_WALK_SCENERY == 0
        && pair.dst & pair.mask == 0
}

pub fn line_of_sight_pair_is_blocked(pair: &LineOfSightPair) -> bool {
    let dx = pair.to.x - pair.from.x;
    let dz = pair.to.z - pair.from.z;
    // Source WALK_SCENERY must be clear: the LOS helper returns false on
    // source scenery before ray tracing, so a scenery-sourced negative is
    // not attributable to the entering V-wall.
    pair.from.level == pair.to.level
        && los_entering_mask(dx, dz) == Some(pair.mask)
        && pair.src & LOS_WALK_SCENERY == 0
        && pair.dst & pair.mask != 0
}

pub fn line_of_sight_dest_vis_alone(pair: &LineOfSightPair) -> bool {
    pair.dst & LOS_VIS_SCENERY != 0 && pair.dst & pair.mask == 0
}

/// Deterministic Chebyshev 0..=8 cardinal scan. Expected answers come from
/// one-step raw V-mask facts, not from calling the LOS helper.
pub fn select_line_of_sight_pairs(
    here: LineOfSightTile,
    flag_at: impl Fn(i32, i32) -> Option<i32>,
) -> Result<(LineOfSightPair, LineOfSightPair), String> {
    let mut open = None;
    let mut blocked = None;
    for r in 0..=LOS_PAIR_RADIUS {
        for dx in -r..=r {
            for dz in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let from = LineOfSightTile {
                    x: here.x + dx,
                    z: here.z + dz,
                    level: here.level,
                };
                if (from.x - here.x).abs().max((from.z - here.z).abs()) > LOS_PAIR_RADIUS {
                    continue;
                }
                let Some(src) = flag_at(from.x, from.z) else {
                    continue;
                };
                for (sx, sz, mask) in LOS_DIRS {
                    let to = LineOfSightTile {
                        x: from.x + sx,
                        z: from.z + sz,
                        level: here.level,
                    };
                    if (to.x - here.x).abs().max((to.z - here.z).abs()) > LOS_PAIR_RADIUS {
                        continue;
                    }
                    let Some(dst) = flag_at(to.x, to.z) else {
                        continue;
                    };
                    let pair = LineOfSightPair {
                        from,
                        to,
                        src,
                        dst,
                        mask,
                    };
                    if blocked.is_none() && line_of_sight_pair_is_blocked(&pair) {
                        blocked = Some(pair);
                    } else if open.is_none() && line_of_sight_pair_is_open(&pair) {
                        open = Some(pair);
                    }
                    if let (Some(open), Some(blocked)) = (open, blocked) {
                        return Ok((open, blocked));
                    }
                }
            }
        }
    }
    Err("los fixture failure: no cardinal open+blocked V-wall pair within 8".into())
}

pub fn parse_los_receipt_from_paint(
    paint: &script::shim::ScriptPaint,
) -> Option<LineOfSightScriptReceipt> {
    paint.lines.iter().find_map(|line| {
        line.strip_prefix(LOS_RECEIPT_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
    })
}

pub fn line_of_sight_baseline_ready(baseline: &Observation) -> bool {
    baseline.ingame
        && baseline.scene_state == 2
        && baseline.los.available
        && baseline.los.here_flag.is_some()
}

fn los_receipt_joined(now: &LineOfSightObservation) -> bool {
    let (Some(host_open), Some(host_blocked), Some(receipt), Some(here), Some(here_flag)) = (
        now.host_open,
        now.host_blocked,
        now.receipt.as_ref(),
        now.here,
        now.here_flag,
    ) else {
        return false;
    };
    now.available
        && now.fixture_failure.is_none()
        && receipt.identity == now.identity
        && receipt.here.x == here.x
        && receipt.here.z == here.z
        && receipt.here.level == here.level
        && receipt.here.flag == here_flag
        && line_of_sight_pair_is_open(&host_open)
        && line_of_sight_pair_is_blocked(&host_blocked)
        && !line_of_sight_dest_vis_alone(&host_blocked)
        && !line_of_sight_dest_vis_alone(&receipt.blocked.pair())
        && receipt.open.pair() == host_open
        && receipt.blocked.pair() == host_blocked
        && receipt.open.v2
        && !receipt.blocked.v2
        && receipt.open.v1
        && !receipt.blocked.v1
}

/// Post-Start witness: host-selected one-step pairs, user-script receipt
/// joined to the same SceneView identity/flags, then the named helper stop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LineOfSightDeliveryCycle {
    pub host_open: Option<LineOfSightPair>,
    pub host_blocked: Option<LineOfSightPair>,
    pub identity: Option<LineOfSightIdentity>,
    pub receipt: Option<LineOfSightScriptReceipt>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
    pub fixture_failure: Option<String>,
}

impl LineOfSightDeliveryCycle {
    pub fn observe(&mut self, now: &Observation) {
        if let Some(msg) = now.los.fixture_failure.clone() {
            self.fixture_failure = Some(msg);
        }
        // Once a same-observation host/receipt join is latched, freeze host
        // pairs and identity with that historical witness. Later host drift
        // must not overwrite evidence that still backs a valid receipt.
        if self.receipt.is_some() {
            return;
        }
        if now.los.available {
            if let (Some(open), Some(blocked)) = (now.los.host_open, now.los.host_blocked) {
                if line_of_sight_pair_is_open(&open) && line_of_sight_pair_is_blocked(&blocked) {
                    self.host_open = Some(open);
                    self.host_blocked = Some(blocked);
                    self.identity = Some(now.los.identity);
                }
            }
        }
        if los_receipt_joined(&now.los) {
            // Latch host + receipt from the same joined observation.
            self.host_open = now.los.host_open;
            self.host_blocked = now.los.host_blocked;
            self.identity = Some(now.los.identity);
            self.receipt = now.los.receipt;
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
        if self.fixture_failure.is_some() || self.stopped.is_none() {
            return false;
        }
        let (Some(host_open), Some(host_blocked), Some(identity), Some(receipt)) = (
            self.host_open,
            self.host_blocked,
            self.identity,
            self.receipt.as_ref(),
        ) else {
            return false;
        };
        // Re-assert stored host still matches the latched receipt (fail-closed).
        identity == receipt.identity
            && host_open == receipt.open.pair()
            && host_blocked == receipt.blocked.pair()
            && line_of_sight_pair_is_open(&host_open)
            && line_of_sight_pair_is_blocked(&host_blocked)
            && !line_of_sight_dest_vis_alone(&host_blocked)
            && receipt.open.v2
            && !receipt.blocked.v2
            && receipt.open.v1
            && !receipt.blocked.v1
    }
}
