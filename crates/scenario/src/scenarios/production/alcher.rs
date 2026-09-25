use super::*;
const ALCHER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "items",
    value: ScriptInjectValue::StrList(&["rune_chainbody"]),
}];

const ALCHER_DEFAULTS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&[]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_CUSTOM_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["custom"]),
    },
    ScriptSettingInject {
        id: "customItem",
        value: ScriptInjectValue::Str("rune_chainbody"),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_CUSTOM_ALIAS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["custom"]),
    },
    ScriptSettingInject {
        id: "customItem",
        value: ScriptInjectValue::Str("adamant_scimitar"),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_CUSTOM_NAME_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["custom"]),
    },
    ScriptSettingInject {
        id: "customItem",
        value: ScriptInjectValue::Str("Adamant scimitar"),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_ORDERED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_platebody", "rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1.0),
    },
];

const ALCHER_LARGE_BATCH_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(1000.0),
    },
];

fn alcher_variant_scenario(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    fodder: &'static str,
    fodder_count: i32,
    runes: i32,
    required_stack: Option<(&'static str, i32)>,
) -> Scenario {
    let xp = Proof::StatXpGain { id: 6, min: 1 };
    let bank = VARROCK_WEST_BANK;
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed magic 55 and bounded Alcher stock before Start",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "setstat magic 55");
                        cheat(c, &format!("givebank {fodder} {fodder_count}"));
                        if name == "alcher_ordered" {
                            cheat(c, "givebank rune_chainbody 1");
                        }
                        cheat(c, &format!("givebank naturerune {runes}"));
                        cheat(c, "givebank staff_of_fire 1");
                        cheat(c, &tele_args(bank.level, bank.x, bank.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: bank.x,
                        z: bank.z,
                        level: bank.level,
                        radius: 6,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(Step {
                name: "confirm Magic 55 before Start",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 6, min: 55 },
                    budget_ticks: 200,
                },
            });
            steps.push(start_catalog_step());
            if let Some((item, count)) = required_stack {
                steps.push(Step {
                    name: "watch selected noted stack",
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: Wait {
                        arm: Proof::Item { name: item, count },
                        budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                    },
                });
            }
            steps.push(Step {
                name: "watch the Alcher option variant cast",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn alcher_custom_scenario() -> Scenario {
    alcher_variant_scenario(
        "alcher_custom",
        ALCHER_CUSTOM_INJECT,
        "rune_chainbody",
        2,
        4,
        Some(("Rune chainbody", 1)),
    )
}

pub(crate) fn alcher_ordered_scenario() -> Scenario {
    alcher_variant_scenario(
        "alcher_ordered",
        ALCHER_ORDERED_INJECT,
        "rune_platebody",
        1,
        4,
        Some(("Rune platebody", 1)),
    )
}

pub(crate) fn alcher_large_batch_scenario() -> Scenario {
    alcher_variant_scenario(
        "alcher_large_batch",
        ALCHER_LARGE_BATCH_INJECT,
        "rune_chainbody",
        1000,
        1000,
        Some(("Rune chainbody", 1000)),
    )
}

pub(crate) const RUNE_CHAINBODY_ID: i32 = 1113;
pub(crate) const CERT_RUNE_CHAINBODY_ID: i32 = 1114;
/// High Level Alchemy pays 60% of shop cost: floor(50000 * 0.6) = 30000.
pub(crate) const RUNE_CHAINBODY_HIGH_ALCH_COINS: i32 = 30_000;
/// Low Level Alchemy pays 40% of shop cost: floor(50000 * 0.4) = 20000.
pub(crate) const RUNE_CHAINBODY_LOW_ALCH_COINS: i32 = 20_000;
pub(crate) const LOW_ALCH_MAGIC_XP: i32 = 31;
pub(crate) const ATTACK_STAT: i32 = 0;

/// `spell=Low` on noted rune chainbodies, the `alcher-low-744-live` fixture:
/// Magic 25 at Varrock West with twelve chainbodies, 200 natures and a Staff
/// of fire banked. Nothing is worn and no outcome is seeded.
const ALCHER_LOW_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(10.0),
    },
    ScriptSettingInject {
        id: "spell",
        value: ScriptInjectValue::Str("Low"),
    },
];

/// The `alcher-fire-battlestaff-live` fixture: High stays the default (no
/// `spell` inject) and the only fire staff banked is a Fire battlestaff.
const ALCHER_FIRE_BATTLESTAFF_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(8.0),
    },
];

