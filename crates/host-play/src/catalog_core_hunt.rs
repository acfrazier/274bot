//! Host-owned evidence for the v2 hunt File cells: hold, retreat, walk spot,
//! enter, leave, key, cell and bank.
//!
//! Each example is `*Begin` + one awaited `*Run` (FENCE Q2). Its paint says
//! only what the host cannot see: how the run settled and the tiles the
//! script chose. Everything a gate trusts beyond that is host data:
//!
//! - the Start baseline's host tile and inventory;
//! - the game requests host-play dispatched for the slot, in order. The
//!   [`ScriptActLedger`] is filled where `script_runtime` handles each
//!   `InteractReq`; it is host data, not a new isolate wire;
//! - the host tile and inventory when the receipt is painted.
//!
//! A script that never starts the run sends none of the family's requests
//! and never moves, so no cell can pass on its paint alone.
//!
//! Known limit: the ledger cannot tell a request the run's Rust machine
//! emitted from one the script queued itself. Both reach host-play merged in
//! one interact batch, and tagging them would need a new isolate wire. A
//! script that queues the family's walk itself, with any nonzero request id,
//! and then walks there can satisfy the walk-only cells (hold, walk spot,
//! enter, leave, bank) without the run. The gates defend against a missing
//! or no-op run, not against a script that forges the family's requests.

use std::collections::VecDeque;

use api::snapshot::GameSnapshot;
use serde::{Deserialize, Serialize};

use super::{LineOfSightTile, Observation};

pub const HOLD_SPOT_V2_STOP: &str = "hold spot qualification complete";
pub const HOLD_SPOT_RECEIPT_PREFIX: &str = "hold-spot-receipt:";
pub const HOLD_SPOT_MIN_CHEB: i32 = 2;
pub const HOLD_SPOT_MAX_CHEB: i32 = 6;
pub const RETREAT_SPOT_V2_STOP: &str = "retreat spot qualification complete";
pub const RETREAT_SPOT_RECEIPT_PREFIX: &str = "retreat-spot-receipt:";
pub const RETREAT_SPOT_MIN_CHEB: i32 = 2;
pub const RETREAT_SPOT_MAX_CHEB: i32 = 6;
pub const WALK_SPOT_V2_STOP: &str = "walk spot qualification complete";
pub const WALK_SPOT_RECEIPT_PREFIX: &str = "walk-spot-receipt:";
/// Chebyshev 12 is Hold's walk-back. This cell requires a longer walk-in.
pub const WALK_SPOT_MIN_CHEB: i32 = 13;
pub const WALK_SPOT_MAX_CHEB: i32 = 20;
pub const ENTER_LAIR_V2_STOP: &str = "enter lair qualification complete";
pub const ENTER_LAIR_RECEIPT_PREFIX: &str = "enter-lair-receipt:";
/// Greater than the approach skip of 1, and not Hold's 2-tile walk-back.
/// Inclusive 8–16. Do not copy Walk's `> 12` gate.
pub const ENTER_LAIR_MIN_CHEB: i32 = 8;
pub const ENTER_LAIR_MAX_CHEB: i32 = 16;
pub const LEAVE_LAIR_V2_STOP: &str = "leave lair qualification complete";
pub const LEAVE_LAIR_RECEIPT_PREFIX: &str = "leave-lair-receipt:";
/// Inclusive 8–16. Not Hold's 2-tile walk and not Walk's `> 12` gate.
pub const LEAVE_LAIR_MIN_CHEB: i32 = 8;
pub const LEAVE_LAIR_MAX_CHEB: i32 = 16;
/// The leave family's walk-out radius.
pub const LEAVE_LAIR_RADIUS: i32 = 3;
/// The leave File card's curated lair box is the Start tile ± 2.
pub const LEAVE_LAIR_BOX_PAD: i32 = 2;
pub const ACQUIRE_KEY_V2_STOP: &str = "acquire key qualification complete";
pub const ACQUIRE_KEY_RECEIPT_PREFIX: &str = "acquire-key-receipt:";
/// The key family's corridor walk and pickup radius (`hunt_key.rs`).
pub const ACQUIRE_KEY_RADIUS: i32 = 1;
/// Frozen `supply.ts:129` `JAIL_DOOR`: the prison corridor north of Velrak's
/// cell. The key family walks near it and the cell family walks near it
/// before `killJailer` (`fetchFromVelrak`, `supply.ts:942-947`).
pub const JAIL_DOOR: LineOfSightTile = LineOfSightTile {
    x: 2931,
    z: 9690,
    level: 0,
};
pub const ACQUIRE_KEY_DEST: LineOfSightTile = JAIL_DOOR;
pub const CELL_V2_STOP: &str = "cell qualification complete";
pub const CELL_V2_RECEIPT_PREFIX: &str = "cell-receipt:";
/// Frozen `fetchFromVelrak` walks near `JAIL_DOOR` at radius 1
/// (`supply.ts:944`).
pub const CELL_V2_DOOR: LineOfSightTile = JAIL_DOOR;
pub const CELL_V2_RADIUS: i32 = 1;
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
/// Velrak's jail cell (`hunt_key.rs` / `hunt_cell.rs` `CELL`).
pub const JAIL_CELL: HuntBox = HuntBox {
    min_x: 2928,
    max_x: 2934,
    min_z: 9683,
    max_z: 9689,
    level: 0,
};
/// The File cards' projected lair fixture. No real tile is inside it, so a
/// run never takes the leave leg.
pub const HUNT_LAIR_FIXTURE: HuntBox = HuntBox {
    min_x: 40,
    max_x: 60,
    min_z: 40,
    max_z: 60,
    level: 0,
};
pub const JAIL_KEY_ID: i32 = 1591;
pub const DUSTY_KEY_ID: i32 = 1590;
/// `dungeonjail`, Velrak's cell door.
pub const JAIL_DOOR_LOC: i32 = 2631;
pub const JAILER: &str = "Jailer";
pub const VELRAK: &str = "Velrak the explorer";
/// Ledger rows a slot keeps for the watch to read.
pub const HUNT_ACT_ROWS: usize = 32;
/// Rows one run's cycle keeps; more is a run this gate does not model.
const HUNT_CYCLE_ACTS: usize = 256;
const KBD_TILE: (i32, i32) = (3017, 3849);
const KBD_LOCS: [i32; 4] = [1765, 1766, 1816, 1817];
const EDGEVILLE: (i32, i32) = (3094, 3493);
const VARROCK: (i32, i32) = (3213, 3424);

