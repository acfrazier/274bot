use std::sync::{Arc, Mutex};

use api::snapshot::ChatLineView;

use super::combat::{BRASS_KEY_ID, RUNE_SCIMITAR_ID};
use super::pair::{follow_step, nav_kit_steps, tele_step};
use super::production::{COINS_ID, HITPOINTS_STAT, MAGIC_STAT};
use super::shop::{shop_buyout_open_npc_bank, GUNDAI_BANK_APPROACH};
use crate::*;

/// The `walk` scenario: log in `test`/`test`, mainland-hop into the
/// Lumbridge courtyard, walk south across the open courtyard in two steps,
/// and prove the player is standing at (3220, 3212, 0). The landing tile
/// after the mainland tele is (3220, 3220) or (3220, 3222); both steps
/// route through open, walkable tiles on the whole-world `NavWorld`.
pub(crate) fn walk_scenario() -> Scenario {
    let mid = WorldTile {
        x: 3220,
        z: 3216,
        level: 0,
    };
    let dest = WorldTile {
        x: 3220,
        z: 3212,
        level: 0,
    };
    Scenario {
        name: "walk",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "walk to courtyard mid",
                kind: StepKind::Walk { dest: mid },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: mid.x,
                        z: mid.z,
                        level: 0,
                    },
                    budget_ticks: 90,
                },
            },
            Step {
                name: "walk to courtyard south",
                kind: StepKind::Walk { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: 0,
                    },
                    budget_ticks: 90,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

/// The `nav_full` scenario: log in `test`/`test`, mainland-hop into the
/// Lumbridge courtyard, then drive the whole-world nav proof. `find` runs
/// the Dijkstra router over the **live scene's** collision map (so the
/// route is one the client can actually walk — the baked pack's boolean
/// walk grid can diverge from the live collision) plus the transport
/// graph derived from the baked whole-world pack, and `Traveller::follow`
/// drives the route one step per game tick until arrival. The destination is a
/// concrete Lumbridge tile from the pack — (3220, 3264, 0), 44 chebyshev
/// tiles north of the tele landing — in mapsquare (50,51), which the
/// pre-bake 2-square pack (m50_50 + m44_53) never covered, so the walk
/// crosses the z=3264 square boundary. It is walk-only (no boat/teleport:
/// those have no content-derivable origin tile); the route is checked at
/// arm time, and the run fails with a clear message if no walk path
/// exists. The step budget is sized for a ~100-tile walk plus re-routing.
pub fn nav_full_scenario() -> Scenario {
    let dest = WorldTile {
        x: 3220,
        z: 3264,
        level: 0,
    };
    Scenario {
        name: "nav_full",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![Step {
            name: "follow the whole-world route",
            kind: StepKind::Follow { dest },
            wait: Wait {
                arm: Proof::Arrived {
                    x: dest.x,
                    z: dest.z,
                    level: dest.level,
                },
                budget_ticks: 600,
            },
        }],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_full terminal"),
            nav: nav_test_paints(),
            ..Default::default()
        },
    }
}

/// Single-close recovery at the Catherby range-house door. Pause the armed
/// follow when the walker sees its first Open take effect. Resume that same
/// follow only after the companion's one Close takes effect on both clients.
pub(crate) fn nav_door_scenario() -> Scenario {
    let recovery = Arc::new(Mutex::new(DoorRecovery::default()));
    let walker = Arc::clone(&recovery);
    let observed = Arc::clone(&recovery);
    let summary = Arc::clone(&recovery);
    let arrive = Proof::Arrived {
        x: DEST.x,
        z: DEST.z,
        level: DEST.level,
    };
    let outside = Proof::Arrived {
        x: OUTSIDE.x,
        z: OUTSIDE.z,
        level: OUTSIDE.level,
    };
    Scenario {
        name: "nav_door",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "tele the walker to the Catherby outside stand",
                kind: StepKind::Perform {
                    // Return true on send: the cheat is queued through the
                    // ISAAC sink, the arm waits for the tele to land.
                    send: Box::new(|c, _| cheat(c, WALKER_TELE).is_sent()),
                },
                wait: Wait {
                    arm: outside,
                    budget_ticks: 120,
                },
            },
            Step {
                name: "start with the range-house door closed",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| match live_door_leaf(c) {
                        Some(CLOSED_ID) => true,
                        // The previous run may have left the real door open.
                        Some(OPEN_ID) => op_loc(c, OPEN_DOOR.x, OPEN_DOOR.z, OPEN_ID),
                        _ => false,
                    }),
                },
                wait: Wait {
                    arm: Proof::LocIdNear {
                        id: CLOSED_ID,
                        x: DOOR.x,
                        z: DOOR.z,
                        level: DOOR.level,
                        radius: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "return to the outside stand after door setup",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        door_client_tile(c) == Some(OUTSIDE) || cheat(c, WALKER_TELE).is_sent()
                    }),
                },
                wait: Wait {
                    arm: outside,
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow through the range-house door",
                kind: StepKind::FollowWhen {
                    dest: DEST,
                    may_poll: Box::new(move |c, snapshot| {
                        let mut state = walker.lock().expect("door recovery witness mutex");
                        let before = *state;
                        let here = door_client_tile(c);
                        let poll = state.walker_poll(
                            live_door_leaf(c),
                            here,
                            snapshot.chat_lines(),
                            || cheat(c, "getvar runemysteries").is_sent(),
                        );
                        if debug_enabled() && *state != before {
                            eprintln!("[nav-door] walker {before:?} -> {state:?} at {here:?}");
                        }
                        poll
                    }),
                },
                wait: Wait {
                    arm: arrive,
                    budget_ticks: 600,
                },
            },
            Step {
                name: "verify the door recovery and crossing",
                kind: StepKind::Await {
                    evidence: "single_close_reopen_and_cross",
                    ready: Box::new(move |_| {
                        *observed.lock().expect("door recovery witness mutex")
                            == DoorRecovery::Crossed
                    }),
                    summary: Some(Box::new(move || {
                        let state = summary.lock().expect("door recovery witness mutex");
                        format!("single-close recovery: {state:?}")
                    })),
                },
                // The armed follow has terminated. Its witness cannot change
                // now, so this check needs no further observation window.
                wait: Wait {
                    arm: arrive,
                    budget_ticks: 1,
                },
            },
        ],
        proof: arrive,
        companions: vec![Companion {
            profile: 1,
            per_frame: {
                let mut slot = CloserSlot::default();
                Box::new(move |c| {
                    closer_frame(c, &mut slot, &recovery);
                    false
                })
            },
        }],
        settings: ScenarioSettings {
            full_rate: true,
            only_render_selected: false,
            nav: nav_test_paints(),
            ..Default::default()
        },
    }
}