/// The `alcher_low` scenario: Magic 25 at Varrock West, twelve rune
/// chainbodies / 200 natures / one Staff of fire banked, `spell=Low`, ten alchs
/// a trip. The core witness proves the Low cast arithmetic and the worn staff.
pub(crate) fn alcher_low_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: LOW_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 25 and the exact Low fixture stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 25");
                cheat(c, "givebank rune_chainbody 12");
                cheat(c, "givebank naturerune 200");
                cheat(c, "givebank staff_of_fire 1");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Magic 25 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 25,
            },
        ),
        (
            "confirm no seeded noted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Nature rune outcome before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no staff in the pack before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact rune chainbody seed bank",
        Proof::BankItemId {
            id: RUNE_CHAINBODY_ID,
            count: 12,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 200,
            },
        ),
        (
            "acknowledge the exact Staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded note of the chainbody in bank",
            Proof::BankItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no Fire battlestaff in the Low bank",
            Proof::BankItemIdAtMost {
                id: FIRE_BATTLESTAFF_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch the noted rune chainbody land in the pack",
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
        ),
        (
            "watch the Staff of fire worn natively",
            Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
        ),
        ("watch Magic XP from a Low Level Alchemy cast", xp),
        (
            "watch the exact Low Level Alchemy coin payment",
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_LOW_ALCH_COINS,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "alcher_low",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_LOW_INJECT),
            terminal_shot: Some("alcher_low"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// The `alcher_fire_battlestaff` scenario: Magic 70 / Attack 40 at Varrock
/// West with eight chainbodies, 200 natures and exactly one Fire battlestaff
/// banked (no Staff of fire), default High, eight alchs a trip.
pub(crate) fn alcher_fire_battlestaff_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 70, Attack 40 and the exact alternative-staff stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 70");
                cheat(c, "setstat attack 40");
                cheat(c, "givebank fire_battlestaff 1");
                cheat(c, "givebank naturerune 200");
                cheat(c, "givebank rune_chainbody 8");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Magic 70 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 70,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: ATTACK_STAT,
                min: 40,
            },
        ),
        (
            "confirm no seeded Fire battlestaff in the pack before Start",
            Proof::ItemIdAtMost {
                id: FIRE_BATTLESTAFF_ID,
                count: 0,
            },
        ),
        (
            "confirm the default Staff of fire is absent from the pack before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Nature rune outcome before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact Fire battlestaff seed bank",
        Proof::BankItemId {
            id: FIRE_BATTLESTAFF_ID,
            count: 1,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 200,
            },
        ),
        (
            "acknowledge the exact rune chainbody seed bank",
            Proof::BankItemId {
                id: RUNE_CHAINBODY_ID,
                count: 8,
            },
        ),
        (
            "acknowledge Staff of fire is absent from the alternative-staff bank",
            Proof::BankItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded note of the chainbody in bank",
            Proof::BankItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch the Fire battlestaff worn natively",
            Proof::EquipmentId {
                id: FIRE_BATTLESTAFF_ID,
            },
        ),
        (
            "watch the noted rune chainbody land in the pack",
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
        ),
        ("watch Magic XP from a High Level Alchemy cast", xp),
        (
            "watch the exact High Level Alchemy coin payment",
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_HIGH_ALCH_COINS,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "alcher_fire_battlestaff",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_FIRE_BATTLESTAFF_INJECT),
            terminal_shot: Some("alcher_fire_battlestaff"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Frozen `alcher-swarm-drain-live`: Magic 70, 20 rich + 8 poor, High default,
/// 20 alchs a trip. After the first native High cast, inject `~macro_event 1`
/// until a Swarm NPC targets the local player. Busy-player `please_finish`
/// is explicit rejection, not spawn. CoreWatch owns interruption/recovery.
pub(crate) const SWARM_MACRO_EVENT_CHEAT: &str = "~macro_event 1";
pub(crate) const SWARM_NPC_NAME: &str = "Swarm";
pub(crate) const SWARM_BUSY_REJECT: &str = "Please finish what you are doing first.";

pub(crate) fn swarm_macro_event_accepted(snap: &GameSnapshot) -> bool {
    Proof::NpcNameTargetingLocal {
        name: SWARM_NPC_NAME,
    }
    .check(snap, None)
}

pub(crate) fn swarm_busy_reject_seq(snap: &GameSnapshot) -> Option<i32> {
    snap.chat_lines().iter().find_map(|line| {
        line.text
            .contains(SWARM_BUSY_REJECT)
            .then_some(line.sequence)
    })
}

/// First attempt after the firstcast arms; later attempts only when chat
/// shows a newer busy rejection than the one consumed by the last send.
pub(crate) fn swarm_macro_event_should_send(
    snap: &GameSnapshot,
    ever_sent: bool,
    last_handled_reject_seq: i32,
) -> bool {
    if swarm_macro_event_accepted(snap) {
        return false;
    }
    if !ever_sent {
        return true;
    }
    swarm_busy_reject_seq(snap).is_some_and(|seq| seq > last_handled_reject_seq)
}

fn swarm_macro_event_inject(
    c: &mut Client,
    snap: &GameSnapshot,
    ever_sent: &AtomicBool,
    last_handled_reject_seq: &AtomicI32,
) -> bool {
    if !swarm_macro_event_should_send(
        snap,
        ever_sent.load(Ordering::Relaxed),
        last_handled_reject_seq.load(Ordering::Relaxed),
    ) {
        return true;
    }
    if !cheat(c, SWARM_MACRO_EVENT_CHEAT) {
        return false;
    }
    ever_sent.store(true, Ordering::Relaxed);
    if let Some(seq) = swarm_busy_reject_seq(snap) {
        last_handled_reject_seq.store(seq, Ordering::Relaxed);
    }
    true
}

const ALCHER_SWARM_DRAIN_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "items",
        value: ScriptInjectValue::StrList(&["rune_chainbody", "yew_longbow"]),
    },
    ScriptSettingInject {
        id: "alchs",
        value: ScriptInjectValue::Num(20.0),
    },
];