/// One v2 hunt File cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HuntCell {
    Hold,
    Retreat,
    WalkSpot,
    Enter,
    Leave,
    Key,
    Cell,
    Bank,
}

impl HuntCell {
    pub fn stop_reason(self) -> &'static str {
        match self {
            Self::Hold => HOLD_SPOT_V2_STOP,
            Self::Retreat => RETREAT_SPOT_V2_STOP,
            Self::WalkSpot => WALK_SPOT_V2_STOP,
            Self::Enter => ENTER_LAIR_V2_STOP,
            Self::Leave => LEAVE_LAIR_V2_STOP,
            Self::Key => ACQUIRE_KEY_V2_STOP,
            Self::Cell => CELL_V2_STOP,
            Self::Bank => BANK_V2_STOP,
        }
    }

    pub fn receipt_prefix(self) -> &'static str {
        match self {
            Self::Hold => HOLD_SPOT_RECEIPT_PREFIX,
            Self::Retreat => RETREAT_SPOT_RECEIPT_PREFIX,
            Self::WalkSpot => WALK_SPOT_RECEIPT_PREFIX,
            Self::Enter => ENTER_LAIR_RECEIPT_PREFIX,
            Self::Leave => LEAVE_LAIR_RECEIPT_PREFIX,
            Self::Key => ACQUIRE_KEY_RECEIPT_PREFIX,
            Self::Cell => CELL_V2_RECEIPT_PREFIX,
            Self::Bank => BANK_V2_RECEIPT_PREFIX,
        }
    }

    /// The settled value a passing run reports. Hold, retreat and walk spot
    /// settle `null` and prove arrival by the host tile; the boolean runs
    /// must settle `true` (a stepper that gives up settles `false`).
    fn settled_value(self) -> Option<bool> {
        match self {
            Self::Hold | Self::Retreat | Self::WalkSpot => None,
            _ => Some(true),
        }
    }

    /// The run takes a one-shot site whose key the receipt names.
    fn names_site(self) -> bool {
        !matches!(self, Self::Hold | Self::Retreat | Self::WalkSpot)
    }

    /// The inventory item whose arrival proves the run.
    fn proof_item(self) -> Option<i32> {
        match self {
            Self::Key => Some(JAIL_KEY_ID),
            Self::Cell => Some(DUSTY_KEY_ID),
            _ => None,
        }
    }

    /// The walk whose destination the host tile must reach while the run
    /// is still going (key and cell end somewhere else).
    fn latch_walk(self) -> Option<(LineOfSightTile, i32)> {
        match self {
            Self::Key => Some((ACQUIRE_KEY_DEST, ACQUIRE_KEY_RADIUS)),
            Self::Cell => Some((CELL_V2_DOOR, CELL_V2_RADIUS)),
            _ => None,
        }
    }
}