/// Inside the East Ardougne house room sealed by the closed diagonal
/// (`wall_diagonal`, shape 9) door `loc_1530` at (2669,3316).
const DIAGONAL_ROOM: WorldTile = WorldTile {
    x: 2671,
    z: 3316,
    level: 0,
};
/// The East Ardougne south bank booth stand the 0.1.8.1 Thiever could not
/// reach from that room.
const DIAGONAL_BANK: WorldTile = WorldTile {
    x: 2655,
    z: 3286,
    level: 0,
};

/// The `nav_diagonal_door` scenario: cheat-tele into the East Ardougne
/// house room whose only exit is the closed diagonal door `loc_1530` at
/// (2669,3316) (closed is the engine default), then `Follow` the baked
/// route to the south bank. The route's first transport is the door's
/// packed diagonal crossing, so the traveller opens it and walks through
/// the freed door tile. PASS is standing on the bank stand.
pub(crate) fn nav_diagonal_door_scenario() -> Scenario {
    Scenario {
        name: "nav_diagonal_door",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            tele_step("tele into the diagonal-door room", DIAGONAL_ROOM),
            follow_step(
                "follow through the diagonal door to the south bank",
                DIAGONAL_BANK,
            ),
        ],
        proof: Proof::Arrived {
            x: DIAGONAL_BANK.x,
            z: DIAGONAL_BANK.z,
            level: DIAGONAL_BANK.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            terminal_shot: Some("nav_diagonal_door terminal"),
            nav: nav_test_paints(),
            ..Default::default()
        },
    }
}

/// West of the Heroes' Guild double doors (`herodoor_l/_r` at
/// (2902,3510)/(2902,3511), `areas_heroes_guild/scripts/heroes_entrance.rs2`).
const HERO_GUILD_OUTSIDE: WorldTile = WorldTile {
    x: 2899,
    z: 3510,
    level: 0,
};
/// Inside the guild's entrance hall, east of the doors.
const HERO_GUILD_INSIDE: WorldTile = WorldTile {
    x: 2905,
    z: 3510,
    level: 0,
};

/// The `nav_quest_door` scenario: the quest-stage door live twin. Set
/// `%heroquest` to `^hero_complete` (15) with the dev `setvar` cheat, relog
/// so the login-time quest list turns the Hero's Quest row green (as the
/// nav kit does for its transport quests), cheat-tele west of the Heroes'
/// Guild, then `Follow` inside. The guild doors open only for
/// `%heroquest >= ^hero_complete`, which the pack carries as the completed
/// "Hero's Quest" journal row, so the route's door leg exists only once
/// the journal is green; PASS is standing in the entrance hall.
pub(crate) fn nav_quest_door_scenario() -> Scenario {
    let start = HERO_GUILD_OUTSIDE;
    let dest = HERO_GUILD_INSIDE;
    Scenario {
        name: "nav_quest_door",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "complete Hero's Quest and its required quests",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        // quest_hero.rs2:1-7 requires these quests and either Shield of Arrav gang.
                        cheat(c, "setvar zanaris 6");
                        cheat(c, "setvar dragonquest 10");
                        cheat(c, "setvar arthur 7");
                        cheat(c, "setvar blackarmgang 4");
                        cheat(c, "setvar heroquest 15");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: 3220,
                        z: 3220,
                        level: 0,
                    },
                    budget_ticks: 30,
                },
            },
            Step {
                name: "relog so the quest journal refreshes",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "journal shows Hero's Quest complete",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Hero's Quest",
                    },
                    budget_ticks: 200,
                },
            },
            tele_step("tele west of the Heroes' Guild", start),
            follow_step("follow through the guild doors", dest),
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_quest_door terminal"),
            ..Default::default()
        },
    }
}

/// North-facing straight wall in `content/maps/m44_53.jm2:4200`.
const DOOR: WorldTile = WorldTile {
    x: 2816,
    z: 3438,
    level: 0,
};
// Opening moves the north-facing leaf one tile north
// (`content/scripts/doors/scripts/door_procs.rs2:1-9,21-29`).
const OPEN_DOOR: WorldTile = WorldTile {
    z: DOOR.z + 1,
    ..DOOR
};
pub(crate) const CLOSED_ID: i32 = 1530;
pub(crate) const OPEN_ID: i32 = 1531;
/// Briefed outside stand (west of pack origin 2816; on-pack fallback is
/// (2816,3436), the walkable tile south of the door).
const OUTSIDE: WorldTile = WorldTile {
    x: 2813,
    z: 3436,
    level: 0,
};
/// Inside stand, north of the door.
const DEST: WorldTile = WorldTile {
    x: 2817,
    z: 3443,
    level: 0,
};
/// `::tele` to OUTSIDE (level, mx, mz, lx, lz).
const WALKER_TELE: &str = "tele 0,43,53,61,44";
/// Inside, diagonal to the door (2817,3439) — off the 2816 corridor.
const CLOSER_STAND: WorldTile = WorldTile {
    x: 2817,
    z: 3439,
    level: 0,
};
const CLOSER_TELE: &str = "tele 0,44,53,1,47";

/// Client-cycle state of the closer companion (profile 1).
#[derive(Default)]
struct CloserSlot {
    /// First `scene_state == 2` was observed; that frame host-play still
    /// queues `mainland_hop` after `per_frame`, so the Catherby tele
    /// waits one client cycle.
    scene2_seen: bool,
    tele_sent: bool,
    last_gen: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DoorRecovery {
    #[default]
    WaitingForCloser,
    Ready,
    DrainingOpen {
        chat_after: i32,
    },
    DrainQueryRefused,
    CloseReady,
    CloseSent,
    CloseRefused,
    CloseConfirmed,
    Reopening,
    Reopened,
    Crossed,
}

impl DoorRecovery {
    fn walker_poll(
        &mut self,
        leaf: Option<i32>,
        here: Option<WorldTile>,
        chat: &[ChatLineView],
        query: impl FnOnce() -> bool,
    ) -> bool {
        match (*self, leaf) {
            (Self::Ready, Some(OPEN_ID)) => {
                // DoorRetry can resend an unanswered Open at +2 game ticks
                // (nav-executor transport_hop.rs:662-673,709-718). Freeze the
                // traveller now, but drain its earlier sends before Close.
                // The engine decodes the read-only getvar after prior Open
                // packets in stream order (NetworkPlayer.ts:55-138). It
                // writes the reply during input (ClientCheatHandler.ts:306-318,
                // NetworkPlayer.ts:191-226, TcpClientSocket.ts:15-17).
                // TCP data enters the input buffer only between synchronous
                // ticks (World.ts:341-509, TcpServer.ts:24-31, app.ts:36-39).
                // Thus the Close can be decoded only in a later tick.
                // Before the door opens, each Open replaces the one pending
                // interaction. After it opens, the engine rejects Open for
                // the missing closed leaf (OpLocHandler.ts:30-35,53-54).
                let chat_after = chat.first().map_or(0, |line| line.sequence);
                *self = if query() {
                    Self::DrainingOpen { chat_after }
                } else {
                    Self::DrainQueryRefused
                };
                false
            }
            (Self::DrainingOpen { chat_after }, Some(OPEN_ID))
                if chat.iter().any(|line| {
                    line.sequence > chat_after
                        && line.type_ == 0
                        && line
                            .text
                            .strip_prefix("get runemysteries: ")
                            .is_some_and(|value| value.parse::<i32>().is_ok())
                }) =>
            {
                *self = Self::CloseReady;
                false
            }
            (Self::CloseConfirmed, Some(CLOSED_ID)) => {
                // Do not resume on a stale open read from before Close.
                *self = Self::Reopening;
                true
            }
            (Self::Reopening, Some(OPEN_ID)) => {
                *self = Self::Reopened;
                true
            }
            (Self::Reopened, _) if here == Some(OPEN_DOOR) => {
                *self = Self::Crossed;
                true
            }
            (Self::Ready | Self::Reopening, Some(CLOSED_ID))
            | (Self::Reopened | Self::Crossed, _) => true,
            _ => false,
        }
    }