pub(crate) fn alcher_swarm_drain_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 70 and the exact swarm-drain stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 70");
                cheat(c, "givebank rune_chainbody 20");
                cheat(c, "givebank yew_longbow 8");
                cheat(c, "givebank naturerune 200");
                cheat(c, "givebank staff_of_fire 1");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Magic 70 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: 70,
            },
        ),
        (
            "confirm no seeded noted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted rune chainbody before Start",
            Proof::ItemIdAtMost {
                id: RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted Yew longbow before Start",
            Proof::ItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unnoted Yew longbow before Start",
            Proof::ItemIdAtMost {
                id: YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Nature rune outcome before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no staff in the pack before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact rune chainbody seed bank",
        Proof::BankItemId {
            id: RUNE_CHAINBODY_ID,
            count: 20,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Yew longbow seed bank",
            Proof::BankItemId {
                id: YEW_LONGBOW_ID,
                count: 8,
            },
        ),
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 200,
            },
        ),
        (
            "acknowledge the exact Staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded note of the chainbody in bank",
            Proof::BankItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded note of the Yew longbow in bank",
            Proof::BankItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch the noted rune chainbody land in the pack",
            Proof::ItemId {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 1,
            },
        ),
        (
            "watch the Staff of fire worn natively",
            Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
        ),
        ("watch Magic XP from a High Level Alchemy cast", xp),
        (
            "watch the exact High Level Alchemy coin payment",
            Proof::ItemId {
                id: COINS_ID,
                count: RUNE_CHAINBODY_HIGH_ALCH_COINS,
            },
        ),
        (
            "watch the first High cast consume a noted rune chainbody",
            Proof::ItemIdAtMost {
                id: CERT_RUNE_CHAINBODY_ID,
                count: 19,
            },
        ),
        (
            "watch the first High cast consume a Nature rune",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 19,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let ever_sent = AtomicBool::new(false);
    let last_handled_reject_seq = AtomicI32::new(i32::MIN);
    steps.push(Step {
        name: "inject the upstream swarm macro_event after the first native High cast",
        kind: StepKind::Repeat {
            send: Box::new(move |c, snap| {
                swarm_macro_event_inject(c, snap, &ever_sent, &last_handled_reject_seq)
            }),
        },
        wait: Wait {
            arm: Proof::NpcNameTargetingLocal {
                name: SWARM_NPC_NAME,
            },
            budget_ticks: 50,
        },
    });
    Scenario {
        name: "alcher_swarm_drain",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(420),
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_SWARM_DRAIN_INJECT),
            terminal_shot: Some("alcher_swarm_drain"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) const ADAMANT_SCIMITAR_ID: i32 = 1331;
pub(crate) const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
pub(crate) const YEW_LONGBOW_ID: i32 = 855;
const CERT_YEW_LONGBOW_ID: i32 = 856;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
pub(crate) const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
/// High Level Alchemy pays 60% of shop cost: floor(1280 * 0.6) = 768.
const YEW_LONGBOW_ALCH_COINS: i32 = 768;
pub(crate) const HIGH_ALCH_MAGIC_XP: i32 = 65;

pub(crate) fn alcher_custom_alias_scenario() -> Scenario {
    alcher_generated_custom_scenario("alcher_custom_alias", ALCHER_CUSTOM_ALIAS_INJECT)
}

pub(crate) fn alcher_custom_name_scenario() -> Scenario {
    alcher_generated_custom_scenario("alcher_custom_name", ALCHER_CUSTOM_NAME_INJECT)
}

/// Empty `items` selects the frozen catalog's DEFAULT_ALCH_ITEMS. Only one
/// default target is banked, so the noted withdrawal proves fallback selection.
pub(crate) fn alcher_defaults_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: 6,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed only one default Alcher target at Varrock West before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 55");
                cheat(c, "givebank yew_longbow 1");
                cheat(c, "givebank naturerune 1");
                cheat(c, "givebank staff_of_fire 1");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Magic 55 before Start",
            Proof::Stat { id: 6, min: 55 },
        ),
        (
            "confirm no seeded unnoted Yew longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted Yew longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Nature rune outcome before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "confirm no Rune chainbody fallback confounder before Start",
            Proof::ItemAtMost {
                name: "Rune chainbody",
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the sole unnoted default target seed",
        Proof::BankItemId {
            id: YEW_LONGBOW_ID,
            count: 1,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded noted Yew longbow in bank",
            Proof::BankItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no Rune chainbody in bank",
            Proof::BankItemAtMost {
                name: "Rune chainbody",
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch default fallback withdraw the noted Yew longbow id",
            Proof::ItemId {
                id: CERT_YEW_LONGBOW_ID,
                count: 1,
            },
        ),
        ("watch Magic XP from the default High Alchemy cast", xp),
        (
            "watch the noted default target consumed",
            Proof::ItemIdAtMost {
                id: CERT_YEW_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "watch the Nature rune consumed",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "watch the exact default High Alchemy coin increase",
            Proof::ItemId {
                id: COINS_ID,
                count: YEW_LONGBOW_ALCH_COINS,
            },
        ),
        (
            "confirm Rune chainbody never entered the fallback path",
            Proof::ItemAtMost {
                name: "Rune chainbody",
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "alcher_defaults",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_DEFAULTS_INJECT),
            terminal_shot: Some("alcher_defaults"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Select the frozen Alcher Custom sentinel for a generated item that was
/// never in the handwritten ITEM_DB. The target is banked unnoted and must
/// be withdrawn as certificate id 1332 before the cast.
fn alcher_generated_custom_scenario(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: 6,
        min: HIGH_ALCH_MAGIC_XP,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic 55 and one banked generated custom target before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat magic 55");
                cheat(c, "givebank adamant_scimitar 1");
                cheat(c, "givebank naturerune 1");
                cheat(c, "givebank staff_of_fire 1");
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Magic 55 before Start",
            Proof::Stat { id: 6, min: 55 },
        ),
        (
            "confirm no seeded unnoted custom target before Start",
            Proof::ItemIdAtMost {
                id: ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted custom target before Start",
            Proof::ItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Nature rune outcome before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unnoted custom seed bank",
        Proof::BankItemId {
            id: ADAMANT_SCIMITAR_ID,
            count: 1,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact Nature rune seed bank",
            Proof::BankItemId {
                id: NATURE_RUNE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact staff of fire seed bank",
            Proof::BankItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded noted custom target in bank",
            Proof::BankItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch the generated custom item land as the noted id",
            Proof::ItemId {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 1,
            },
        ),
        ("watch Magic XP from one High Level Alchemy cast", xp),
        (
            "watch the noted custom target consumed",
            Proof::ItemIdAtMost {
                id: CERT_ADAMANT_SCIMITAR_ID,
                count: 0,
            },
        ),
        (
            "watch the Nature rune consumed",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "watch the exact High Alchemy coin increase",
            Proof::ItemId {
                id: COINS_ID,
                count: ADAMANT_SCIMITAR_ALCH_COINS,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// The `alcher` scenario: live Alcher gold — Varrock West, noted fodder +
/// natures + fire staff in bank, magic 55+. Proof is magic XP or coin gain
/// from alchs (magic XP delta here).
pub(crate) fn alcher_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 6, min: 1 };
    let bank = VARROCK_WEST_BANK;
    Scenario {
        name: "alcher",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed magic 55 before Start",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setstat magic 55");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat { id: 6, min: 55 },
                    budget_ticks: 200,
                },
            });
            steps.push(Step {
                name: "seed bank stock and tele to Varrock West",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "givebank rune_chainbody 30");
                        cheat(c, "givebank naturerune 200");
                        cheat(c, "givebank staff_of_fire 1");
                        cheat(c, &tele_args(bank.level, bank.x, bank.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ArrivedNear {
                        x: bank.x,
                        z: bank.z,
                        level: bank.level,
                        radius: 6,
                    },
                    budget_ticks: 200,
                },
            });
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script alch noted stock",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            steps
        },
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"),
            script_settings_inject: Some(ALCHER_INJECT),
            terminal_shot: Some("alcher terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Start off-scene in the dungeon that makes air-nearest choose Edgeville.
/// The measured native winner for this members, teleport-off route is Falador
/// East. Every action after Start belongs to Alcher; the cell only observes.
pub(crate) fn alcher_dwarven_mine_scenario() -> Scenario {
    let origin = WorldTile { x: 3016, z: 9840, level: 0 };
    let xp = Proof::FreshStatXpGain { id: MAGIC_STAT, min: HIGH_ALCH_MAGIC_XP };
    let watch = |name, arm| Step {
        name, kind: StepKind::Perform { send: Box::new(|_, _| true) },
        wait: Wait { arm, budget_ticks: 240 },
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed bank-only alchemy supplies and enter the Dwarven Mine before Start",
        kind: StepKind::Perform { send: Box::new(move |c, _| {
            cheat(c, "setstat magic 55");
            cheat(c, "givebank rune_chainbody 30");
            cheat(c, "givebank naturerune 200");
            cheat(c, "givebank staff_of_fire 1");
            cheat(c, &tele_args(origin.level, origin.x, origin.z));
            true
        }) },
        wait: Wait {
            arm: Proof::ArrivedNear { x: origin.x, z: origin.z, level: origin.level, radius: 0 },
            budget_ticks: 200,
        },
    });
    steps.push(watch("confirm Magic 55 before Start", Proof::Stat { id: MAGIC_STAT, min: 55 }));
    steps.push(start_catalog_step());
    steps.push(watch("watch Alcher leave the mine and reach the walk-cost winner",
        Proof::ArrivedNear { x: 3013, z: 3355, level: 0, radius: 4 }));
    steps.push(watch("watch Alcher open the stocked Falador East bank",
        Proof::BankItem { name: "Rune chainbody", count: 30 }));
    steps.push(watch("watch Alcher withdraw its bank-only fodder",
        Proof::Item { name: "Rune chainbody", count: 1 }));
    steps.push(watch("watch the bank stock decrease through real withdrawal",
        Proof::BankItemAtMost { name: "Rune chainbody", count: 29 }));
    steps.push(watch("watch fresh High Alchemy after the dungeon bank trip", xp));
    Scenario {
        name: "alcher_dwarven_mine",
        seed: Seed { profiles: vec![("test", "test")], mainland: true },
        steps, proof: xp, companions: vec![],
        settings: ScenarioSettings {
            full_rate: true, require_mainland_base: true, deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Alcher"), script_settings_inject: Some(ALCHER_INJECT),
            terminal_shot: Some("alcher_dwarven_mine"), nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
