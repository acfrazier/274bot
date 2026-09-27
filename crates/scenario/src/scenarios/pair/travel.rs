use super::*;
/// Journal colour is login-time `~update_questlist`; the Relog step after
/// these cheats is what actually opens packed quest-gated edges.
const TRANSPORT_QUEST_SETVARS: &[&str] = &[
    "setvar runemysteries 6",
    "setvar grandtree 160",
    "setvar treequest 9",
    "setvar zombiequeen 15",
    "setvar priestperil 60",
    "setvar elenaquest 30",
    "setvar itwatchtower 14",
    "setvar eadgar_quest 110",
    "setvar waterfall_quest 10",
    "setvar dragonquest 10",
    "setvar desertrescue 30",
];

/// Live nav kit. Mainland hop, stick `tutorial=1000`, quest `setvar`s,
/// clean Relog (side tab 3 bound), then rs2b0t `seedTeleKit`: knife, coins,
/// runes, charged jewellery. Auto-run is host bothost.
pub(crate) fn nav_kit_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip",
            kind: StepKind::Repeat {
                send: Box::new(|c, _| {
                    cheat(c, "setvar tutorial 1000");
                    cheat(c, "getvar tutorial");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Chat {
                    needle: "get tutorial: 1000",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "setvar transport quests",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    for cmd in TRANSPORT_QUEST_SETVARS {
                        cheat(c, cmd);
                    }
                    true
                }),
            },
            wait: Wait {
                // Already on the mainland courtyard after the seed hop;
                // this only sequences the cheats before Relog.
                arm: Proof::Arrived {
                    x: 3220,
                    z: 3220,
                    level: 0,
                },
                budget_ticks: 30,
            },
        },
        Step {
            name: "relog so journal, side icons, and tutorial lock refresh",
            kind: StepKind::Relog,
            wait: Wait {
                // rs2b0t mainlandAccount: sideIcon[3] bound after relog.
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
        Step {
            name: "journal shows Grand Tree complete",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::QuestDone {
                    name: "The Grand Tree",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "maxme setstat 99 (debug heading, not ~maxme)",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    for cmd in MAXME_SETSTATS {
                        cheat(c, cmd);
                    }
                    true
                }),
            },
            wait: Wait {
                // Attack is skill 0; Stat id 16 is run energy, not agility.
                arm: Proof::Stat { id: 0, min: 99 },
                budget_ticks: 80,
            },
        },
        seed_give("seed knife", "knife", 1, "Knife", 1),
        seed_give("seed coins for fares", "coins", 5000, "Coins", 5000),
        // Lean tele kit: one Varrock-shaped hop (nav-tele-smoke), not
        // RUNE_SEEDS 80/200/80/80/80. Jewellery is one charged copy each.
        seed_give("seed law runes", "lawrune", 10, "Law rune", 1),
        seed_give("seed air runes", "airrune", 30, "Air rune", 1),
        seed_give("seed fire runes", "firerune", 10, "Fire rune", 1),
        seed_give(
            "seed dueling ring",
            "ring_of_dueling_8",
            1,
            "Ring of dueling(8)",
            1,
        ),
        seed_give(
            "seed glory",
            "amulet_of_glory_4",
            1,
            "Amulet of glory(4)",
            1,
        ),
        seed_give(
            "seed games necklace",
            "necklace_of_minigames_8",
            1,
            "Games necklace(8)",
            1,
        ),
    ]
}

/// One `give <debug> <qty>`, then wait until that display name is in the
/// pack (rs2b0t `seedItem`: give once, poll presence — not re-give every
/// tick).
fn seed_give(
    step: &'static str,
    debug: &'static str,
    qty: i32,
    display: &'static str,
    want: i32,
) -> Step {
    Step {
        name: step,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("give {debug} {qty}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Item {
                name: display,
                count: want,
            },
            budget_ticks: 80,
        },
    }
}

pub(crate) fn tele_step(note: &'static str, tile: WorldTile) -> Step {
    Step {
        name: note,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| cheat(c, &tele_args(tile.level, tile.x, tile.z))),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            },
            budget_ticks: 120,
        },
    }
}

pub(crate) fn follow_step(note: &'static str, dest: WorldTile) -> Step {
    let arm = if glider_landing(dest) {
        // `map_findsquare($dest, 0, 1, lineofwalk)` — live Kar-Hewo
        // landed (3285,3211) for packed (3284,3211).
        Proof::ArrivedNear {
            x: dest.x,
            z: dest.z,
            level: dest.level,
            radius: 1,
        }
    } else {
        Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        }
    };
    Step {
        name: note,
        kind: StepKind::Follow { dest },
        wait: Wait {
            arm,
            budget_ticks: 600,
        },
    }
}

/// Packed Gnome Air pads + hub (`gnome_glider.rs2` / `glider.constant`).
fn glider_landing(tile: WorldTile) -> bool {
    matches!(
        (tile.x, tile.z, tile.level),
        (2465, 3501, 3) | (2971, 2969, 0) | (2850, 3497, 0) | (3320, 3430, 0) | (3284, 3211, 0)
    )
}