    fn closer_poll(&mut self, leaf: Option<i32>, send: impl FnOnce() -> bool) {
        match (*self, leaf) {
            (Self::CloseReady, Some(OPEN_ID)) => {
                *self = if send() {
                    Self::CloseSent
                } else {
                    Self::CloseRefused
                };
            }
            (Self::CloseSent, Some(CLOSED_ID)) => *self = Self::CloseConfirmed,
            _ => {}
        }
    }
}

fn closer_frame(c: &mut Client, s: &mut CloserSlot, recovery: &Mutex<DoorRecovery>) {
    let Some(here) = door_client_tile(c) else {
        return;
    };
    if stage_closer_tele(c, here, s) || c.scene_state != 2 || c.gens.player == s.last_gen {
        return;
    }
    s.last_gen = c.gens.player;
    let mut state = recovery.lock().expect("door recovery witness mutex");
    let before = *state;
    if before == DoorRecovery::WaitingForCloser && here == CLOSER_STAND {
        *state = DoorRecovery::Ready;
    }
    if matches!(before, DoorRecovery::CloseReady | DoorRecovery::CloseSent) {
        // OP_LOC1 swaps the leaf without p_delay for straight walls. Wait
        // for the closed leaf on a later PLAYER_INFO, not a timer or the
        // accepted send (`content/scripts/doors/scripts/doors.rs2:1-2,10-37`).
        state.closer_poll(live_door_leaf(c), || {
            op_loc(c, OPEN_DOOR.x, OPEN_DOOR.z, OPEN_ID)
        });
    }
    if debug_enabled() && *state != before {
        eprintln!(
            "[nav-door] closer {before:?} -> {state:?} at player generation {}",
            s.last_gen
        );
    }
}

/// Host-play queues `mainland_hop` *after* `per_frame` on the first
/// `scene_state == 2`. Skip that frame; Catherby-tele only once `here` is
/// the Lumbridge courtyard (or `x > 3100` and not already at Catherby).
fn stage_closer_tele(c: &mut Client, here: WorldTile, s: &mut CloserSlot) -> bool {
    if at_catherby(here) || s.tele_sent {
        return false;
    }
    if c.scene_state != 2 {
        return false;
    }
    if !s.scene2_seen {
        s.scene2_seen = true;
        return false;
    }
    if !at_lumbridge(here) && here.x <= 3100 {
        if debug_enabled() {
            eprintln!(
                "[nav-closer] waiting mainland, here={here:?} scene={}",
                c.scene_state
            );
        }
        return false;
    }
    if debug_enabled() {
        eprintln!("[nav-closer] tele {CLOSER_TELE} from {here:?}");
    }
    cheat(c, CLOSER_TELE);
    s.tele_sent = true;
    true
}

fn door_client_tile(c: &Client) -> Option<WorldTile> {
    let (x, z) = c.local_route()?;
    let (bx, bz) = c.build_base();
    Some(WorldTile {
        x: bx + x,
        z: bz + z,
        level: c.minusedlevel,
    })
}

fn live_door_leaf(c: &Client) -> Option<i32> {
    let (bx, bz) = c.build_base();
    door_leaf(|x, z| c.loc_typecode(x - bx, z - bz).map(|tc| (tc >> 14) & 0x7fff))
}

pub(crate) fn door_leaf(lookup: impl Fn(i32, i32) -> Option<i32>) -> Option<i32> {
    let closed = lookup(DOOR.x, DOOR.z) == Some(CLOSED_ID);
    let open = lookup(OPEN_DOOR.x, OPEN_DOOR.z) == Some(OPEN_ID);
    match (closed, open) {
        (true, false) => Some(CLOSED_ID),
        (false, true) => Some(OPEN_ID),
        _ => None,
    }
}

fn at_lumbridge(here: WorldTile) -> bool {
    here.x >= 3200 && here.x < 3264 && here.z >= 3200 && here.z < 3264
}

fn at_catherby(here: WorldTile) -> bool {
    here.x >= 2800 && here.x < 2860 && here.z >= 3420 && here.z < 3460
}

/// Vigroy's Shilo Village cart driver tile (npc 511, m44_46 local
/// (18,10)): the `at` of the packed Shilo→Brimhaven Npc edge.
const SHILO_DRIVER: WorldTile = WorldTile {
    x: 2834,
    z: 2954,
    level: 0,
};
/// The Shilo→Brimhaven cart's landing tile (m43_50 local (24,14)): the
/// packed edge's `to`.
const BRIM_CART: WorldTile = WorldTile {
    x: 2776,
    z: 3214,
    level: 0,
};

/// The `nav_cart` scenario: set `%zombiequeen` to 15 on the `test` account,
/// then relog so the completed Shilo Village journal row reaches the client.
/// Content defines `^zombiequeen_complete = 15`
/// (`content/scripts/general/configs/quest.constant:62`). Give the fare,
/// teleport to Vigroy and follow the cart hop to Brimhaven. Both drivers
/// refuse the journey below the completion value
/// (`area_shilo/scripts/vigroy.rs2:17-20`,
/// `area_brimhaven/scripts/hajedy.rs2:17-20,26-29`).
pub(crate) fn nav_cart_scenario() -> Scenario {
    let driver = SHILO_DRIVER;
    let dest = BRIM_CART;
    Scenario {
        name: "nav_cart",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "set Shilo Village complete on the test account",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar zombiequeen 15");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: 3220,
                        z: 3220,
                        level: 0,
                    },
                    budget_ticks: 30,
                },
            },
            Step {
                name: "relog so the Shilo Village journal refreshes",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "verify Shilo Village completion in the journal",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Shilo Village",
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "give the fare and tele to the Shilo cart driver",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "give coins 500");
                        cheat(c, &tele_args(driver.level, driver.x, driver.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: driver.x,
                        z: driver.z,
                        level: driver.level,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow the cart to Brimhaven",
                kind: StepKind::Follow { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_cart terminal"),
            ..Default::default()
        },
    }
}

