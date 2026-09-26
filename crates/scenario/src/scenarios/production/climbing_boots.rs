use super::*;
/// ClimbingBoots option cells. `useTeleport` and `runeStock` are typed on
/// purpose: the frozen script's defaults are true/50, and the walking cell
/// must be the explicit false branch, the teleport cell the explicit true
/// branch with the smallest non-zero rune stock.
const CLIMBING_BOOTS_WALK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "useTeleport",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "runeStock",
        value: ScriptInjectValue::Num(1.0),
    },
];
const CLIMBING_BOOTS_TELEPORT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "useTeleport",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "runeStock",
        value: ScriptInjectValue::Num(1.0),
    },
];

pub(crate) const CLIMBING_BOOTS_ID: i32 = 3105;
const CLIMBING_BOOTS_PAIR_COINS: i32 = 12;
const CLIMBING_BOOTS_TELE_MAGIC: i32 = 37;
/// `readyToBuy` wants coins === 12 * tripQty with no unrelated inventory.
/// tripQty is 28 minus the carried rune stacks, so the walk cell carries
/// 28 pairs and the teleport cell 25 pairs beside Law 1/Air 3/Water 1.
pub(crate) const CLIMBING_BOOTS_WALK_PACK_COINS: i32 = 28 * CLIMBING_BOOTS_PAIR_COINS;
pub(crate) const CLIMBING_BOOTS_TELE_PACK_COINS: i32 = 25 * CLIMBING_BOOTS_PAIR_COINS;
/// Bank stock for the later withdrawals the full cycle claims: two further
/// trips of coins, and — teleport cell only — the exact Law 1/Air 3/Water 1
/// restock, because `runeStock=1` consumes the whole carried stack per cast.
pub(crate) const CLIMBING_BOOTS_BANK_TRIPS: i32 = 2;
const CLIMBING_BOOTS_RUNES: &[(&str, i32)] = &[("lawrune", 1), ("airrune", 3), ("waterrune", 1)];
/// The rest-of-trip arm after the 2-pair spend watch: the walk cell's
/// return to Falador West and the teleport cell's Falador landing.
///
/// **Units:** runner dirty-snapshot increments (`budget_ticks`), not engine
/// ticks or wall seconds. `readyToBuy` fixes one trip at 28 (walk) / 25
/// (teleport) pairs, and the frozen card returns only once `tripComplete`
/// (`ClimbingBoots.ts` loop: `returnToBank` runs only when the pack is
/// complete). Each pair is seven `death_sherpa.rs2` pause pages (two
/// `~chatplayer`, two `~chatnpc`, the `~objbox`, `~p_choice2`, the
/// `~p_choice5` reprompt), and frozen `driveShop` waits for each page to
/// change, then `delayTicks(1)` per continue and `delayTicks(2)` per choice:
/// at least 16 engine ticks (9.6s) a pair. Live 290253afd measured ~11s a
/// pair at ~2.4 dirty increments/s, so the ordinary 150 (~62s) covered
/// seven pairs of the 28.
/// Walk: 26 pairs (~290s) plus the ~260-tile hut → Falador West walk
/// (≤156s at walking pace) ≈ 450s ≈ 1080 dirties → **1200**.
/// Teleport: 23 pairs (~255s) plus the cast ≈ 615 dirties → **750**. In the
/// teleport cell that arm is the cast's Magic XP (see the watch list); the
/// landing follows it within a few ticks.
pub(crate) const CLIMBING_BOOTS_WALK_RETURN_WATCH_TICKS: u32 = 1200;
pub(crate) const CLIMBING_BOOTS_TELE_CAST_WATCH_TICKS: u32 = 750;
/// The further-pair arm (`has_item_id(3105)>=2` after the restock left zero
/// carried): the walk back to the hut (≤156s), the 3745 entry, the talk and
/// two pairs (~30s) ≈ 190s ≈ 460 dirties → **600**.
pub(crate) const CLIMBING_BOOTS_FURTHER_WATCH_TICKS: u32 = 600;
/// Whole-scenario wall: seed (~50s) + first pairs (~30s) + the arms above at
/// their wall estimates + bank open/deposit/restock (~25s). Walk ≈ 745s
/// with ~20% margin. Teleport measured live at bce85f2df: Start 15s,
/// 25th pair 286s, landing 292s, restock closed 306s, hut re-entered 404s,
/// further pair (last watch) 427s → 720s keeps ~65% margin. The Magic XP
/// proof shares the cast arm's baseline, so it holds at the last watch; a
/// baseline taken after the cycle instead needs a second full trip (that
/// run's 25th second-trip pair only landed at 715s).
pub(crate) const CLIMBING_BOOTS_WALK_DEADLINE: Duration = Duration::from_secs(900);
pub(crate) const CLIMBING_BOOTS_TELE_DEADLINE: Duration = Duration::from_secs(720);
/// The Water rune id for the bank-seed acknowledgement (Law 563 and Air 556
/// already have crate constants).
pub(crate) const WATER_RUNE_ID: i32 = 555;
/// Tenzing's hut: the door tile the pack stands on and the inside tile the
/// frozen script targets. Route/prep targets only, never PASS predicates.
pub(crate) const TENZING_DOOR: WorldTile = WorldTile {
    x: 2823,
    z: 3555,
    level: 0,
};
pub(crate) const TENZING_INSIDE: WorldTile = WorldTile {
    x: 2820,
    z: 3556,
    level: 0,
};
pub(crate) const TENZING_NAME: &str = "Tenzing";
pub(crate) fn climbing_boots_scenario() -> Scenario {
    climbing_boots_variant(
        "climbing_boots",
        CLIMBING_BOOTS_WALK_INJECT,
        CLIMBING_BOOTS_WALK_PACK_COINS,
        false,
    )
}

