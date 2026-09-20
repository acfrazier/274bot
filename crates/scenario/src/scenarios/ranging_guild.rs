//! Bounded RangingGuild gold: on-stand round/repeat and seeded-ticket redeem.
//!
//! Facts are pinned to frozen `e2e/rangingguild-live.ts`, `RangingGuildLogic.ts`,
//! selected-289 `obj`/`interface`/`varp.pack`, and `competition_judge.rs2` /
//! `ticket_merchant.rs2`. This is not a full Seers bank/restock cycle.

use crate::*;

/// Selected 289 `obj.pack` / `289.json`.
pub(crate) const COINS_ID: i32 = 995;
pub(crate) const MAGIC_SHORTBOW_ID: i32 = 861;
pub(crate) const ARCHERY_TICKET_ID: i32 = 1464;
pub(crate) const RUNE_ARROW_ID: i32 = 892;
/// Snapshot skill table: Ranged. Distinct from [`Proof::Stat`] energy id 16.
pub(crate) const RANGED_STAT: i32 = 4;
/// Packed `varp.pack`: `156=targetcount`.
pub(crate) const VARP_TARGET_COUNT: i32 = 156;

/// Frozen live `--phase round` stand and Logic `STAND`.
pub(crate) const RANGING_GUILD_STAND: WorldTile = WorldTile {
    x: 2672,
    z: 3419,
    level: 0,
};
/// Frozen live merchant seat and Logic `MERCHANT_STAND`. Script `walkTo` r3.
pub(crate) const RANGING_GUILD_MERCHANT_STAND: WorldTile = WorldTile {
    x: 2659,
    z: 3430,
    level: 0,
};

/// `RangingGuildLogic` / `competition_judge.rs2` entry fee.
pub(crate) const ENTRY_FEE: i32 = 200;
/// Live full `coinsPerTrip` and two-round funding (`2 * ENTRY_FEE`).
pub(crate) const COINS_FOR_TWO_ROUNDS: i32 = 400;
/// Frozen live `RANGED = 70` (door 40, Magic shortbow 50).
pub(crate) const RANGED_LIVE: i32 = 70;
/// Logic / `ticket_merchant.rs2` `tickets_shop:com_90`.
pub(crate) const TICKETS_PER_TRADE: i32 = 2000;
pub(crate) const RUNE_ARROWS_PER_TRADE: i32 = 50;

/// `SHOT_MS`(6000)×10×2 + `CHAT_MS`(20000)×5 enter/collect/second-enter.
/// Not the 15-minute live-full harness budget (bank walk is out of scope).
pub(crate) const RANGING_GUILD_ROUND_DEADLINE: Duration = Duration::from_secs(300);
const RANGING_GUILD_ROUND_WATCH_TICKS: u32 = 300;

const ROUND_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "coinsPerTrip",
    value: ScriptInjectValue::Num(400.0),
}];

fn watch(name: &'static str, arm: Proof, budget_ticks: u32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait { arm, budget_ticks },
    }
}