/// Aubury's Varrock rune-shop anchor (3253,3401): `^essence_mine_to_aubury`
/// = `0_50_53_53_9` — the tile the mine exit portal returns to after
/// entering through Aubury.
const AUBURY_ANCHOR: WorldTile = WorldTile {
    x: 3253,
    z: 3401,
    level: 0,
};
/// Shop-side setup beside Aubury's authored home (3253,3402), not the
/// essence session's separate return anchor.
const AUBURY_SHOP_STAND: WorldTile = WorldTile {
    x: 3254,
    z: 3402,
    level: 0,
};
/// The Rune Essence mine pad (m45_75 local (32,33)): the packed entry
/// edge's landing anchor (the real landing is randomised in the mine).
const MINE_PAD: WorldTile = WorldTile {
    x: 2912,
    z: 4833,
    level: 0,
};

/// The `nav_essence` scenario: the EssenceSession execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, seed only
/// Rune Mysteries, and relog to observe its completed journal state.
/// Whole-world quest completion queues unrelated rewards and can still
/// own the player when the transport fixture starts. Enter from beside
/// Aubury, then follow the session-gated exit back to his return anchor.
pub(crate) fn nav_essence_scenario() -> Scenario {
    Scenario {
        name: "nav_essence",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "seed Rune Mysteries without unrelated quest rewards",
                kind: StepKind::Repeat {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "setvar runemysteries 6");
                        cheat(c, "getvar runemysteries");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get runemysteries: 6",
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "relog so the Rune Mysteries journal refreshes",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Rune Mysteries Quest",
                    },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to Aubury's shop",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(
                            c,
                            &tele_args(
                                AUBURY_SHOP_STAND.level,
                                AUBURY_SHOP_STAND.x,
                                AUBURY_SHOP_STAND.z,
                            ),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: AUBURY_SHOP_STAND.x,
                        z: AUBURY_SHOP_STAND.z,
                        level: AUBURY_SHOP_STAND.level,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow into the essence mine",
                kind: StepKind::Follow { dest: MINE_PAD },
                wait: Wait {
                    arm: Proof::EssenceMine,
                    budget_ticks: 600,
                },
            },
            Step {
                name: "follow out to Aubury through the exit portal",
                kind: StepKind::Follow {
                    dest: AUBURY_ANCHOR,
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: AUBURY_ANCHOR.x,
                        z: AUBURY_ANCHOR.z,
                        level: AUBURY_ANCHOR.level,
                        radius: 2,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::ArrivedNear {
            x: AUBURY_ANCHOR.x,
            z: AUBURY_ANCHOR.z,
            level: AUBURY_ANCHOR.level,
            radius: 2,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_essence terminal"),
            ..Default::default()
        },
    }
}

/// Stand at the authored maze entrance, one tile north of Elkoy's home
/// (npc 473 at m39_49 local (8,55)). Never set up on the NPC's own tile.
const ELKOY_MAZE_SIDE: WorldTile = WorldTile {
    x: 2504,
    z: 3192,
    level: 0,
};
/// The village maze coord (`^elkoy_maze_coord = 0_39_49_19_23`): the
/// packed escort edge's `to` — the exact tile the maze-side Elkoy's
/// `p_telejump(` lands on (the script's own landing, never a snap).
const ELKOY_MAZE_COORD: WorldTile = WorldTile {
    x: 2515,
    z: 3159,
    level: 0,
};

/// The `nav_elkoy` scenario: the Elkoy OP_NPC execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, seed only
/// Tree Gnome Village and relog to observe its completed journal state.
/// Follow from beside maze-side Elkoy through the real NPC escort and
/// its "Yes please." choice to the exact content landing (2515,3159).
pub(crate) fn nav_elkoy_scenario() -> Scenario {
    let driver = ELKOY_MAZE_SIDE;
    let dest = ELKOY_MAZE_COORD;
    Scenario {
        name: "nav_elkoy",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "seed Tree Gnome Village without unrelated quest rewards",
                kind: StepKind::Repeat {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "setvar treequest 9");
                        cheat(c, "getvar treequest");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get treequest: 9",
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "relog so the Tree Gnome Village journal refreshes",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::QuestDone {
                        name: "Tree Gnome Village",
                    },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to the maze-side Elkoy",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(driver.level, driver.x, driver.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: driver.x,
                        z: driver.z,
                        level: driver.level,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow Elkoy into the village",
                kind: StepKind::Follow { dest },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_elkoy terminal"),
            ..Default::default()
        },
    }
}

/// The packed dueling-ring landing: the Al Kharid Duel Arena (m51_50
/// local (51,35)) — the `to` of the packed `ring_of_dueling_8` rub edge
/// (obj 2552, `opheld4`), a random standable tile within the
/// `map_findsquare` scatter, never the tile exactly.
const DUEL_ARENA: WorldTile = WorldTile {
    x: 3315,
    z: 3235,
    level: 0,
};

/// The `nav_tele` scenario: the packed Teleport execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, clear the
/// persistent slot's backpack (`~clearinv`) and cheat-give a charged
/// dueling ring (the packed jewellery rub edge's `item_req`), then
/// `Follow` with `allow_teleports` on to the Al Kharid Duel Arena —
/// a destination the 2-game-tick rub edge always beats (the walk is toll-
/// gated and over a hundred game ticks, so the packed leg only ever routes
/// when the layer is on and the item is held). The traveller rubs the
/// held ring (`OpTarget::Item` + option 4 — never the WalkTo `::tele`
/// cheat), answers the destination choice the rub opens, and PASSes on
/// `TravelOutcome::Arrived` within the packed landing's scatter radius.
/// Relog after the mainland hop is required: `mainland_hop` does not
/// refresh tutorial-locked side icons, and the jewellery arm waits on
/// `inventory()` (the bound inv tab), not the `inv()` fallback.
pub(crate) fn nav_tele_scenario() -> Scenario {
    let dest = DUEL_ARENA;
    Scenario {
        name: "nav_tele",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "relog so the inventory tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    // mainlandAccount tele+setvar does not relog; side icons
                    // stay tutorial-locked. The jewellery hop reads
                    // snapshot.inventory(), which needs side tab 3.
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "clear the backpack and give the charged dueling ring",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        // The persistent `test` slot's backpack fills up
                        // across prior live twins, so the give silently
                        // drops when there is no room — clear the default
                        // inventory first (`[debugproc,clearinv]`).
                        cheat(c, "~clearinv");
                        cheat(c, "give ring_of_dueling_8 1");
                        true
                    }),
                },
                // The arm waits for the ring to actually land in the
                // inventory: the WorldState of the follow step then proves
                // the packed rub edge's `item_req`, or the router falls
                // back to the walk.
                wait: Wait {
                    arm: Proof::Item {
                        name: "Ring of dueling(8)",
                        count: 1,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "follow the packed ring rub to the Duel Arena",
                kind: StepKind::FollowTele { dest },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: dest.x,
                        z: dest.z,
                        level: dest.level,
                        radius: 2,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::ArrivedNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius: 2,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_tele terminal"),
            ..Default::default()
        },
    }
}

