//! Source-backed terminal contracts for normal File actions.
//!
//! Each contract names the exact engine and content heads it was read from. A
//! request whose family, target or selected heads have no entry is refused at
//! admission: a File root never closes from an unrelated check, a quiet tick
//! or a local send.

use api::snapshot::{GameSnapshot, WorldTile};

/// Revision-377 heads of the shared 377 slot inputs (engine slot mirror and
/// content canon). Citations below are `file:line` at these heads.
pub(super) const R377_CANON_ENGINE: &str = "28b7c03232b959a141ab88114af6b9a29ab4e2c4";
pub(super) const R377_CANON_CONTENT: &str = "314228314e4c3c0f13c46d034e87c51f0dd9ab5a";
/// Revision-377 heads of the SC-01 swamp-caves content line (LD2 §5.1).
pub(super) const R377_SC01_ENGINE: &str = "15301127e60ff6ef3d7d2547b084de15fb565338";
pub(super) const R377_SC01_CONTENT: &str = "0ed5f39138f3064e5a76af5c9f42db555441fc82";
/// Revision-289 heads of the lab slot (engine and content mirrors).
pub(super) const R289_ENGINE: &str = "574f10f493cf39ffcc70a9b2f9a7b12b11d914e8";
pub(super) const R289_CONTENT: &str = "a60809082bc9f8fbdaa00c7b1a1d3fd6134c18fe";

// R289 content `pack/obj.pack:527` (bones), `pack/loc.pack:2214` (bankbooth).
const R289_BONES: i32 = 526;
const R289_BANK_BOOTH: i32 = 2213;

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
    /// INV_BUTTONn on one row of the open bank's withdraw inventory.
    BankWithdraw { item: i32, op: u8 },
    /// IF_CLOSE of the open main modal.
    CloseModal,
}

/// Inventory domains a terminal closure observes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Domain {
    Carried,
    Worn,
    Bank,
}

/// A bank modal transition a terminal record can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BankEdge {
    /// Closed at send, open with a loaded withdraw inventory now.
    Opened,
    /// Open at send, closed now.
    Closed,
}

/// The terminal record a fresh admitted check must match after the send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Terminal {
    /// Inventory domains whose record must involve one of `items`.
    pub(super) inventories: &'static [Domain],
    pub(super) items: Vec<i32>,
    /// An exact type-0 message that also qualifies, when the family emits one.
    pub(super) message: Option<String>,
    /// A bank modal transition that also qualifies.
    pub(super) bank: Option<BankEdge>,
    /// The family queues a delay-0 follow-up: close only on a PLAYER_INFO
    /// after the one that first showed the terminal record, which proves the
    /// next cycle's queue ran (engine `World.ts:699-701`, `Player.ts:891-913`).
    pub(super) settle: bool,
    /// Npc types the family's own branch can spawn as a live descendant. A
    /// fresh one in view at closure means the root still has a pending
    /// effect, so the run fails instead of closing it.
    pub(super) descendants: &'static [i32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Closure {
    /// A fresh admitted check must match a post-send terminal record (LD2
    /// §5.1 route 1). The handler finishes synchronously, so that record is
    /// the family's terminal outcome.
    Terminal(Terminal),
    /// A silent level guard: below `level` the script returns with no effect.
    /// Closure is the admitted silence window, then the two-query barrier,
    /// with branch cause Unattributed. A later same-item paired control at or
    /// above `level` must close by its terminal check before the fence.
    SilentGuard { skill: i32, level: i32, item: i32 },
    /// A content script returns before any effect or message. Closure is the
    /// admitted silence window, then the two-query barrier. Unlike
    /// [`Closure::SilentGuard`], there is no skill precondition to pair.
    SilentReturn,
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

const LIGHT_CITATIONS: &[&str] = &[
    "engine OpHeldUHandler.ts:118: [opheldu,tinderbox] executes the resolved script synchronously",
    "engine Player.ts:2329-2345: executeScript returns without a queued continuation for a finished script",
    "content scripts/skill_firemaking/scripts/firemaking.rs2:13-28: tinderbox on light_source_unlit calls ignite_light_source",
    "content scripts/skill_firemaking/scripts/light_source.rs2:14-23: at or above the level, inv_del unlit, inv_add product, light message",
    "content scripts/_unpack/377/all.obj:52109-52120,52260-52271: oil lamp/lantern unlit are light_source_unlit with product",
];

