//! Host-owned evidence for the retained v2 hunt File cells: walk spot and bank.
//!
//! Each example is `*Begin` + one awaited `*Run` (FENCE Q2). Its paint says
//! only what the host cannot see: how the run settled and the tiles the
//! script chose. Everything a gate trusts beyond that is host data:
//!
//! - the Start baseline's host tile and bank state;
//! - the game requests host-play dispatched for the slot, in order. The
//!   [`ScriptActLedger`] is filled where `script_runtime` handles each
//!   `InteractReq`; it is host data, not a new isolate wire;
//! - the host tile and bank state when the receipt is painted.
//!
//! A script that never starts the run sends none of the family's requests
//! and never moves, so no cell can pass on its paint alone.
//!
//! The ledger cannot distinguish requests emitted by a run's Rust machine
//! from requests queued directly by the script. Both reach host-play in one
//! interact batch, and tagging them would need a new isolate wire.

use std::collections::VecDeque;

use api::snapshot::GameSnapshot;
use serde::{Deserialize, Serialize};

use super::Observation;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineOfSightTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

pub const WALK_SPOT_V2_STOP: &str = "walk spot qualification complete";
pub const WALK_SPOT_RECEIPT_PREFIX: &str = "walk-spot-receipt:";
/// Chebyshev 13–20 separates this longer walk-in from nearby actions.
pub const WALK_SPOT_MIN_CHEB: i32 = 13;
pub const WALK_SPOT_MAX_CHEB: i32 = 20;
pub const BANK_V2_STOP: &str = "bank qualification complete";
pub const BANK_V2_RECEIPT_PREFIX: &str = "bank-receipt:";
/// The bank family's approach radius (`hunt_bank.rs` `APPROACH_RADIUS`).
pub const BANK_V2_RADIUS: i32 = 3;
/// How far the booth approach's stand may be from the site bank tile: the
/// approach radius plus the one tile between a booth and its stand.
pub const BANK_V2_BOOTH_REACH: i32 = BANK_V2_RADIUS + 1;
/// Falador west bank, the bank File card's site bank.
pub const BANK_V2_DEST: LineOfSightTile = LineOfSightTile {
    x: 2946,
    z: 3369,
    level: 0,
};
/// The fixture bounds that the bank Start baseline must avoid.
const HUNT_LAIR_FIXTURE: HuntBox = HuntBox {
    min_x: 40,
    max_x: 60,
    min_z: 40,
    max_z: 60,
    level: 0,
};
/// Ledger rows a slot keeps for the watch to read.
pub const HUNT_ACT_ROWS: usize = 32;
/// Rows one run's cycle keeps; more is a run this gate does not model.
const HUNT_CYCLE_ACTS: usize = 256;
const KBD_TILE: (i32, i32) = (3017, 3849);
const KBD_LOCS: [i32; 4] = [1765, 1766, 1816, 1817];

/// One v2 hunt File cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HuntCell {
    WalkSpot,
    Bank,
}

impl HuntCell {
    pub fn stop_reason(self) -> &'static str {
        match self {
            Self::WalkSpot => WALK_SPOT_V2_STOP,
            Self::Bank => BANK_V2_STOP,
        }
    }

    pub fn receipt_prefix(self) -> &'static str {
        match self {
            Self::WalkSpot => WALK_SPOT_RECEIPT_PREFIX,
            Self::Bank => BANK_V2_RECEIPT_PREFIX,
        }
    }

    /// Walk spot settles `null`; the bank run must settle `true`.
    fn settled_value(self) -> Option<bool> {
        match self {
            Self::WalkSpot => None,
            Self::Bank => Some(true),
        }
    }
}

/// A rectangle of tiles on one level.
#[derive(Clone, Copy)]
struct HuntBox {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
    level: i32,
}

impl HuntBox {
    fn contains(self, tile: LineOfSightTile) -> bool {
        tile.level == self.level
            && (self.min_x..=self.max_x).contains(&tile.x)
            && (self.min_z..=self.max_z).contains(&tile.z)
    }
}