/// Irena's desert-side stand (m51_48 local (38,42) = (3302,3114)).
const SHANTAY_DESERT_START: WorldTile = WorldTile {
    x: 3302,
    z: 3114,
    level: 0,
};
/// The Shantay bank chest's stand on the pass side (3308,3120).
const SHANTAY_PASS_DEST: WorldTile = WorldTile {
    x: 3308,
    z: 3120,
    level: 0,
};

/// Upstream `shantay-pass-route-test` start, west of the Shantay pass
/// (3246,3092); the tail reaches it with the same `tele` the upstream
/// fixture uses.
const SHANTAY_TOLL_START: WorldTile = WorldTile {
    x: 3246,
    z: 3092,
    level: 0,
};
/// Upstream `shantay-pass-route-test` destination: Varrock (3213,3424),
/// arrival radius 3.
const SHANTAY_VARROCK: WorldTile = WorldTile {
    x: 3213,
    z: 3424,
    level: 0,
};
/// Melee and Hitpoints seeded to 99 before the toll tail: combat 113, above
/// the 84 cap of the Ugthanki zones on the desert route.
const SHANTAY_TOLL_COMBAT_SKILLS: [&str; 4] = ["attack", "strength", "defence", "hitpoints"];

/// The `nav_shantay` scenario exercises both henge branches with exact
/// WalkTo arrival: from Irena, it walks to the Shantay bank chest with no
/// pass, using the free desert exit; it then gets a pass and walks back to
/// Irena through the paid branch. Both routes use the real loc and content.
/// The tail then ports upstream `shantay-pass-route-test`: from the
/// desert start with 10 coins and no pass, it walks to Varrock and must
/// finish with the toll paid.
pub(crate) fn nav_shantay_scenario() -> Scenario {
    Scenario {
        name: "nav_shantay",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "clear the backpack and teleport to Irena",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        cheat(c, "tele 0,51,48,38,42");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_DESERT_START.x,
                        z: SHANTAY_DESERT_START.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "WalkTo the bank chest through the free desert exit",
                kind: StepKind::Walk {
                    dest: SHANTAY_PASS_DEST,
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_PASS_DEST.x,
                        z: SHANTAY_PASS_DEST.z,
                        level: 0,
                    },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "clear the backpack and give the Shantay pass",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        // The gated branch shows its first-crossing
                        // disclaimer dialog (three mesboxes + a "Go into
                        // Desert?" choice) when the player lacks the
                        // disclaimer item — give the disclaimer the script
                        // hands out after any prior crossing so the branch
                        // goes straight to the pass-handover chat the
                        // traveller drives for the loc-4031 hop.
                        cheat(c, "give thshantaydisc 1");
                        cheat(c, "give shantay_pass 1");
                        true
                    }),
                },
                // Wait for the pass to land so the following Walk's
                // WorldState can prove its consumed-item requirement.
                wait: Wait {
                    arm: Proof::Item {
                        name: "Shantay pass",
                        count: 1,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "WalkTo back to Irena through the paid henge",
                kind: StepKind::Walk {
                    dest: SHANTAY_DESERT_START,
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_DESERT_START.x,
                        z: SHANTAY_DESERT_START.z,
                        level: 0,
                    },
                    budget_ticks: 600,
                },
            },
            // This scenario's Never routing overlay refuses the desert walk
            // for the new level-3 account: Ugthanki (L42) and Desert Wolves
            // (L27) hunt (`desert.npc` `huntmode=cowardly`,
            // `check_nottoostrong=outside_wilderness`) players up to combat
            // 84 and 54. Seed the account's own melee stats to 99 (combat
            // 113), so neither can hunt it and the route is safe under the
            // overlay, then drain the level-up dialogs.
            Step {
                name: "set melee stats to 99 so the desert hunters ignore the account",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        for skill in SHANTAY_TOLL_COMBAT_SKILLS {
                            cheat(c, &format!("setstat {skill} 99"));
                        }
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat {
                        id: HITPOINTS_STAT,
                        min: 99,
                    },
                    budget_ticks: 60,
                },
            },
            drain_setstat_levelups_before_hostile_tele(),
            // ported from rs2b0t e2e/shantay-pass-route-test.ts @2de6bb23:
            // upstream fixture is the desert start with 10 coins only, an empty
            // bank and no pass (`~clearbank` and `~clearinv worn` here). The walk
            // is a plain Walk; walkResilient retries are the planner's job. The
            // upstream crossing-tile watcher (x3300-3306, z3118-3122, level 0) is
            // the walk's VisitedWithin latch. The "Go-through Shantay pass" and
            // "Al Kharid toll gate: crossed" lines are bot `log` output, not game
            // chat, and no proof reads them, so the toll is checked as the coins
            // reaching zero. Upstream's `speed 300` is not ported (the harness keeps
            // the 600ms engine speed), and its `--expect-unreachable` baseline is
            // not ported: that mode passes only when the route fails (upstream
            // `shantay-pass-route-test.ts:348-351`), which inverts this cell's oracle.
            Step {
                name: "clear the backpack, give the toll coins, and teleport to the desert start",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        cheat(c, "~clearinv worn");
                        cheat(c, "~clearbank");
                        cheat(c, "give coins 10");
                        cheat(
                            c,
                            &tele_args(
                                SHANTAY_TOLL_START.level,
                                SHANTAY_TOLL_START.x,
                                SHANTAY_TOLL_START.z,
                            ),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_TOLL_START.x,
                        z: SHANTAY_TOLL_START.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            // The upstream fixture check before the walk: the toll coins are in
            // the backpack. ItemId is a lower bound (>= 10); the cleared
            // inventory and worn slots are the fixture's exact-stack part.
            Step {
                name: "the backpack holds the 10 toll coins before the walk",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ItemId {
                        id: COINS_ID,
                        count: 10,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "WalkTo Varrock through the Shantay Pass and the Al Kharid toll gate",
                kind: StepKind::Walk {
                    dest: SHANTAY_VARROCK,
                },
                wait: Wait {
                    arm: Proof::VisitedWithin {
                        min_x: 3300,
                        max_x: 3306,
                        min_z: 3118,
                        max_z: 3122,
                        level: 0,
                    },
                    budget_ticks: 800,
                },
            },
            Step {
                name: "the walk ended within the Varrock arrival radius",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: SHANTAY_VARROCK.x,
                        z: SHANTAY_VARROCK.z,
                        level: 0,
                        radius: 3,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "the Al Kharid toll took the 10 coins",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ItemIdAtMost {
                        id: COINS_ID,
                        count: 0,
                    },
                    budget_ticks: 20,
                },
            },
        ],
        proof: Proof::ArrivedNear {
            x: SHANTAY_VARROCK.x,
            z: SHANTAY_VARROCK.z,
            level: 0,
            radius: 3,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(600),
            terminal_shot: Some("nav_shantay terminal"),
            ..Default::default()
        },
    }
}