const CANON_GUARD_CITATIONS: &[&str] = &[
    "content scripts/skill_firemaking/scripts/light_source.rs2:4-16: oil lamp 12, lantern 26, helmet 65; below level returns with no message",
    "engine OpHeldUHandler.ts:118: the ignite script executes synchronously, so the return is the whole effect",
];

const EXTINGUISH_CITATIONS: &[&str] = &[
    "content scripts/skill_firemaking/scripts/light_source.rs2:34-35,41-43: [opheld2,oil_lamp_lit] and [opheld2,oil_lantern_lit] call extinguish_light_source",
    "content scripts/_unpack/377/all.obj:52127-52140,52278-52291: lit lamp and lantern iop2 Extinguish and next_obj_stage is the unlit object",
];

const EQUIP_CITATIONS: &[&str] = &[
    "engine OpHeldHandler.ts:65-68: opheld2 executes [opheld2,_] synchronously",
    "content scripts/player/scripts/equip.rs2:1,10-72: ~equip moves the carried item to worn; only elemental_shield delays",
];

const UNEQUIP_CITATIONS: &[&str] = &[
    "content scripts/player/scripts/equip.rs2:3-8,74-: [inv_button1,wornitems:worn] runs ~unequip synchronously",
];

const DARK_HOLE_CITATIONS: &[&str] = &[
    "content scripts/_unpack/377/all.loc:41820-41824: goblin_cave_entrance op1 is Climb-down",
    "content scripts/areas/area_lumbridge_swamp_caves/scripts/entrance.rs2:18-19: [oploc1,goblin_cave_entrance] returns with no effect when the entrance is not roped",
];

const SILENT_GUARD_CITATIONS: &[&str] = &[
    "content 0ed5f39138f3 scripts/skill_firemaking/scripts/light_source.rs2:4-26: oil_lantern_unlit needs level 26 and returns silently below it",
    "engine 15301127e60f OpHeldUHandler.ts:84-118: synchronous script execution",
    "engine 15301127e60f Player.ts:2297-2302,2328-2344: protected/delayed access can refuse silently before execution",
];

// Revision-289 citations are `file:line` at R289_ENGINE / R289_CONTENT.
const R289_BURY_CITATIONS: &[&str] = &[
    "engine OpHeldHandler.ts:16-19,40-43,65-68: a delayed player or a missing slot item is refused silently; else [opheld1,_bones] executes at once",
    "content scripts/skill_prayer/configs/bones.obj:1-12: bones (pack/obj.pack:527 id 526) have iop1 Bury and category bones",
    "content scripts/skill_prayer/scripts/bury_bone.rs2:1-16: dig message, p_delay(0), then inv_delslot, stat_advance(prayer) and the exact terminal message",
    "content scripts/skill_prayer/scripts/bury_bone.rs2:18-20: afk_event (engine PlayerOps.ts:1114-1116, World.ts:610-611) runs @macro_event_prayer in the same step",
    "content scripts/macro events/scripts/prayer/macro_event_prayer.rs2:1-9, macro_event_zombie.rs2:3-19, macro_event_shade.rs2:3-18, scripts/macro_events.rs2:26-45: each branch npc_adds one event npc within 1 tile synchronously, or nothing",
    "content scripts/macro events/configs/macro_events.enum (general_macro_events, macro_event_zombie_levels, macro_event_shade_levels) and pack/npc.pack:408,410-412,420-431,957: those npc types",
    "engine World.ts:1101-1112: that cycle's NPC_INFO follows its PLAYER_INFO in the same flush, so the closing PLAYER_INFO after the message shows any spawned descendant",
];

/// Npc types `@macro_event_prayer` can spawn (pack/npc.pack ids).
const R289_PRAYER_EVENT_NPCS: &[i32] = &[
    407, 409, 410, 411, 419, 420, 421, 422, 423, 424, 425, 426, 427, 428, 429, 430, 956,
];