/// One game request host-play dispatched for a slot's script. Npc and loc
/// rows name the scene entity the host resolved, not the request's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ScriptAct {
    /// `walk` (`exact`) or `walk-near`, queued on the slot's walk arm.
    Walk {
        dest: LineOfSightTile,
        radius: i32,
        exact: bool,
        allow_teleports: bool,
        allow_wilderness: bool,
        allow_bank_fetch: bool,
        request_id: u64,
    },
    /// `walk-to`: the scene walk packet was sent.
    WalkTo { dest: LineOfSightTile },
    /// An npc op was sent to this scene npc.
    Npc {
        name: String,
        action: String,
        index: i32,
    },
    /// A loc op was sent to this scene loc.
    Loc {
        tile: LineOfSightTile,
        id: i32,
        action: String,
    },
    /// A held item was used on the loc at `tile`.
    UseOnLoc { item: String, tile: LineOfSightTile },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptActRow {
    pub seq: u64,
    pub act: ScriptAct,
}

/// A slot's bounded record of the game requests its script sent, as host-play
/// dispatched them. `seq` counts every row ever recorded.
#[derive(Debug, Clone, Default)]
pub struct ScriptActLedger {
    seq: u64,
    rows: VecDeque<ScriptActRow>,
}

impl ScriptActLedger {
    pub fn record(&mut self, act: ScriptAct) {
        self.seq += 1;
        if self.rows.len() == HUNT_ACT_ROWS {
            self.rows.pop_front();
        }
        self.rows.push_back(ScriptActRow { seq: self.seq, act });
    }

    pub fn published(&self) -> ScriptActsPublished {
        ScriptActsPublished {
            seq: self.seq,
            rows: self.rows.iter().cloned().collect(),
        }
    }
}

/// The ledger as one frame read it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScriptActsPublished {
    pub seq: u64,
    pub rows: Vec<ScriptActRow>,
}

/// How the awaited run settled, as the script saw it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HuntOutcome {
    pub kind: String,
    #[serde(default)]
    pub value: Option<bool>,
}

/// The paint row an example writes after its run settles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HuntReceipt {
    pub outcome: HuntOutcome,
    /// The tile the script started the run from.
    pub from: LineOfSightTile,
    /// The tile after the run settled.
    pub here: LineOfSightTile,
    /// The run's destination.
    pub dest: LineOfSightTile,
    /// The one-shot site's key.
    #[serde(default)]
    pub key: Option<String>,
}

pub fn parse_hunt_receipt_line(cell: HuntCell, line: &str) -> Option<HuntReceipt> {
    serde_json::from_str(line.strip_prefix(cell.receipt_prefix())?).ok()
}

/// Compact hunt witness. Empty unless the active Core case is a hunt cell.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct HuntObservation {
    /// The scene was decoded this frame.
    pub available: bool,
    pub acts: ScriptActsPublished,
    pub receipt: Option<HuntReceipt>,
}

impl Observation {
    /// Attach the slot's act ledger and the cell's paint receipt. Never
    /// copies the collision grid or world.
    pub fn attach_hunt(
        &mut self,
        cell: HuntCell,
        snapshot: &GameSnapshot,
        paint: Option<&script::paint::ScriptPaint>,
        acts: Option<ScriptActsPublished>,
    ) {
        self.hunt = HuntObservation {
            available: snapshot.scene().available,
            acts: acts.unwrap_or_default(),
            receipt: paint.and_then(|paint| {
                paint
                    .lines
                    .iter()
                    .find_map(|line| parse_hunt_receipt_line(cell, line))
            }),
        };
    }
}

fn host_tile(observation: &Observation) -> Option<LineOfSightTile> {
    observation
        .tile
        .map(|(x, z, level)| LineOfSightTile { x, z, level })
}