/// One tile south of packed 1804 `at` (3115,3449). A headed follow that
/// teled onto the `at` refused `StaleTarget` with hops=0 while a continue
/// dialog was still up.
const BRASS_HUT_APPROACH: WorldTile = WorldTile {
    x: 3115,
    z: 3448,
    level: 0,
};
/// Inside the hut, the stand south of the cellar ladder. North of 1804, so a
/// follow from `BRASS_HUT_APPROACH` has to take the keyed door. The ladder
/// itself (`ladder_cellar` loc 1754, `m48_53.jm2` LOC `0 44 60: 1754 10 3`
/// = (3116,3452), shape 10) blocks its own tile, so a walk can never end
/// there; `ladders.rs2` `[oploc1,ladder_cellar]` climbs from the player's
/// own stand.
const BRASS_HUT_INSIDE: WorldTile = WorldTile {
    x: 3116,
    z: 3451,
    level: 0,
};
/// Varrock West bank stand HillGiant walks after the hut exit.
const VARROCK_WEST_BANK: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};

fn drain_brass_hut_continues(name: &'static str) -> Step {
    Step {
        name,
        kind: StepKind::DrainDialogs { choice: 1 },
        wait: Wait {
            arm: Proof::NoActiveContinue,
            budget_ticks: 60,
        },
    }
}

/// Native Traveller follow through brass-hut door loc 1804 in both
/// directions: outside → hut interior (1804 N, into the dungeon hut),
/// then hut → Varrock West bank (1804 S, back out). Does not Start a
/// catalog card; hill_giant_bank's Strength-XP watch never reached this
/// door on headed 289 because the isolate did not run Eat/SetAttackStyle.
pub(crate) fn nav_brass_hut_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the backpack and give the Brass key",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give edgevilledungeonkey 1");
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: BRASS_KEY_ID,
                count: 1,
            },
            budget_ticks: 80,
        },
    });
    steps.push(drain_brass_hut_continues(
        "drain Inventory-wiped continue before the hut tele",
    ));
    steps.push(tele_step(
        "tele one tile south of the brass-key hut door",
        BRASS_HUT_APPROACH,
    ));
    steps.push(drain_brass_hut_continues(
        "drain speed/tele continues before the inbound follow",
    ));
    steps.push(Step {
        name: "follow north through loc 1804 into the hut",
        kind: StepKind::Follow {
            dest: BRASS_HUT_INSIDE,
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRASS_HUT_INSIDE.x,
                z: BRASS_HUT_INSIDE.z,
                level: BRASS_HUT_INSIDE.level,
                radius: 2,
            },
            budget_ticks: 600,
        },
    });
    steps.push(Step {
        name: "shot inside the hut after the 1804 north hop",
        kind: StepKind::Shot {
            label: "nav_brass_hut_in",
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRASS_HUT_INSIDE.x,
                z: BRASS_HUT_INSIDE.z,
                level: BRASS_HUT_INSIDE.level,
                radius: 2,
            },
            budget_ticks: 30,
        },
    });
    steps.push(Step {
        name: "follow south through loc 1804 out to Varrock West",
        kind: StepKind::Follow {
            dest: VARROCK_WEST_BANK,
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: VARROCK_WEST_BANK.x,
                z: VARROCK_WEST_BANK.z,
                level: VARROCK_WEST_BANK.level,
                radius: 4,
            },
            budget_ticks: 600,
        },
    });
    Scenario {
        name: "nav_brass_hut",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ArrivedNear {
            x: VARROCK_WEST_BANK.x,
            z: VARROCK_WEST_BANK.z,
            level: VARROCK_WEST_BANK.level,
            radius: 4,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            deadline: Duration::from_secs(420),
            terminal_shot: Some("nav_brass_hut"),
            ..Default::default()
        },
    }
}

/// Borrowed OD pairs: rs2b0t `script-routes.hardest.json` plus
/// `transport-heavy.routes.json` / boat table so we hit walk, stairs,
/// doors, Karamja fare, slashable web, and gnome glider. Teleports off.
pub(crate) const NAV_ROUTES: &[(&str, WorldTile, WorldTile)] = &[
    // HARD COMMUTE-14-R
    (
        "Seers bank → RockCrab field",
        WorldTile {
            x: 2725,
            z: 3491,
            level: 0,
        },
        WorldTile {
            x: 2710,
            z: 3720,
            level: 0,
        },
    ),
    // HARD WALK-5-4
    (
        "Taverley → Rellekka",
        WorldTile {
            x: 2895,
            z: 3435,
            level: 0,
        },
        WorldTile {
            x: 2668,
            z: 3660,
            level: 0,
        },
    ),
    // HARD WALK-3-10
    (
        "Ardougne → Yanille",
        WorldTile {
            x: 2661,
            z: 3301,
            level: 0,
        },
        WorldTile {
            x: 2612,
            z: 3092,
            level: 0,
        },
    ),
    // HARD BOT-ClueSolver-8-6 (stairs, upstairs houses)
    (
        "Falador house → Rimmington house",
        WorldTile {
            x: 3040,
            z: 3364,
            level: 1,
        },
        WorldTile {
            x: 2970,
            z: 3215,
            level: 1,
        },
    ),
    // Boat: Port Sarim seaman (npc 378 @ 3026,3217) 30-coin fare → Musa dock.
    (
        "Port Sarim → Musa Point",
        WorldTile {
            x: 3029,
            z: 3217,
            level: 0,
        },
        WorldTile {
            x: 2956,
            z: 3146,
            level: 0,
        },
    ),
    // Boat back: customs officer @ 2955,3146.
    (
        "Musa Point → Port Sarim",
        WorldTile {
            x: 2954,
            z: 3146,
            level: 0,
        },
        WorldTile {
            x: 3029,
            z: 3217,
            level: 0,
        },
    ),
    // rs2b0t two-route smoke: Yanille bank → chaos-druid field (web + stairs + ledge).
    (
        "Yanille bank → dungeon warriors",
        WorldTile {
            x: 2612,
            z: 3092,
            level: 0,
        },
        WorldTile {
            x: 2580,
            z: 9501,
            level: 0,
        },
    ),
    // TH-glider-gandius-hub (varp grandtree >= 160).
    (
        "Gandius glider → Grand Tree hub",
        WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        },
        WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
    ),
    // TH-glider-hub-karhewo.
    (
        "Grand Tree hub → Kar-Hewo",
        WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
        WorldTile {
            x: 3284,
            z: 3211,
            level: 0,
        },
    ),
    // Live door we just proved.
    (
        "Catherby range-house door",
        WorldTile {
            x: 2813,
            z: 3436,
            level: 0,
        },
        WorldTile {
            x: 2817,
            z: 3443,
            level: 0,
        },
    ),
];

