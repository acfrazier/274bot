//! Source-backed terminal contracts for normal File actions.
//!
//! Each contract names the exact engine and content heads it was read from. A
//! request whose family, target or selected heads have no entry is refused at
//! admission: a File root never closes from an unrelated check, a quiet tick
//! or a local send.

use api::snapshot::{GameSnapshot, WorldTile};

/// Revision-377 heads of the shared 377 slot inputs (engine slot mirror and
/// content canon). Citations below are `file:line` at these heads.
pub(super) const R377_CANON_ENGINE: &str = "b8bc0c1c801427107743cc9f5132000b90557ba3";
pub(super) const R377_CANON_CONTENT: &str = "cb6cc5ea1c9bbe13b9680bb2ddd4ccaeab2b5f40";
/// Revision-377 heads of the SC-01 swamp-caves content line (LD2 §5.1).
pub(super) const R377_SC01_ENGINE: &str = "15301127e60ff6ef3d7d2547b084de15fb565338";
pub(super) const R377_SC01_CONTENT: &str = "0ed5f39138f3064e5a76af5c9f42db555441fc82";

/// Firemaking stat index (`PlayerStat.ts`: FIREMAKING = 11).
const FIREMAKING: i32 = 11;

/// The normal action one File root sent, resolved to exact type identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileAction {
    /// OPHELDU: one carried item used on another carried item.
    UseOnHeld { source: i32, target: i32 },
    /// OPHELDn on a carried item, `op` 1..=5.
    HeldOp { item: i32, op: u8 },
    /// Worn-items Remove (`inv_button1` on the worn component).
    Unequip { item: i32 },
    /// OPLOCn on the exact loc type at `tile`, `op` 1..=5.
    LocOp { loc: i32, tile: WorldTile, op: u8 },
}

/// Inventory domains a terminal closure observes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Domain {
    Carried,
    Worn,
}

/// The terminal record a fresh admitted check must match after the send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Terminal {
    /// Inventory domains whose record must involve one of `items`.
    pub(super) inventories: &'static [Domain],
    pub(super) items: Vec<i32>,
    /// An exact type-0 message that also qualifies, when the family emits one.
    pub(super) message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Closure {
    /// A fresh admitted check must match a post-send terminal record (LD2
    /// §5.1 route 1). The handler finishes synchronously, so that record is
    /// the family's terminal outcome.
    Terminal(Terminal),
    /// The handler emits this exact target-specific diagnostic and clears the
    /// interaction. The root needs that fresh record and then the two-query
    /// ordered read barrier for the completed player pass (LD2 §5.1).
    NoTrigger { message: String },
    /// A silent level guard: below `level` the script returns with no effect.
    /// Closure is the admitted silence window, then the two-query barrier,
    /// with branch cause Unattributed. A later same-item paired control at or
    /// above `level` must close by its terminal check before the fence.
    SilentGuard { skill: i32, level: i32, item: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Contract {
    pub(super) id: &'static str,
    pub(super) closure: Closure,
    pub(super) citations: &'static [&'static str],
}

/// Selected engine/content heads for a File run.
#[derive(Clone, Copy)]
pub(super) struct Heads<'a> {
    pub(super) revision: u16,
    pub(super) engine: &'a str,
    pub(super) content: &'a str,
}

/// Pinned type ids at a content head (`pack/obj.pack`, `pack/loc.pack`).
struct Ids {
    tinderbox: i32,
    oil_lamp_unlit: i32,
    oil_lamp_lit: i32,
    oil_lantern_unlit: i32,
    oil_lantern_lit: i32,
    helmet_lit: i32,
    helmet_unlit: i32,
    goblin_cave_entrance: i32,
}

// canon content `pack/obj.pack:591,4523,4525,4538,4540,5014,5015`, `pack/loc.pack:5948`.
const CANON_IDS: Ids = Ids {
    tinderbox: 590,
    oil_lamp_unlit: 4522,
    oil_lamp_lit: 4524,
    oil_lantern_unlit: 4537,
    oil_lantern_lit: 4539,
    helmet_lit: 5013,
    helmet_unlit: 5014,
    goblin_cave_entrance: 5947,
};