/// A rectangle of tiles on one level.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HuntBox {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub level: i32,
}

impl HuntBox {
    pub fn contains(self, tile: LineOfSightTile) -> bool {
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
    /// Enter: the lair box the site names.
    #[serde(default, rename = "box")]
    pub area: Option<HuntBox>,
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
        paint: Option<&script::shim::ScriptPaint>,
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

fn town_tile(tile: LineOfSightTile) -> bool {
    (tile.x, tile.z) == EDGEVILLE || (tile.x, tile.z) == VARROCK
}

/// The leave card's curated lair: the Start tile ± 2.
pub fn leave_lair_box(from: LineOfSightTile) -> HuntBox {
    HuntBox {
        min_x: from.x - LEAVE_LAIR_BOX_PAD,
        max_x: from.x + LEAVE_LAIR_BOX_PAD,
        min_z: from.z - LEAVE_LAIR_BOX_PAD,
        max_z: from.z + LEAVE_LAIR_BOX_PAD,
        level: from.level,
    }
}

/// Start predicate. The run has not begun, so nothing it proves may already
/// hold: key and cell start outside the cell, away from the door and without
/// the key they fetch; bank starts outside its approach radius.
pub fn hunt_baseline_ready(cell: HuntCell, baseline: &Observation) -> bool {
    let Some(from) = host_tile(baseline) else {
        return false;
    };
    let placed = match cell {
        HuntCell::Hold | HuntCell::Retreat | HuntCell::WalkSpot => true,
        HuntCell::Enter => !kbd_tile(from),
        HuntCell::Leave => !kbd_tile(from) && !town_tile(from),
        HuntCell::Key => {
            !JAIL_CELL.contains(from)
                && !HUNT_LAIR_FIXTURE.contains(from)
                && cheb(from, ACQUIRE_KEY_DEST) > ACQUIRE_KEY_RADIUS
                && baseline.item_id(JAIL_KEY_ID) == 0
        }
        HuntCell::Cell => {
            !JAIL_CELL.contains(from)
                && !HUNT_LAIR_FIXTURE.contains(from)
                && cheb(from, CELL_V2_DOOR) > CELL_V2_RADIUS
                && baseline.item_id(DUSTY_KEY_ID) == 0
        }
        HuntCell::Bank => {
            !HUNT_LAIR_FIXTURE.contains(from)
                && cheb(from, BANK_V2_DEST) > BANK_V2_RADIUS
                && !baseline.bank_open
        }
    };
    baseline.ingame && baseline.scene_state == 2 && baseline.hunt.available && placed
}

/// Post-Start witness for one hunt cell: the run's own host requests, the
/// host tile it reached, the joined receipt, and the named stop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HuntDeliveryCycle {
    /// Host tile of the Start baseline.
    pub from: Option<LineOfSightTile>,
    /// Ledger seq at the Start baseline; only later rows are this run's.
    pub start_seq: Option<u64>,
    /// Host count of the cell's proof item at the Start baseline.
    pub start_proof_items: i32,
    /// This run's host-dispatched requests, in order.
    pub acts: Vec<ScriptActRow>,
    /// Rows left the ledger before this watch read them: fail closed.
    pub acts_lost: bool,
    /// Key and cell: the host tile reached the walk's destination after the
    /// walk was sent.
    pub arrived: bool,
    /// Bank: the host saw the bank open and loaded after the approach walk.
    pub bank_opened: bool,
    /// Bank: the host bank was still open when the receipt joined.
    pub bank_open_at_receipt: bool,
    pub receipt: Option<HuntReceipt>,
    /// Host tile when the receipt joined.
    pub here: Option<LineOfSightTile>,
    /// Host count of the proof item when the receipt joined.
    pub proof_items: i32,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl HuntDeliveryCycle {
    pub fn observe(&mut self, cell: HuntCell, baseline: &Observation, now: &Observation) {
        let start = *self.start_seq.get_or_insert(baseline.hunt.acts.seq);
        if self.from.is_none() {
            self.from = host_tile(baseline);
            self.start_proof_items = cell.proof_item().map_or(0, |id| baseline.item_id(id));
        }
        if self.receipt.is_some() {
            return;
        }
        self.take_acts(start, &now.hunt.acts);
        let here = host_tile(now);
        if let (Some((dest, radius)), Some(here)) = (cell.latch_walk(), here) {
            if self.walked_near(dest, radius).is_some() && cheb(here, dest) <= radius {
                self.arrived = true;
            }
        }
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
            self.proof_items = cell.proof_item().map_or(0, |id| now.item_id(id));
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
        let (Some(from), Some(receipt), Some(here), Some(_)) =
            (self.from, self.receipt.as_ref(), self.here, self.stopped.as_ref())
        else {
            return false;
        };
        let dest = receipt.dest;
        let settled =
            receipt.outcome.kind == "done" && receipt.outcome.value == cell.settled_value();
        let site_ok = !cell.names_site()
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
            // The world walk (teleports off) to dest and nothing else.
            HuntCell::Hold => {
                in_ring(from, dest, HOLD_SPOT_MIN_CHEB, HOLD_SPOT_MAX_CHEB)
                    && here == dest
                    && self.first(|act| exact_to(act, true)).is_some()
                    && self.all_walks(|act| exact_to(act, true))
                    && !self.any(|act| walk_to(act) || scene_op(act))
            }
            // The scene walk-to dest; no world walk.
            HuntCell::Retreat => {
                in_ring(from, dest, RETREAT_SPOT_MIN_CHEB, RETREAT_SPOT_MAX_CHEB)
                    && here == dest
                    && self.first(|act| matches!(act, ScriptAct::WalkTo { dest: d } if *d == dest))
                        .is_some()
                    && !self.any(|act| {
                        matches!(act, ScriptAct::Walk { .. })
                            || matches!(act, ScriptAct::WalkTo { dest: d } if *d != dest)
                            || scene_op(act)
                    })
            }
            // The world walk to a dest past Hold's walk-back; no walk-near.
            HuntCell::WalkSpot => {
                in_ring(from, dest, WALK_SPOT_MIN_CHEB, WALK_SPOT_MAX_CHEB)
                    && here == dest
                    && self.first(|act| exact_to(act, true)).is_some()
                    && self.all_walks(|act| exact_to(act, true))
                    && !self.any(|act| walk_to(act) || scene_op(act))
            }
            // The gateless approach walk into the lair box; inside at the end.
            HuntCell::Enter => {
                let Some(area) = receipt.area else {
                    return false;
                };
                in_ring(from, dest, ENTER_LAIR_MIN_CHEB, ENTER_LAIR_MAX_CHEB)
                    && !area.contains(from)
                    && area.contains(dest)
                    && area.contains(here)
                    && self.first(|act| exact_to(act, false)).is_some()
                    && self.all_walks(|act| exact_to(act, false))
                    && !self.any(|act| walk_to(act) || scene_op(act))
            }
            // The gateless walk-near walkOut from the Start box. The family
            // settles `true` as soon as here leaves the box, so the end tile
            // is outside it and on the way to walkOut, not necessarily there.
            HuntCell::Leave => {
                let area = leave_lair_box(from);
                in_ring(from, dest, LEAVE_LAIR_MIN_CHEB, LEAVE_LAIR_MAX_CHEB)
                    && !area.contains(dest)
                    && !town_tile(from)
                    && !town_tile(dest)
                    && !area.contains(here)
                    && cheb(here, dest) < cheb(from, dest)
                    && self.walked_near(dest, LEAVE_LAIR_RADIUS).is_some()
                    && self.all_walks(|act| walk_near_closed(act, dest, LEAVE_LAIR_RADIUS))
                    && !self.any(|act| walk_to(act) || scene_op(act))
            }
            // The corridor walk, then the Jailer attack; the jail key held.
            HuntCell::Key => {
                let Some(walk) = self.walked_near(ACQUIRE_KEY_DEST, ACQUIRE_KEY_RADIUS) else {
                    return false;
                };
                let attack = self.first(|act| {
                    matches!(act, ScriptAct::Npc { name, action, .. }
                        if name.eq_ignore_ascii_case(JAILER) && action.eq_ignore_ascii_case("Attack"))
                });
                dest == ACQUIRE_KEY_DEST
                    && self.arrived
                    && attack.is_some_and(|attack| attack > walk)
                    && self.start_proof_items == 0
                    && self.proof_items > 0
                    && !JAIL_CELL.contains(here)
                    && !self.any(|act| {
                        walk_to(act)
                            || matches!(act, ScriptAct::Walk { exact: true, .. })
                            || matches!(act, ScriptAct::Loc { .. } | ScriptAct::UseOnLoc { .. })
                    })
            }
            // Frozen `fetchFromVelrak` (`supply.ts:942-965`): walk near the
            // jail door, unlock it with the jail key, talk to Velrak, open
            // the door from inside; the dusty key held outside the cell.
            HuntCell::Cell => {
                let Some(walk) = self.walked_near(CELL_V2_DOOR, CELL_V2_RADIUS) else {
                    return false;
                };
                let unlock = self.first(|act| {
                    matches!(act, ScriptAct::UseOnLoc { item, tile }
                        if item.eq_ignore_ascii_case("Jail key") && cheb(*tile, CELL_V2_DOOR) <= 1)
                });
                let talk = self.first(|act| {
                    matches!(act, ScriptAct::Npc { name, .. } if name.eq_ignore_ascii_case(VELRAK))
                });
                let open = self.first(|act| {
                    matches!(act, ScriptAct::Loc { id, action, .. }
                        if *id == JAIL_DOOR_LOC && action.eq_ignore_ascii_case("Open"))
                });
                let ordered = matches!(
                    (unlock, talk, open),
                    (Some(unlock), Some(talk), Some(open)) if walk < unlock && unlock < talk && talk < open
                );
                dest == CELL_V2_DOOR
                    && self.arrived
                    && ordered
                    && self.start_proof_items == 0
                    && self.proof_items > 0
                    && !JAIL_CELL.contains(here)
            }
            // The approach walk-near the bank, then (bank_open.rs) at most the
            // booth approach; the host saw the bank open and shut again;
            // within the radius at the end.
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

/// A `walk` to exactly `dest` with a request id and teleports off. `open`:
/// the family's world walk may use the wilderness and bank fetch (Hold,
/// WalkToSpot); otherwise every route permission is off.
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

/// The bank family's booth approach once it stands near the bank: the
/// `bank-open` child's `walk-near` radius 0 to the booth's stand, which it
/// emits itself with wilderness and bank fetch on and no walk-wait id
/// (`bank_open.rs` `walk_near_req`).
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