/// Headed corpus smoke: mainland hop (unique live accounts spawn on
/// tutorial), `setvar` the transport-quest perm varps, clean logout +
/// login so the quest list and side icons refresh from the login payload
/// (no `~completequests` dialog pile), then the rs2b0t item kit (knife,
/// coins) and tele+Follow each borrowed OD pair. Proof is arrival at the
/// last dest. Teleports off. Auto-run is the host bothost feature.
/// The routes run with **Danger routing: Always**: Seers → Rock Crabs and
/// Taverley → Rellekka cross Wolf (L64, `huntmode=cowardly`, aggressive up
/// to combat 128) and Rock Crab (`check_nottoostrong=off`) zones, which the
/// default setting refuses at every combat level.
pub(crate) fn nav_routes_scenario() -> Scenario {
    let last = NAV_ROUTES[NAV_ROUTES.len() - 1].2;
    let mut steps = nav_kit_steps();
    for (note, from, to) in NAV_ROUTES {
        steps.push(tele_step(note, *from));
        steps.push(follow_step(note, *to));
    }
    Scenario {
        name: "nav_routes",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::Arrived {
            x: last.x,
            z: last.z,
            level: last.level,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: ScenarioNav {
                allow_danger_zones: true,
                ..nav_test_paints()
            },
            require_mainland_base: true,
            deadline: Duration::from_secs(3600),
            sustains: nav_energy_sustains(),
            ..Default::default()
        },
    }
}

/// The `nav_paint_path` scenario: log in `test`/`test`, mainland-hop into
/// the Lumbridge courtyard, and walk ~8 tiles south in one `Walk` step.
/// `nav` arms the panel's live overlay, so the headed runner shows
/// the red baked path clipped to the viewport, the cyan client trail, and
/// (with run on) the two-tone run-alt trail. No closer, no transport — the
/// whole point is a plain courtyard path the camera can hold.
pub(crate) fn nav_paint_path_scenario() -> Scenario {
    let dest = WorldTile {
        x: 3220,
        z: 3212,
        level: 0,
    };
    Scenario {
        name: "nav_paint_path",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![Step {
            name: "walk to courtyard south",
            kind: StepKind::Walk { dest },
            wait: Wait {
                arm: Proof::Arrived {
                    x: dest.x,
                    z: dest.z,
                    level: 0,
                },
                budget_ticks: 90,
            },
        }],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints(),
            require_mainland_base: true,
            terminal_shot: Some("nav_paint_path terminal"),
            ..Default::default()
        },
    }
}

/// Skills upstream `mage-bank-live.ts` raises to level 70 before the walk, in
/// its order.
const MAGE_BANK_SKILLS: [&str; 19] = [
    "attack",
    "strength",
    "defence",
    "ranged",
    "magic",
    "hitpoints",
    "prayer",
    "crafting",
    "mining",
    "smithing",
    "fishing",
    "cooking",
    "firemaking",
    "woodcutting",
    "runecraft",
    "herblore",
    "agility",
    "thieving",
    "fletching",
];
const MAGE_BANK_LEVEL: i32 = 70;
/// Outside Ardougne, a walk from the Ardougne lever (upstream `START`).
const MAGE_START: WorldTile = WorldTile {
    x: 2570,
    z: 3320,
    level: 0,
};
/// The cellar the ladder lands in; Gundai banks here (upstream `CELLAR`).
const MAGE_CELLAR: WorldTile = WorldTile {
    x: 2542,
    z: 4714,
    level: 0,
};

/// Wear an item the cell just gave, re-sending while the inventory view
/// lags the give. `Interactions::wear` refuses `StaleTarget` until
/// `snapshot.inventory()` lists the item, although `Proof::ItemId` (the flat
/// `inv()` list) already holds; combat cells avoid this by seeding the
/// weapon in the login fixture. Any other refusal is fatal.
fn wear_given_item_step(name: &'static str, id: i32) -> Step {
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(
                move |c, snapshot| match Interactions::new(snapshot, c).wear(id) {
                    SendResult::Sent { .. } => true,
                    SendResult::Refused { reason, .. } => reason == SendReason::StaleTarget,
                },
            ),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id },
            budget_ticks: 200,
        },
    }
}

