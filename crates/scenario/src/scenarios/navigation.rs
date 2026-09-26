use super::pair::{follow_step, nav_kit_steps, tele_step};
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
/// drives the route one step per tick until arrival. The destination is a
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
            nav: nav_test_paints().with_tick_ms(300),
            ..Default::default()
        },
    }
}

/// The `nav_door` scenario: the two-bot door slam as a scenario fleet.
/// Profile 0 (`test`) is the driven walker: after the mainland seed it
/// cheat-teles to the Catherby range-house `OUTSIDE` stand, then `Follow`s
/// a whole-world route through the door to `DEST` inside. Profile 1
/// (`test2`) is the closer companion: it cheat-teles inside, then
/// `op_loc`s the door shut on every player-info tick the instant the door
/// reads open, so the walker's follow is stressed against a tick-perfect
/// closer. The follow's terminal outcome at the door is the diagnostic
/// target (the closer slams the door shut as the walker approaches, which
/// surfaces as `Refused Unreachable` in the traveller's settle). The
/// companion's gating mirrors the old harness: the first `scene_state ==
/// 2` frame is skipped (host-play queues `mainland_hop` after the hook),
/// and the Catherby tele waits until `here` is the mainland courtyard.
pub(crate) fn nav_door_scenario() -> Scenario {
    let outside = WorldTile {
        x: OUTSIDE.x,
        z: OUTSIDE.z,
        level: 0,
    };
    let dest = WorldTile {
        x: DEST.x,
        z: DEST.z,
        level: 0,
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
                    send: Box::new(|c, _| cheat(c, WALKER_TELE)),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: outside.x,
                        z: outside.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow through the range-house door",
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
        companions: vec![Companion {
            profile: 1,
            per_frame: {
                let mut slot = CloserSlot::default();
                Box::new(move |c| closer_frame(c, &mut slot))
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
            nav: nav_test_paints().with_tick_ms(300),
            ..Default::default()
        },
    }
}

/// Closed Catherby range-house door (loc 1530) the closer slams.
const DOOR: WorldTile = WorldTile {
    x: 2816,
    z: 3438,
    level: 0,
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
const CLOSER_TELE: &str = "tele 0,44,53,1,47";

/// Per-frame state of the closer companion (profile 1), owned by the
/// companion's closure. All plain fields, so the closure is `Send`.
#[derive(Default)]
struct CloserSlot {
    /// First `scene_state == 2` was observed; that frame host-play still
    /// queues `mainland_hop` after `per_frame`, so the Catherby tele
    /// waits a tick.
    scene2_seen: bool,
    tele_sent: bool,
    last_gen: u64,
}

/// The closer companion's per-frame hook: gate the Catherby tele behind
/// the mainland hop, then on every player-info tick `op_loc` the live
/// open leaf (id 1531, often a tile off packed `at`) so it slams shut.
fn closer_frame(c: &mut Client, s: &mut CloserSlot) {
    let Some(lp) = &c.local_player else {
        if debug_enabled() {
            eprintln!("[nav-closer] no local_player scene={}", c.scene_state);
        }
        return;
    };
    let here = WorldTile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    if stage_closer_tele(c, here, s) {
        return;
    }
    if c.gens.player == s.last_gen {
        return;
    }
    s.last_gen = c.gens.player;
    let loc = wall_loc(c, DOOR);
    if debug_enabled() {
        eprintln!(
            "[nav-closer] here={here:?} loc={loc:?} tele_sent={} scene={}",
            s.tele_sent, c.scene_state
        );
    }
    let Some((loc_tile, loc_id)) = loc else {
        return;
    };
    if loc_id == CLOSED_ID {
        return;
    }
    // OP_LOC1 on the live open leaf is Close. The leaf often sits a tile
    // off packed `at`; slamming packed `at` looks up an empty typecode.
    if debug_enabled() {
        eprintln!(
            "[nav-closer] SLAM op_loc id={loc_id} at ({},{}) packed=({},{})",
            loc_tile.x, loc_tile.z, DOOR.x, DOOR.z
        );
    }
    op_loc(c, loc_tile.x, loc_tile.z, loc_id);
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

fn wall_loc(c: &Client, tile: WorldTile) -> Option<(WorldTile, i32)> {
    let (bx, bz) = c.build_base();
    nearest_door_loc(tile, |x, z| {
        c.loc_typecode(x - bx, z - bz).map(|tc| (tc >> 14) & 0x7fff)
    })
}

/// Nearest 1530/1531 within chebyshev 3 of packed `at`. Returns the live
/// tile — the Catherby open leaf sits a tile off that origin, and
/// `op_loc` must click the leaf, not packed `at`.
pub(crate) fn nearest_door_loc(
    packed: WorldTile,
    lookup: impl Fn(i32, i32) -> Option<i32>,
) -> Option<(WorldTile, i32)> {
    let mut best: Option<(i32, WorldTile, i32)> = None;
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            let gap = dx.abs().max(dz.abs());
            if gap > 3 {
                continue;
            }
            let x = packed.x + dx;
            let z = packed.z + dz;
            let Some(id) = lookup(x, z) else {
                continue;
            };
            if id != CLOSED_ID && id != OPEN_ID {
                continue;
            }
            if best.map(|(g, _, _)| gap < g).unwrap_or(true) {
                best = Some((
                    gap,
                    WorldTile {
                        x,
                        z,
                        level: packed.level,
                    },
                    id,
                ));
            }
        }
    }
    best.map(|(_, tile, id)| (tile, id))
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

/// The `nav_cart` scenario: the first OP_NPC execute follow. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, cheat-give
/// the fare (the packed Shilo→Brimhaven Npc edge carries a 200-coin
/// `item_req`), cheat-tele to the Shilo Village cart driver, then
/// `Follow` a whole-world route to the Brimhaven cart landing — a
/// destination that **requires** the cart hop (the only way across the
/// sea to the Brimhaven side). The traveller interacts the driver
/// (`OpTarget::Npc` + option 1), answers the driver's fare dialog (the
/// "Yes please…" choice), and PASSes on `TravelOutcome::Arrived` at the
/// landing.
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
            nav: nav_test_paints().with_tick_ms(300),
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
/// The Rune Essence mine pad (m45_75 local (32,33)): the packed entry
/// edge's landing anchor (the real landing is randomised in the mine).
const MINE_PAD: WorldTile = WorldTile {
    x: 2912,
    z: 4833,
    level: 0,
};

/// The `nav_essence` scenario: the EssenceSession execute twin. Log in
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, then seed
/// Rune Mysteries by painting the quest journal green: the
/// `~completequests` debugproc completes every quest and calls
/// `~update_questlist` (a bare `setvar runemysteries 6` leaves the
/// client's journal colours stale, so `WorldState::from_snapshot` would
/// still gate the packed entry edges closed), but it first opens two
/// `p_choice` dialogs — a `ChatAnswer` janitor answers them as they
/// appear and the step waits on the journal going green. Then cheat-tele
/// to Aubury's shop, `Follow` into the mine (the entry hop latches the
/// session on any mine landing), and `Follow` back out through the exit
/// portal — the session-gated return may only land near Aubury, never
/// another wizard. PASSes when the player stands within the exit landing
/// radius of Aubury's anchor.
pub(crate) fn nav_essence_scenario() -> Scenario {
    Scenario {
        name: "nav_essence",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "complete Rune Mysteries",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~completequests");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ChatChoice,
                    budget_ticks: 20,
                },
            },
            Step {
                name: "answer the quest-seed dialogs until the journal is green",
                kind: StepKind::DrainDialogs { choice: 1 },
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
                            &tele_args(AUBURY_ANCHOR.level, AUBURY_ANCHOR.x, AUBURY_ANCHOR.z),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: AUBURY_ANCHOR.x,
                        z: AUBURY_ANCHOR.z,
                        level: AUBURY_ANCHOR.level,
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
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_essence terminal"),
            ..Default::default()
        },
    }
}

/// The maze-side Elkoy (npc 473, m39_49 local (8,55)): the `at` of the
/// packed maze→village escort edge, one tile south of the entrance coord
/// (2504,3192).
const ELKOY_MAZE_SIDE: WorldTile = WorldTile {
    x: 2504,
    z: 3191,
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
/// `test`/`test`, mainland-hop into the Lumbridge courtyard, then seed
/// Tree Gnome Village by painting the quest journal green: the
/// `~completequests` debugproc completes every quest and calls
/// `~update_questlist` (a bare `setvar treequest …` leaves the client's
/// journal colours stale, so `WorldState::from_snapshot` would still gate
/// the packed escort edges closed), but it first opens two `p_choice`
/// dialogs — a `ChatAnswer` janitor answers them as they appear and the
/// step waits on the journal going green. Then cheat-tele to the maze-side
/// Elkoy, and `Follow` a whole-world route into the village — across the
/// hedge maze, whose 1-tick escort hop beats the maze walk in the router,
/// so the route executes the packed `TransportKind::Npc` edge. The
/// traveller talks to Elkoy
/// (`OpTarget::Npc` + option 1), answers the escort dialog's "Yes please."
/// choice (the chat modal's first), and PASSes on `TravelOutcome::Arrived`
/// at the packed `edge.to` (2515,3159).
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
                name: "complete Tree Gnome Village",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~completequests");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ChatChoice,
                    budget_ticks: 20,
                },
            },
            Step {
                name: "answer the quest-seed dialogs until the journal is green",
                kind: StepKind::DrainDialogs { choice: 1 },
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
            nav: nav_test_paints().with_tick_ms(300),
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
/// dueling ring (the packed jewellery rub edge's `item_req`),
/// then `Follow` with `allow_teleports` on to the Al Kharid Duel Arena —
/// a destination the 2-tick rub edge always beats (the walk is toll-
/// gated and over a hundred ticks, so the packed leg only ever routes
/// when the layer is on and the item is held). The traveller rubs the
/// held ring (`OpTarget::Item` + option 4 — never the WalkTo `::tele`
/// cheat), answers the destination choice the rub opens, and PASSes on
/// `TravelOutcome::Arrived` within the packed landing's scatter radius.
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
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(360),
            terminal_shot: Some("nav_tele terminal"),
            ..Default::default()
        },
    }
}