pub(crate) fn climbing_boots_teleport_scenario() -> Scenario {
    climbing_boots_variant(
        "climbing_boots_teleport",
        CLIMBING_BOOTS_TELEPORT_INJECT,
        CLIMBING_BOOTS_TELE_PACK_COINS,
        true,
    )
}

/// Clean pack at Tenzing's hut: Death Plateau completed through the authentic
/// primary `setvar death_equiproom 80` **and** retained map progress
/// `setvar death_map 8`, each with its own native `getvar` Chat receipt, a
/// relog so the journal repaints, then the exact carried trip money (plus the
/// rune stack for the teleport cell) and a bank stock for later withdrawals.
///
/// Authentic completion keeps map progress: the commander path reaches
/// `denulth_has_map` only when primary is 70 and `death_get_map >= 8`
/// (`death_scouted_area`); the completion queue then sets primary 80 and
/// leaves `death_map` unchanged. Tenzing door gates read bits 0..3 of that
/// same varp. Seeding primary alone leaves map 0 and is not a completed state.
///
/// Both `getvar` readbacks (`get death_equiproom: 80` and `get death_map: 8`),
/// the `Death Plateau` journal row and the framed `Tenzing` NPC are the
/// fixture's fail-closed prerequisite: they are exact server Chat replies to
/// the cheat path, not a client varp snapshot (default published 315 stays 0
/// and does not prove transmission). A pack that does not provide them times
/// this cell out before Start instead of seeding a shortcut. There is no
/// revision switch in this runner and none is invented here; the guard is the
/// authenticated content those steps resolve.
///
/// The pack itself carries zero boots. The bank stock is a real booth session
/// (the ordinary window, the inventory's bulk deposit op, a real close) — not
/// `givebank` and not a bank-side cheat — and it is not a purchase claim.
/// The open seed session also acknowledges `BankItemIdAtMost` boots 3105 = 0
/// before close, so leftover banked boots cannot hide behind a closed-bank
/// Start snapshot. Start is the real frozen script; nothing intervenes after
/// it, and the purchase, return, deposit and further stages are watched
/// separately. No boots, extra cash, reward XP, or unrelated progress is
/// granted in preparation.
fn climbing_boots_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    pack_coins: i32,
    use_teleport: bool,
) -> Scenario {
    let door = Proof::ArrivedNear {
        x: TENZING_DOOR.x,
        z: TENZING_DOOR.z,
        level: TENZING_DOOR.level,
        radius: 4,
    };
    let tenzing = Proof::NpcNameNear {
        name: TENZING_NAME,
        x: TENZING_INSIDE.x,
        z: TENZING_INSIDE.z,
        level: TENZING_INSIDE.level,
        radius: 12,
    };
    let bank = FALADOR_WEST_BANK;
    let returned = Proof::ArrivedNear {
        x: bank.x,
        z: bank.z,
        level: bank.level,
        radius: 8,
    };
    let mut steps: Vec<Step> = Vec::new();
    // Authentic completed Death Plateau: primary 80 + retained map progress 8
    // (bits 0..3). Separate setvar/getvar + Chat receipts so each value is
    // proven by the server reply before relog/Start — not a client snapshot.
    steps.push(Step {
        name: "complete Death Plateau primary by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar death_equiproom 80");
                cheat(c, "getvar death_equiproom");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "get death_equiproom: 80",
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "retain Death Plateau map progress by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar death_map 8");
                cheat(c, "getvar death_map");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "get death_map: 8",
            },
            budget_ticks: 200,
        },
    });
    steps.extend(script_live_seed_steps());
    steps.push(bank_fletcher_watch(
        "acknowledge Death Plateau complete before Start",
        Proof::QuestDone {
            name: "Death Plateau",
        },
    ));
    let booth = FALADOR_WEST_BOOTH;
    let bank_coins = CLIMBING_BOOTS_BANK_TRIPS * pack_coins;
    steps.push(Step {
        name: "seed the bank-bound stack and stand at the Falador West booth",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give coins {bank_coins}"));
                if use_teleport {
                    for (alias, count) in CLIMBING_BOOTS_RUNES {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                cheat(c, &tele_args(booth.level, booth.x, booth.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: booth.x,
                z: booth.z,
                level: booth.level,
                radius: 4,
            },
            budget_ticks: 200,
        },
    });
    // The bank seed is a real session, not a bank-side cheat: the ordinary
    // booth window, the inventory's bulk deposit op, and a real close. The
    // deposited rows are the same stackables the script later withdraws.
    steps.push(open_seed_booth(
        "open the real Falador West bank for the seed deposit",
        booth,
        Proof::BankItemId {
            id: COINS_ID,
            count: 0,
        },
    ));
    steps.push(Step {
        name: "deposit the seeded coins and runes through the bank window",
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                // Repeat sends before checking its arm. Once the deposit has
                // landed the bank-side pack is empty; acknowledge the actual
                // fresh-bank readback instead of refusing an unnecessary send.
                if (Proof::BankItemId {
                    id: COINS_ID,
                    count: bank_coins,
                })
                .check(snapshot, None)
                {
                    return true;
                }
                let mut ix = Interactions::new(snapshot, c);
                let mut wrote = false;
                for item in snapshot.bank_side() {
                    if let Some(op) = bank_deposit_all_op(&item.actions) {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                            SendResult::Sent { .. }
                        );
                    }
                }
                wrote
            }),
        },
        wait: Wait {
            arm: Proof::BankItemId {
                id: COINS_ID,
                count: bank_coins,
            },
            budget_ticks: 200,
        },
    });
    if use_teleport {
        // `runeStock=1` spends the whole carried stack on the first cast, so
        // the seed bank itself must be acknowledged to hold the exact restock
        // the full cycle withdraws (`CLIMBING_BOOTS_RUNES`) before the session
        // closes and Start inherits it. Coins alone would let a rune-less bank
        // seed pass.
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Law rune",
            Proof::BankItemId {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Air runes",
            Proof::BankItemId {
                id: AIR_RUNE_ID,
                count: 3,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Water rune",
            Proof::BankItemId {
                id: WATER_RUNE_ID,
                count: 1,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge the seed bank holds no leftover climbing boots",
        Proof::BankItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "seed the exact trip stack and stand at Tenzing's hut",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                if use_teleport {
                    cheat(c, &format!("setstat magic {CLIMBING_BOOTS_TELE_MAGIC}"));
                    for (alias, count) in CLIMBING_BOOTS_RUNES {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                cheat(c, &format!("give coins {pack_coins}"));
                cheat(
                    c,
                    &tele_args(TENZING_DOOR.level, TENZING_DOOR.x, TENZING_DOOR.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: door,
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm no seeded boots in the pack before Start",
        Proof::ItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm the real Tenzing NPC is framed at the hut before Start",
        tenzing,
    ));
    steps.push(start_catalog_step());
    // Serial arms in cycle order: purchase, spend, return (the teleport cell
    // lands the real Falador cast first, then walks to the bank stand),
    // deposit, restock close, further pair. The cast lands only after a full
    // trip, so its arm follows the purchase arms in cycle order and carries
    // the rest-of-trip budget; a 150-dirty cast arm could never see it.
    let trip_watch = if use_teleport {
        CLIMBING_BOOTS_TELE_CAST_WATCH_TICKS
    } else {
        CLIMBING_BOOTS_WALK_RETURN_WATCH_TICKS
    };
    let mut watches: Vec<(&'static str, Proof, u32)> = vec![
        (
            "watch the real purchase gain a pair of boots",
            Proof::ItemId {
                id: CLIMBING_BOOTS_ID,
                count: 1,
            },
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch the purchase spend 12 coins a pair",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: pack_coins - CLIMBING_BOOTS_PAIR_COINS * 2,
            },
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch the return to the Falador West bank",
            returned,
            if use_teleport {
                SCRIPT_GOLD_WATCH_TICKS
            } else {
                trip_watch
            },
        ),
        (
            "watch Falador West bank hold the deposited boots",
            Proof::BankItemId {
                id: CLIMBING_BOOTS_ID,
                count: 1,
            },
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch the bank close after restocking for a further trip",
            Proof::BankClosed,
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch a further pair for the full cycle",
            Proof::ItemId {
                id: CLIMBING_BOOTS_ID,
                count: 2,
            },
            CLIMBING_BOOTS_FURTHER_WATCH_TICKS,
        ),
    ];
    if use_teleport {
        // The runner takes a `StatXpGain` baseline when the first arm of
        // that shape begins and the proof reuses it. Without an arm here
        // the baseline is taken when proving starts, after the further
        // pair, so the proof would wait for a second full trip and cast.
        // `magic_teleport` deletes the runes and grants the XP one tick
        // before `player_teleport_normal` jumps, so the XP arm (baseline
        // after the 2-pair spend, before any cast) comes first, then the
        // landing.
        watches.insert(
            2,
            (
                "watch the real Falador cast gain Magic XP",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
                trip_watch,
            ),
        );
        watches.insert(
            3,
            (
                "watch the real Falador cast land at the bank",
                Proof::ArrivedNear {
                    x: FALADOR_TELE_LAND.x,
                    z: FALADOR_TELE_LAND.z,
                    level: FALADOR_TELE_LAND.level,
                    radius: 8,
                },
                SCRIPT_GOLD_WATCH_TICKS,
            ),
        );
    }
    for (step_name, arm, budget_ticks) in watches {
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    let proof = if use_teleport {
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 1,
        }
    } else {
        Proof::ItemId {
            id: CLIMBING_BOOTS_ID,
            count: 2,
        }
    };
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: if use_teleport {
                CLIMBING_BOOTS_TELE_DEADLINE
            } else {
                CLIMBING_BOOTS_WALK_DEADLINE
            },
            start_script: Some("ClimbingBoots"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