/// `nav_mage_arena_bank`: from outside Ardougne, walk to Gundai's cellar
/// through the Ardougne lever, the Wilderness lever, the slashable webs and
/// the cellar ladder, then open the bank through Gundai's Talk-to.
// ported from rs2b0t e2e/mage-bank-live.ts @2de6bb23:
// - upstream's walkResilient policy (useTeleports, lever edges are teleport
//   kind) becomes the router's allow_teleports, so the lever is a route edge.
// - upstream's fail gates are `banked` and the Wilderness-lever sighting;
//   the sighting becomes a per-step `VisitedWithin` latch over observed dirty
//   snapshots (box = lever +/-8, as upstream), so a route that skips the lever fails.
// - upstream's Ardougne-lever, webs, ladder and cellar sightings are log-only
//   there; the cellar is gated here by ArrivedNear (radius 6, as upstream).
// - the Gundai choose string is the repo's exact option text (shop_buyout_lundail),
//   not upstream's prefix match.
// - useMageBank is a host-play script preference; this cell runs no bank script.
// - the 19 setstat cheats and the scimitar are the account's own seeded state;
//   the scimitar is worn because the webs need a worn slash weapon.
// - deadline 600s = upstream BUDGET_MS (walk, lever teleport, bank open).
// - no ::speed: engine stays at 600ms (ScenarioNav::engine_speed_ms None).
pub(crate) fn nav_mage_arena_bank_scenario() -> Scenario {
    Scenario {
        name: "nav_mage_arena_bank",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            // Upstream never verifies levels; magic is the arm because its
            // index is pinned (MAGIC_STAT), and the cheats run in upstream order.
            Step {
                name: "set the upstream skills to level 70",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        for skill in MAGE_BANK_SKILLS {
                            cheat(c, &format!("setstat {skill} {MAGE_BANK_LEVEL}"));
                        }
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat {
                        id: MAGIC_STAT,
                        min: MAGE_BANK_LEVEL,
                    },
                    budget_ticks: 60,
                },
            },
            // Drain the setstat level-up dialogs before the walk, as the combat
            // cells do; the helper's name comes from its first use there.
            drain_setstat_levelups_before_hostile_tele(),
            Step {
                name: "give the rune scimitar",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "give rune_scimitar 1");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ItemId {
                        id: RUNE_SCIMITAR_ID,
                        count: 1,
                    },
                    budget_ticks: 60,
                },
            },
            // mainlandAccount's tele and setvar do not relog, so the side icons
            // stay tutorial-locked; `Interactions::wear` reads snapshot.inventory(),
            // which needs side tab 3 (as the jewellery hop at L855-866 does).
            Step {
                name: "relog so the inventory tab binds before the wield",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            wear_given_item_step(
                "wield the rune scimitar (the webs need a worn slash weapon)",
                RUNE_SCIMITAR_ID,
            ),
            Step {
                name: "teleport outside Ardougne, where the walk begins",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, &tele_args(MAGE_START.level, MAGE_START.x, MAGE_START.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: MAGE_START.x,
                        z: MAGE_START.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            // The Wilderness lever box (3154,3924) with Chebyshev 8, as upstream.
            Step {
                name: "walk to the Mage Arena cellar through the levers, webs and ladder",
                kind: StepKind::Walk { dest: MAGE_CELLAR },
                wait: Wait {
                    arm: Proof::VisitedWithin {
                        min_x: 3146,
                        max_x: 3162,
                        min_z: 3916,
                        max_z: 3932,
                        level: 0,
                    },
                    budget_ticks: 800,
                },
            },
            Step {
                name: "the walk ended within the cellar arrival radius",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: MAGE_CELLAR.x,
                        z: MAGE_CELLAR.z,
                        level: 0,
                        radius: 6,
                    },
                    budget_ticks: 60,
                },
            },
            shop_buyout_open_npc_bank(
                "open Gundai's bank through Talk-to in the cellar",
                Proof::BankOpen,
                "Gundai",
                GUNDAI_BANK_APPROACH,
                "Talk-to",
                "Cool, I'd like to access my bank account please.",
            ),
        ],
        proof: Proof::BankOpen,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: ScenarioNav {
                allow_teleports: true,
                allow_wilderness: true,
                ..nav_test_paints()
            },
            deadline: Duration::from_secs(600),
            terminal_shot: Some("nav_mage_arena_bank terminal"),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod door_recovery_tests {
    use super::*;

    fn reply(sequence: i32, type_: i32, text: &str) -> ChatLineView {
        ChatLineView {
            type_,
            username: None,
            text: text.to_owned(),
            sequence,
        }
    }

    fn unexpected_query() -> bool {
        panic!("the drain query must only be sent on the first open read")
    }

    #[test]
    fn closer_cannot_act_on_the_walkers_first_open_read_alone() {
        let mut recovery = DoorRecovery::Ready;
        let stale = [reply(8, 0, "get runemysteries: 0")];
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &stale, || true));
        let bad_replies = [
            reply(11, 0, "get runemysteries: unknown"),
            reply(10, 2, "get runemysteries: 0"),
            reply(9, 0, "get tutorial: 1000"),
            reply(8, 0, "get runemysteries: 0"),
        ];
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &bad_replies, unexpected_query));
        let mut close_sent = false;
        recovery.closer_poll(Some(OPEN_ID), || {
            close_sent = true;
            true
        });
        assert!(!close_sent, "an earlier Open can still be in flight");
        assert_eq!(recovery, DoorRecovery::DrainingOpen { chat_after: 8 });
    }

    #[test]
    fn refused_drain_query_never_releases_closer() {
        let mut recovery = DoorRecovery::Ready;
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &[], || false));
        let later = [reply(9, 0, "get runemysteries: 0")];
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &later, unexpected_query));
        recovery.closer_poll(Some(OPEN_ID), || panic!("close after a refused query"));
        assert_eq!(recovery, DoorRecovery::DrainQueryRefused);
    }

    #[test]
    fn closer_sends_only_once_through_multiple_open_observations() {
        let mut recovery = DoorRecovery::CloseReady;
        let mut sends = 0;
        for leaf in [
            CLOSED_ID, OPEN_ID, OPEN_ID, CLOSED_ID, OPEN_ID, CLOSED_ID, OPEN_ID,
        ] {
            recovery.closer_poll(Some(leaf), || {
                sends += 1;
                true
            });
        }
        assert_eq!(sends, 1);
        assert_eq!(recovery, DoorRecovery::CloseConfirmed);
    }

    #[test]
    fn recovery_requires_a_confirmed_close_reopen_and_far_stand_in_order() {
        let mut recovery = DoorRecovery::Ready;
        recovery.closer_poll(Some(OPEN_ID), || panic!("close before walker Open"));
        assert!(recovery.walker_poll(Some(CLOSED_ID), Some(DOOR), &[], unexpected_query));
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &[], || true));
        let ack = [reply(1, 0, "get runemysteries: 0")];
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &ack, unexpected_query));
        assert_eq!(recovery, DoorRecovery::CloseReady);
        recovery.closer_poll(Some(OPEN_ID), || true);
        assert!(!recovery.walker_poll(Some(CLOSED_ID), Some(DOOR), &[], unexpected_query));
        recovery.closer_poll(Some(CLOSED_ID), || panic!("close on the closed leaf"));
        assert!(!recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &[], unexpected_query));
        assert!(recovery.walker_poll(Some(CLOSED_ID), Some(DOOR), &[], unexpected_query));
        assert!(recovery.walker_poll(Some(CLOSED_ID), Some(OPEN_DOOR), &[], unexpected_query));
        assert_eq!(recovery, DoorRecovery::Reopening);
        assert!(recovery.walker_poll(Some(OPEN_ID), Some(DOOR), &[], unexpected_query));
        assert_eq!(recovery, DoorRecovery::Reopened);
        assert!(recovery.walker_poll(Some(OPEN_ID), Some(OPEN_DOOR), &[], unexpected_query));
        assert_eq!(recovery, DoorRecovery::Crossed);
    }

    #[test]
    fn refused_close_does_not_retry_or_release_the_walker() {
        let mut recovery = DoorRecovery::CloseReady;
        recovery.closer_poll(Some(OPEN_ID), || false);
        recovery.closer_poll(Some(OPEN_ID), || panic!("second Close attempt"));
        recovery.closer_poll(Some(CLOSED_ID), || panic!("close on the closed leaf"));
        assert!(!recovery.walker_poll(Some(CLOSED_ID), Some(DOOR), &[], unexpected_query));
        assert_eq!(recovery, DoorRecovery::CloseRefused);
    }
}