const R289_BANK_OPEN_CITATIONS: &[&str] = &[
    "engine OpLocHandler.ts:14-18,46-56: a delayed player or an invalid op is refused; else the oploc interaction is set",
    "content scripts/interface_bank/configs/bank_booth.loc:1-7: bankbooth (pack/loc.pack:2214 id 2213) op2 Use-quickly",
    "content scripts/interface_bank/scripts/bank_booth.rs2:7: [oploc2,bankbooth] goes straight to @openbank",
    "content scripts/interface_bank/scripts/bank.rs2:18-23: openbank transmits both inventories and opens bank_main synchronously",
    "excluded: bank_booth.rs2:1-5 op1 Use starts banker dialogue when a teller is within 2 tiles",
];

const R289_BANK_WITHDRAW_CITATIONS: &[&str] = &[
    "engine InvButtonHandler.ts:14-17,28-46,51-55: a delayed player, invalid op or missing bank slot item is refused; else the script executes at once",
    "content scripts/interface_bank/scripts/bank.rs2:1-4,37-85: inv_button1-4 on bank_main:inv move the item synchronously from bank to inv",
    "content scripts/interface_bank/scripts/bank.rs2:19: openbank resets %bankcert, so the carried id is the banked id until a note toggle",
    "excluded: bank.rs2:5,41-45 inv_button5 Withdraw-X suspends on p_countdialog",
];

const R289_BANK_CLOSE_CITATIONS: &[&str] = &[
    "engine CloseModalHandler.ts:6-14: IF_CLOSE sets requestModalClose",
    "engine Player.ts:874-889,760-789: processQueues closes the modal, runs [if_close,bank_main], then the primary queue",
    "content scripts/interface_bank/scripts/bank.rs2:16,25-35: closebank stops transmits and queues reorganize_bank with delay 0",
    "engine World.ts:699-701, Player.ts:891-913: that delay-0 queue entry runs no later than the next cycle's processQueues",
];

fn bank_open(snapshot: &GameSnapshot) -> bool {
    snapshot.bank_component_id() >= 0 && snapshot.bank_loaded()
}

fn bank_closed(snapshot: &GameSnapshot) -> bool {
    snapshot.bank_component_id() < 0 && !snapshot.bank_loaded()
}

fn r289_terminal(
    id: &'static str,
    terminal: Terminal,
    citations: &'static [&'static str],
) -> Result<Contract, String> {
    Ok(Contract {
        id,
        closure: Closure::Terminal(terminal),
        citations,
    })
}

/// Revision-289 families at the pinned lab heads.
fn select_r289(action: FileAction, snapshot: &GameSnapshot) -> Result<Contract, String> {
    match action {
        FileAction::HeldOp {
            item: R289_BONES,
            op: 1,
        } => r289_terminal(
            "r289-bury-bones",
            Terminal {
                inventories: &[Domain::Carried],
                items: vec![R289_BONES],
                message: Some("You bury the bones.".into()),
                bank: None,
                settle: true,
                descendants: R289_PRAYER_EVENT_NPCS,
            },
            R289_BURY_CITATIONS,
        ),
        FileAction::LocOp {
            loc: R289_BANK_BOOTH,
            op: 2,
            ..
        } => {
            if !bank_closed(snapshot) {
                return Err("bank open contract needs a closed bank at send".into());
            }
            r289_terminal(
                "r289-bank-open-quickly",
                Terminal {
                    inventories: &[],
                    items: Vec::new(),
                    message: None,
                    bank: Some(BankEdge::Opened),
                    settle: false,
                    descendants: &[],
                },
                R289_BANK_OPEN_CITATIONS,
            )
        }
        FileAction::BankWithdraw { item, op: 1..=4 } => {
            if !bank_open(snapshot) {
                return Err("bank withdraw contract needs an open loaded bank".into());
            }
            r289_terminal(
                "r289-bank-withdraw",
                Terminal {
                    inventories: &[Domain::Carried, Domain::Bank],
                    items: vec![item],
                    message: None,
                    bank: None,
                    settle: false,
                    descendants: &[],
                },
                R289_BANK_WITHDRAW_CITATIONS,
            )
        }
        FileAction::CloseModal => {
            if !bank_open(snapshot) {
                return Err("close contract covers only an open bank".into());
            }
            r289_terminal(
                "r289-bank-close",
                Terminal {
                    inventories: &[],
                    items: Vec::new(),
                    message: None,
                    bank: Some(BankEdge::Closed),
                    settle: true,
                    descendants: &[],
                },
                R289_BANK_CLOSE_CITATIONS,
            )
        }
        _ => Err("no source-backed revision-289 contract for this action family or target".into()),
    }
}