/// Debug name for the NoTrigger diagnostic of a pinned type id.
fn debugname(ids: &Ids, id: i32) -> Option<&'static str> {
    [
        (ids.tinderbox, "tinderbox"),
        (ids.oil_lamp_unlit, "oil_lamp_unlit"),
        (ids.oil_lamp_lit, "oil_lamp_lit"),
        (ids.oil_lantern_unlit, "oil_lantern_unlit"),
        (ids.oil_lantern_lit, "oil_lantern_lit"),
        (ids.helmet_lit, "cave_goblin_mining_helmet_lit"),
        (ids.helmet_unlit, "cave_goblin_mining_helmet_unlit"),
    ]
    .into_iter()
    .find_map(|(known, name)| (known == id).then_some(name))
}

const LIGHT_CITATIONS: &[&str] = &[
    "engine OpHeldUHandler.ts:93-118: [opheldu,tinderbox] resolves and executes synchronously",
    "engine Player.ts:2329-2345: executeScript returns without a queued continuation for a finished script",
    "content scripts/skill_firemaking/scripts/firemaking.rs2:13-25: tinderbox on light_source_unlit calls ignite_light_source",
    "content scripts/skill_firemaking/scripts/light_source.rs2:1-11: inv_del unlit, inv_add product, light message",
    "content scripts/_unpack/377/all.obj:54479-54491,54630-54642: oil lamp/lantern unlit are light_source_unlit with product",
];

const NO_TRIGGER_HELD_CITATIONS: &[&str] = &[
    "engine OpHeldHandler.ts:45-73: a valid iop with no script emits the NoTrigger diagnostic synchronously when NODE_DEBUG",
    "content scripts/_unpack/377/all.obj:54497-54509,54648-54660: lit oil lamp/lantern have iop2 Extinguish and no opheld2 script",
];

const EQUIP_CITATIONS: &[&str] = &[
    "engine OpHeldHandler.ts:65-68: opheld2 executes [opheld2,_] synchronously",
    "content scripts/player/scripts/equip.rs2:1,10-72: ~equip moves the carried item to worn; only elemental_shield delays",
];

const UNEQUIP_CITATIONS: &[&str] = &[
    "content scripts/player/scripts/equip.rs2:3-8,74-: [inv_button1,wornitems:worn] runs ~unequip synchronously",
];

const DARK_HOLE_CITATIONS: &[&str] = &[
    "content scripts/_unpack/377/all.loc:42277-42281: goblin_cave_entrance has op1 Climb-down and no oploc1 script",
    "engine Player.ts:1205-1230: defaultOp emits the NoTrigger diagnostic, Nothing interesting happens, clears waypoints",
    "engine World.ts:344-412: inputs, player interactions and client output run in one synchronous cycle",
    "engine server/tcp/TcpServer.ts:22-35: socket data buffers before World.onClientData",
];

const SILENT_GUARD_CITATIONS: &[&str] = &[
    "content 0ed5f39138f3 scripts/skill_firemaking/scripts/light_source.rs2:4-26: oil_lantern_unlit needs level 26 and returns silently below it",
    "engine 15301127e60f OpHeldUHandler.ts:84-118: synchronous script execution",
    "engine 15301127e60f Player.ts:2297-2302,2328-2344: protected/delayed access can refuse silently before execution",
];