fn cheb(a: LineOfSightTile, b: LineOfSightTile) -> i32 {
    if a.level != b.level {
        return i32::MAX;
    }
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn kbd_tile(tile: LineOfSightTile) -> bool {
    (tile.x, tile.z) == KBD_TILE
}

/// Start predicate. The host must be in a usable scene and not already at
/// the bank.
pub fn hunt_baseline_ready(cell: HuntCell, baseline: &Observation) -> bool {
    let Some(from) = host_tile(baseline) else {
        return false;
    };
    let placed = match cell {
        HuntCell::WalkSpot => true,
        HuntCell::Bank => {
            !HUNT_LAIR_FIXTURE.contains(from)
                && cheb(from, BANK_V2_DEST) > BANK_V2_RADIUS
                && !baseline.bank_open
        }
    };
    baseline.ingame && baseline.scene_state == 2 && baseline.hunt.available && placed
}

/// Post-Start witness for one hunt cell: its host requests, joined receipt,
/// and named stop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HuntDeliveryCycle {
    /// Host tile of the Start baseline.
    pub from: Option<LineOfSightTile>,
    /// Ledger seq at the Start baseline; only later rows are this run's.
    pub start_seq: Option<u64>,
    /// This run's host-dispatched requests, in order.
    pub acts: Vec<ScriptActRow>,
    /// Rows left the ledger before this watch read them: fail closed.
    pub acts_lost: bool,
    /// Bank: the host saw the bank open and loaded after the approach walk.
    pub bank_opened: bool,
    /// Bank: the host bank was still open when the receipt joined.
    pub bank_open_at_receipt: bool,
    pub receipt: Option<HuntReceipt>,
    /// Host tile when the receipt joined.
    pub here: Option<LineOfSightTile>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl HuntDeliveryCycle {
    pub fn observe(&mut self, cell: HuntCell, baseline: &Observation, now: &Observation) {
        let start = *self.start_seq.get_or_insert(baseline.hunt.acts.seq);
        if self.from.is_none() {
            self.from = host_tile(baseline);
        }
        if self.receipt.is_some() {
            return;
        }
        self.take_acts(start, &now.hunt.acts);
        let here = host_tile(now);
        if cell == HuntCell::Bank
            && now.bank_open
            && now.bank_loaded
            && self.walked_near(BANK_V2_DEST, BANK_V2_RADIUS).is_some()
        {
            self.bank_opened = true;
        }
        let Some(receipt) = now.hunt.receipt.as_ref() else {
            return;
        };
        if now.hunt.available && here == Some(receipt.here) {
            self.receipt = Some(receipt.clone());
            self.here = here;
            self.bank_open_at_receipt = now.bank_open;
        }
    }

    fn take_acts(&mut self, start: u64, published: &ScriptActsPublished) {
        let last = self.acts.last().map_or(start, |row| row.seq);
        if published.seq < last {
            // The ledger restarted under this run.
            self.acts_lost = true;
            return;
        }
        if published.seq == last {
            return;
        }
        let mut next = last + 1;
        for row in published.rows.iter().filter(|row| row.seq > last) {
            if row.seq != next || self.acts.len() == HUNT_CYCLE_ACTS {
                self.acts_lost = true;
                return;
            }
            self.acts.push(row.clone());
            next += 1;
        }
        if next != published.seq + 1 {
            self.acts_lost = true;
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

    /// First act matching `f`, by seq.
    fn first(&self, f: impl Fn(&ScriptAct) -> bool) -> Option<u64> {
        self.acts.iter().find(|row| f(&row.act)).map(|row| row.seq)
    }

    fn any(&self, f: impl Fn(&ScriptAct) -> bool) -> bool {
        self.acts.iter().any(|row| f(&row.act))
    }

    fn all_walks(&self, f: impl Fn(&ScriptAct) -> bool) -> bool {
        self.acts
            .iter()
            .filter(|row| matches!(row.act, ScriptAct::Walk { .. }))
            .all(|row| f(&row.act))
    }

    /// The first host walk-near to `dest` at `radius` with every route
    /// permission off.
    fn walked_near(&self, dest: LineOfSightTile, radius: i32) -> Option<u64> {
        self.first(|act| walk_near_closed(act, dest, radius))
    }

    pub fn qualified(&self, cell: HuntCell) -> bool {
        let (Some(from), Some(receipt), Some(here), Some(_)) = (
            self.from,
            self.receipt.as_ref(),
            self.here,
            self.stopped.as_ref(),
        ) else {
            return false;
        };
        let dest = receipt.dest;
        let settled =
            receipt.outcome.kind == "done" && receipt.outcome.value == cell.settled_value();
        let site_ok = cell != HuntCell::Bank
            || receipt
                .key
                .as_deref()
                .is_some_and(|key| !key.is_empty() && key != "kbd-lair");
        let kbd = kbd_tile(from)
            || kbd_tile(here)
            || kbd_tile(dest)
            || self.any(|act| matches!(act, ScriptAct::Loc { id, .. } if KBD_LOCS.contains(id)));
        if self.acts_lost
            || !settled
            || !site_ok
            || kbd
            || receipt.from != from
            || receipt.here != here
            || from.level != dest.level
            || from == dest
        {
            return false;
        }
        let scene_op = |act: &ScriptAct| {
            matches!(
                act,
                ScriptAct::Npc { .. } | ScriptAct::Loc { .. } | ScriptAct::UseOnLoc { .. }
            )
        };
        let walk_to = |act: &ScriptAct| matches!(act, ScriptAct::WalkTo { .. });
        let exact_to = |act: &ScriptAct, open: bool| walk_exact(act, dest, open);
        match cell {
            // The long world walk to a requested spot; no walk-near.
            HuntCell::WalkSpot => {
                in_ring(from, dest, WALK_SPOT_MIN_CHEB, WALK_SPOT_MAX_CHEB)
                    && here == dest
                    && self.first(|act| exact_to(act, true)).is_some()
                    && self.all_walks(|act| exact_to(act, true))
                    && !self.any(|act| walk_to(act) || scene_op(act))
            }
            // The root walk approaches the bank. Any script-act booth approach
            // stays near it; native seat walks have their own HostEffect lane.
            // The host must observe the bank open, then shut, within the radius.
            HuntCell::Bank => {
                let Some(walk) = self.walked_near(BANK_V2_DEST, BANK_V2_RADIUS) else {
                    return false;
                };
                dest == BANK_V2_DEST
                    && cheb(here, dest) <= BANK_V2_RADIUS
                    && self.bank_opened
                    && !self.bank_open_at_receipt
                    && self.acts.iter().all(|row| match &row.act {
                        ScriptAct::Walk { .. } => {
                            walk_near_closed(&row.act, BANK_V2_DEST, BANK_V2_RADIUS)
                                || (row.seq > walk && booth_approach(&row.act))
                        }
                        _ => true,
                    })
                    && !self.any(|act| walk_to(act) || scene_op(act))
            }
        }
    }
}

fn in_ring(from: LineOfSightTile, dest: LineOfSightTile, min: i32, max: i32) -> bool {
    (min..=max).contains(&cheb(from, dest))
}

/// A `walk` to exactly `dest` with a request id and teleports off. WalkSpot
/// may use wilderness and bank fetch; other routes keep them off.
fn walk_exact(act: &ScriptAct, dest: LineOfSightTile, open: bool) -> bool {
    matches!(act, ScriptAct::Walk {
        dest: d,
        exact: true,
        allow_teleports: false,
        allow_wilderness,
        allow_bank_fetch,
        request_id,
        ..
    } if *d == dest && *request_id != 0 && (open || (!allow_wilderness && !allow_bank_fetch)))
}

fn walk_near_closed(act: &ScriptAct, dest: LineOfSightTile, radius: i32) -> bool {
    matches!(act, ScriptAct::Walk {
        dest: d,
        radius: r,
        exact: false,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        request_id,
    } if *d == dest && *r == radius && *request_id != 0)
}

/// The script-act format for a booth approach: radius zero, wilderness and
/// bank fetch enabled, and no legacy walk-wait id. Native bank-seat walks
/// instead carry their authority in `HostEffect::Walk`.
fn booth_approach(act: &ScriptAct) -> bool {
    matches!(act, ScriptAct::Walk {
        dest,
        radius: 0,
        exact: false,
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        request_id: 0,
    } if cheb(*dest, BANK_V2_DEST) <= BANK_V2_BOOTH_REACH)
}