/// Select the contract for one resolved action, or refuse it.
pub(super) fn select(
    heads: Heads<'_>,
    action: FileAction,
    snapshot: &GameSnapshot,
) -> Result<Contract, String> {
    if heads.revision == 289 && heads.engine == R289_ENGINE && heads.content == R289_CONTENT {
        return select_r289(action, snapshot);
    }
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
            // Canon light_source.rs2:4-16 and the SC-01 line both refuse below
            // the object's firemaking level with no message.
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
                    id: if sc01 {
                        "r377-sc01-light-source-below-level"
                    } else {
                        "r377-canon-light-source-below-level"
                    },
                    closure: Closure::SilentGuard {
                        skill: FIREMAKING,
                        level,
                        item: other,
                    },
                    citations: if sc01 {
                        SILENT_GUARD_CITATIONS
                    } else {
                        CANON_GUARD_CITATIONS
                    },
                });
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
                    bank: None,
                    settle: false,
                    descendants: &[],
                }),
                citations: LIGHT_CITATIONS,
            })
        }
        FileAction::HeldOp { item, op: 2 }
            if canon && [ids.oil_lamp_lit, ids.oil_lantern_lit].contains(&item) =>
        {
            let unlit = if item == ids.oil_lamp_lit {
                ids.oil_lamp_unlit
            } else {
                ids.oil_lantern_unlit
            };
            Ok(Contract {
                id: "r377-canon-extinguish",
                closure: Closure::Terminal(Terminal {
                    inventories: &[Domain::Carried],
                    items: vec![item, unlit],
                    message: None,
                    bank: None,
                    settle: false,
                    descendants: &[],
                }),
                citations: EXTINGUISH_CITATIONS,
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
                    bank: None,
                    settle: false,
                    descendants: &[],
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
                    bank: None,
                    settle: false,
                    descendants: &[],
                }),
                citations: UNEQUIP_CITATIONS,
            })
        }
        FileAction::LocOp { loc, op: 1, .. } if loc == ids.goblin_cave_entrance => Ok(Contract {
            id: "r377-dark-hole-unroped",
            closure: Closure::SilentReturn,
            citations: DARK_HOLE_CITATIONS,
        }),
        _ => Err("no source-backed terminal contract for this action family or target".into()),
    }
}

/// Whether an admitted check named `name` observes this contract's own
/// terminal domain: the family's item ids in one of its containers, its
/// exact message, or its bank modal transition. A direct root never closes
/// from an unrelated true predicate (LD2 §5.1 route 1).
pub(super) fn closes_direct_root(
    contract: &Contract,
    name: &str,
    check: &scenario::file_scenario::FileCheck,
    selected: &api::game_data::SelectedGameData,
) -> bool {
    use scenario::file_scenario::{Container, RootDomain};
    let Closure::Terminal(terminal) = &contract.closure else {
        return false;
    };
    let message = terminal
        .message
        .as_deref()
        .is_some_and(|text| check.observes(RootDomain::Message(text), selected));
    let items = terminal.inventories.iter().any(|domain| {
        let container = match domain {
            Domain::Carried => Container::Inventory,
            Domain::Worn => Container::Equipment,
            Domain::Bank => Container::Bank,
        };
        check.observes(
            RootDomain::Items {
                container,
                ids: &terminal.items,
            },
            selected,
        )
    });
    let bank = match terminal.bank {
        Some(BankEdge::Opened) => name == "bank_open",
        Some(BankEdge::Closed) => name == "bank_closed",
        None => false,
    };
    message || items || bank
}

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod tests;