/// Select the contract for one resolved action, or refuse it.
pub(super) fn select(
    heads: Heads<'_>,
    action: FileAction,
    snapshot: &GameSnapshot,
) -> Result<Contract, String> {
    let canon = heads.revision == 377
        && heads.engine == R377_CANON_ENGINE
        && heads.content == R377_CANON_CONTENT;
    let sc01 = heads.revision == 377
        && heads.engine == R377_SC01_ENGINE
        && heads.content == R377_SC01_CONTENT;
    if !canon && !sc01 {
        return Err(
            "no source-backed action contract for the selected engine/content heads".into(),
        );
    }
    let ids = &CANON_IDS;
    let unlit = [ids.oil_lamp_unlit, ids.oil_lantern_unlit, ids.helmet_unlit];
    match action {
        FileAction::UseOnHeld { source, target } => {
            let other = if source == ids.tinderbox {
                target
            } else if target == ids.tinderbox {
                source
            } else {
                return Err("use-on contract covers only the tinderbox on a light source".into());
            };
            if !unlit.contains(&other) {
                return Err("use-on contract covers only the pinned unlit light sources".into());
            }
            if sc01 {
                // Guard levels are content data at light_source.rs2:4-15 of the SC-01 line.
                let level = match other {
                    id if id == ids.oil_lamp_unlit => 12,
                    id if id == ids.oil_lantern_unlit => 26,
                    _ => 65,
                };
                let stat = snapshot
                    .stats()
                    .iter()
                    .find(|stat| stat.index == FIREMAKING)
                    .ok_or("firemaking level is unavailable for the guard contract")?;
                if stat.effective < level {
                    return Ok(Contract {
                        id: "r377-sc01-light-source-below-level",
                        closure: Closure::SilentGuard {
                            skill: FIREMAKING,
                            level,
                            item: other,
                        },
                        citations: SILENT_GUARD_CITATIONS,
                    });
                }
            }
            let product = match other {
                id if id == ids.oil_lamp_unlit => ids.oil_lamp_lit,
                id if id == ids.oil_lantern_unlit => ids.oil_lantern_lit,
                _ => ids.helmet_lit,
            };
            Ok(Contract {
                id: "r377-light-source",
                closure: Closure::Terminal(Terminal {
                    inventories: &[Domain::Carried],
                    items: vec![other, product],
                    message: None,
                }),
                citations: LIGHT_CITATIONS,
            })
        }
        FileAction::HeldOp { item, op: 2 }
            if canon && [ids.oil_lamp_lit, ids.oil_lantern_lit].contains(&item) =>
        {
            Ok(Contract {
                id: "r377-canon-held-no-trigger",
                closure: Closure::Terminal(Terminal {
                    inventories: &[],
                    items: Vec::new(),
                    message: Some(no_trigger_text(ids, "opheld2", item)?),
                }),
                citations: NO_TRIGGER_HELD_CITATIONS,
            })
        }
        FileAction::HeldOp { item, op: 2 }
            if [ids.helmet_lit, ids.helmet_unlit].contains(&item) =>
        {
            Ok(Contract {
                id: "r377-equip",
                closure: Closure::Terminal(Terminal {
                    inventories: &[Domain::Carried, Domain::Worn],
                    items: vec![item],
                    message: None,
                }),
                citations: EQUIP_CITATIONS,
            })
        }
        FileAction::Unequip { item } if [ids.helmet_lit, ids.helmet_unlit].contains(&item) => {
            Ok(Contract {
                id: "r377-unequip",
                closure: Closure::Terminal(Terminal {
                    inventories: &[Domain::Carried, Domain::Worn],
                    items: vec![item],
                    message: None,
                }),
                citations: UNEQUIP_CITATIONS,
            })
        }
        FileAction::LocOp { loc, op: 1, .. } if loc == ids.goblin_cave_entrance => Ok(Contract {
            id: "r377-dark-hole-no-trigger",
            closure: Closure::NoTrigger {
                message: "No trigger for [oploc1,goblin_cave_entrance]".into(),
            },
            citations: DARK_HOLE_CITATIONS,
        }),
        _ => Err("no source-backed terminal contract for this action family or target".into()),
    }
}

fn no_trigger_text(ids: &Ids, trigger: &str, item: i32) -> Result<String, String> {
    debugname(ids, item)
        .map(|name| format!("No trigger for [{trigger},{name}]"))
        .ok_or_else(|| "no pinned debug name for the NoTrigger target".into())
}

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod tests;