/// Smallest native round then a further enter. Tickets are earned, not seeded.
pub(crate) fn ranging_guild_round_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Ranged 70, Magic shortbow and 400 coins, then tele to the range stand",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, &format!("advancestat ranged {RANGED_LIVE}"));
                cheat(c, "give magic_shortbow");
                cheat(c, &format!("give coins {COINS_FOR_TWO_ROUNDS}"));
                cheat(
                    c,
                    &tele_args(
                        RANGING_GUILD_STAND.level,
                        RANGING_GUILD_STAND.x,
                        RANGING_GUILD_STAND.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: RANGING_GUILD_STAND.x,
                z: RANGING_GUILD_STAND.z,
                level: RANGING_GUILD_STAND.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm Ranged 70 before Start",
            Proof::Stat {
                id: RANGED_STAT,
                min: RANGED_LIVE,
            },
        ),
        (
            "confirm the Magic shortbow before Start",
            Proof::ItemId {
                id: MAGIC_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "confirm 400 coins for two 200-coin enters before Start",
            Proof::ItemId {
                id: COINS_ID,
                count: COINS_FOR_TWO_ROUNDS,
            },
        ),
        (
            "confirm no seeded archery tickets before Start",
            Proof::ItemIdAtMost {
                id: ARCHERY_TICKET_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded rune arrows before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ARROW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(watch(name, arm, 60));
    }
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch the first 200-coin judge fee",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: ENTRY_FEE,
            },
        ),
        (
            "watch the judge start a real targetcount round",
            Proof::Varp {
                id: VARP_TARGET_COUNT,
                min: 1,
            },
        ),
        (
            "watch a shot increment targetcount past the paid 1",
            Proof::Varp {
                id: VARP_TARGET_COUNT,
                min: 2,
            },
        ),
        (
            "watch the judge reset targetcount on payout",
            Proof::VarpExact {
                id: VARP_TARGET_COUNT,
                value: 0,
            },
        ),
        (
            "watch earned archery tickets after the payout reset",
            Proof::ItemId {
                id: ARCHERY_TICKET_ID,
                count: 1,
            },
        ),
        (
            "watch the second 200-coin judge fee",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch the further round start",
            Proof::Varp {
                id: VARP_TARGET_COUNT,
                min: 1,
            },
        ),
    ] {
        steps.push(watch(name, arm, RANGING_GUILD_ROUND_WATCH_TICKS));
    }
    Scenario {
        name: "ranging_guild_round",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::Varp {
            id: VARP_TARGET_COUNT,
            min: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: RANGING_GUILD_ROUND_DEADLINE,
            start_script: Some("RangingGuild"),
            script_settings_inject: Some(ROUND_INJECT),
            terminal_shot: Some("ranging_guild_round"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Separate redeem: 2000 tickets are explicitly seeded, not an earned-round proof.
pub(crate) fn ranging_guild_redeem_scenario() -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed a Magic shortbow and seeded archery tickets, then tele to the ticket merchant",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, &format!("advancestat ranged {RANGED_LIVE}"));
                cheat(c, "give magic_shortbow");
                cheat(c, &format!("give archery_ticket {TICKETS_PER_TRADE}"));
                cheat(
                    c,
                    &tele_args(
                        RANGING_GUILD_MERCHANT_STAND.level,
                        RANGING_GUILD_MERCHANT_STAND.x,
                        RANGING_GUILD_MERCHANT_STAND.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: RANGING_GUILD_MERCHANT_STAND.x,
                z: RANGING_GUILD_MERCHANT_STAND.z,
                level: RANGING_GUILD_MERCHANT_STAND.level,
                radius: 3,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm Ranged 70 before Start",
            Proof::Stat {
                id: RANGED_STAT,
                min: RANGED_LIVE,
            },
        ),
        (
            "confirm the Magic shortbow before Start",
            Proof::ItemId {
                id: MAGIC_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "confirm 2000 seeded archery tickets before Start",
            Proof::ItemId {
                id: ARCHERY_TICKET_ID,
                count: TICKETS_PER_TRADE,
            },
        ),
        (
            "confirm no seeded rune arrows before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ARROW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(watch(name, arm, 60));
    }
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch the seeded tickets leave the pack",
            Proof::ItemIdAtMost {
                id: ARCHERY_TICKET_ID,
                count: 0,
            },
        ),
        (
            "watch the ticket shop hand over 50 rune arrows",
            Proof::ItemId {
                id: RUNE_ARROW_ID,
                count: RUNE_ARROWS_PER_TRADE,
            },
        ),
    ] {
        steps.push(watch(name, arm, SCRIPT_GOLD_WATCH_TICKS));
    }
    Scenario {
        name: "ranging_guild_redeem",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: RUNE_ARROW_ID,
            count: RUNE_ARROWS_PER_TRADE,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("RangingGuild"),
            terminal_shot: Some("ranging_guild_redeem"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