/// The Shantay henge desert stand (m51_48 local (38,38) = (3302,3110),
/// south of the gate on the desert side `coordz <= loc z`).
const SHANTAY_DESERT_START: WorldTile = WorldTile {
    x: 3302,
    z: 3110,
    level: 0,
};
/// The Al Kharid-side stand (m51_48 local (40,47) = (3304,3119), north
/// of the gate).
const SHANTAY_PASS_START: WorldTile = WorldTile {
    x: 3304,
    z: 3119,
    level: 0,
};
/// The Al Kharid-side follow dest (m51_48 local (36,48) = (3300,3120)).
const SHANTAY_PASS_DEST: WorldTile = WorldTile {
    x: 3300,
    z: 3120,
    level: 0,
};
/// The desert follow dest (m51_48 local (39,36) = (3303,3108)).
const SHANTAY_DESERT_DEST: WorldTile = WorldTile {
    x: 3303,
    z: 3108,
    level: 0,
};

/// The `nav_shantay` scenario: both directions through the Shantay henge
/// (loc 4031, `shantay_pass.rs2` `oploc1`), driven by `Traveller::follow`
/// like every nav twin. The desert → pass leg follows with an empty
/// inventory — the free desert exit edge (`coordz(coord) <=
/// coordz(loc_coord)` telejump, no `item_req`) is the only Shantay edge
/// the fail-closed WorldState relaxes. The pass → desert leg clears the
/// backpack (`[debugproc,clearinv]` — the shared `test` slot may carry
/// junk from prior live twins), `give`s one Shantay pass, and follows
/// through the gated hop (consume pass + `[queue,shantay_pass_enter]`),
/// the only edge into the desert. PASS is standing on the desert dest.
pub(crate) fn nav_shantay_scenario() -> Scenario {
    Scenario {
        name: "nav_shantay",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "clear the backpack and tele to the desert stand",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "~clearinv");
                        cheat(c, "tele 0,51,48,38,38");
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
                name: "follow the free desert exit to Al Kharid",
                kind: StepKind::Follow {
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
                // The arm waits for the pass to actually land: the
                // WorldState of the follow step then proves the packed
                // gated edge's `item_req`, or the router fails closed.
                wait: Wait {
                    arm: Proof::Item {
                        name: "Shantay pass",
                        count: 1,
                    },
                    budget_ticks: 60,
                },
            },
            Step {
                name: "tele to the Al Kharid stand",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| cheat(c, "tele 0,51,48,40,47")),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_PASS_START.x,
                        z: SHANTAY_PASS_START.z,
                        level: 0,
                    },
                    budget_ticks: 120,
                },
            },
            Step {
                name: "follow the pass-gated hop into the desert",
                kind: StepKind::Follow {
                    dest: SHANTAY_DESERT_DEST,
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: SHANTAY_DESERT_DEST.x,
                        z: SHANTAY_DESERT_DEST.z,
                        level: 0,
                    },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Arrived {
            x: SHANTAY_DESERT_DEST.x,
            z: SHANTAY_DESERT_DEST.z,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            nav: nav_test_paints().with_tick_ms(300),
            deadline: Duration::from_secs(420),
            terminal_shot: Some("nav_shantay terminal"),
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
            nav: nav_test_paints().with_tick_ms(300),
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
            nav: nav_test_paints().with_tick_ms(300),
            require_mainland_base: true,
            terminal_shot: Some("nav_paint_path terminal"),
            ..Default::default()
        },
    }
}
