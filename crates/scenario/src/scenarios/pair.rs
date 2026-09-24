mod travel;
pub(super) use travel::{follow_step, nav_kit_steps, tele_step};
use super::production::*;
use crate::*;

/// Lumbridge courtyard stand where the two-bot trade meets.
const TRADE_COURTYARD: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};

/// Per-frame state for the trade-accept companion (profile 1).
#[derive(Default)]
struct TradeAcceptSlot {
    scene2_seen: bool,
    tele_sent: bool,
}

/// The `script_trade` scenario: a two-bot fleet — both profiles Start the
/// in-tree `TradeBot` fixture (Compat `Trade.request` / `offerAll` /
/// `accept`) with reciprocal `partner` inject; profile 1 also rust-teles
/// beside the driven bot after the mainland hop and presses Accept on
/// both trade screens. Proof: the driven slot holds zero Coins after
/// offering the seeded stack of twenty-five.
pub(crate) fn script_trade_scenario() -> Scenario {
    let courtyard = TRADE_COURTYARD;
    Scenario {
        name: "script_trade",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "stick tutorial skip and seed twenty-five coins",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give coins 25");
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
                name: "relog so the inv tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to Lumbridge courtyard",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(courtyard.level, courtyard.x, courtyard.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: courtyard.x,
                        z: courtyard.z,
                        level: courtyard.level,
                    },
                    budget_ticks: 120,
                },
            },
            start_catalog_step(),
            Step {
                name: "watch the trade consume coins",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ItemAtMost {
                        name: "Coins",
                        count: 0,
                    },
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            },
        ],
        proof: Proof::ItemAtMost {
            name: "Coins",
            count: 0,
        },
        companions: vec![Companion {
            profile: 1,
            per_frame: {
                let mut slot = TradeAcceptSlot::default();
                Box::new(move |c| trade_acceptor_frame(c, &mut slot))
            },
        }],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("TradeBot"),
            inject_companion_as: Some("partner"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn trade_acceptor_frame(c: &mut Client, s: &mut TradeAcceptSlot) {
    let Some(lp) = &c.local_player else {
        if debug_enabled() {
            eprintln!("[trade-companion] no local_player scene={}", c.scene_state);
        }
        return;
    };
    let here = WorldTile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    if stage_trade_companion_tele(c, here, s) {
        return;
    }
    press_trade_accept(c);
}

/// Host-play queues `mainland_hop` after the first `scene_state == 2`
/// companion frame; cheat-tele beside the driven bot once ingame on a
/// mainland build base (the runner seed gate), not only when `here.x > 3100`.
fn stage_trade_companion_tele(c: &mut Client, here: WorldTile, s: &mut TradeAcceptSlot) -> bool {
    if at_trade_courtyard(here) || s.tele_sent {
        return false;
    }
    if c.scene_state != 2 {
        return false;
    }
    if !s.scene2_seen {
        s.scene2_seen = true;
        return false;
    }
    if c.map_build_base_x < 3000 {
        if debug_enabled() {
            eprintln!(
                "[trade-companion] waiting mainland base, here={here:?} base_x={}",
                c.map_build_base_x
            );
        }
        return false;
    }
    if debug_enabled() {
        eprintln!(
            "[trade-companion] tele {} from {here:?}",
            tele_args(TRADE_COURTYARD.level, TRADE_COURTYARD.x, TRADE_COURTYARD.z)
        );
    }
    cheat(
        c,
        &tele_args(TRADE_COURTYARD.level, TRADE_COURTYARD.x, TRADE_COURTYARD.z),
    );
    s.tele_sent = true;
    true
}

fn at_trade_courtyard(here: WorldTile) -> bool {
    here.x == TRADE_COURTYARD.x
        && here.z == TRADE_COURTYARD.z
        && here.level == TRADE_COURTYARD.level
}

/// Fail-closed when the trade screen is open but the posted accept
/// component id is absent (companion cannot press Accept).
pub(crate) fn trade_accept_missing_block(trade: &api::snapshot::TradeView) -> Option<&'static str> {
    if (trade.offer_open || trade.confirm_open) && trade.accept_component_id < 0 {
        Some("BLOCKED: missing trade accept com")
    } else {
        None
    }
}

fn press_trade_accept(c: &mut Client) {
    let mut snap = GameSnapshot::default();
    snap.rebuild(c);
    let trade = snap.trade();
    if let Some(msg) = trade_accept_missing_block(trade) {
        fail(msg);
    }
    if !trade.offer_open && !trade.confirm_open {
        return;
    }
    let mut ix = Interactions::new(&snap, c);
    let ctx = ReadContext::new(&snap);
    let Some(widget) = ctx.component(trade.accept_component_id) else {
        return;
    };
    match ix.press(widget) {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
    }
}

#[derive(Clone, Copy)]
enum PairCompanionKind {
    AirRunner,
    MuleMule,
    FlaxSpinner,
    DuelPeer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PairPrepStage {
    WaitMainland,
    TutSkip,
    Relog,
    WaitRelog,
    Seed,
    WaitSeed,
    AckBank,
    CloseBank,
    FirstLoad,
    WaitReady,
    Wear,
    WaitWear,
    Idle,
}

struct PairCompanionSlot {
    kind: PairCompanionKind,
    stage: PairPrepStage,
    logout_sent: bool,
    saw_logout: bool,
    logout_session: u64,
    seed_sent: bool,
    first_load_sent: bool,
    last_action: Instant,
}

impl PairCompanionSlot {
    fn new(kind: PairCompanionKind) -> Self {
        Self {
            kind,
            stage: PairPrepStage::WaitMainland,
            logout_sent: false,
            saw_logout: false,
            logout_session: 0,
            seed_sent: false,
            first_load_sent: false,
            last_action: Instant::now() - Duration::from_secs(1),
        }
    }
}

fn pair_companion_frame(c: &mut Client, slot: &mut PairCompanionSlot) {
    let mut snap = GameSnapshot::default();
    snap.rebuild(c);
    let now = Instant::now();
    let inv_tab = snap
        .side_tabs()
        .iter()
        .any(|tab| tab.index == 3 && tab.available);
    match slot.stage {
        PairPrepStage::WaitMainland => {
            if c.ingame && c.scene_state == 2 && c.map_build_base_x >= 3000 {
                slot.stage = PairPrepStage::TutSkip;
            }
        }
        PairPrepStage::TutSkip => {
            if !send_ok(slot, now) {
                return;
            }
            cheat(c, "setvar tutorial 1000");
            cheat(c, "getvar tutorial");
            slot.last_action = now;
            slot.stage = PairPrepStage::Relog;
        }
        PairPrepStage::Relog => {
            if c.ingame && !slot.logout_sent {
                if !send_ok(slot, now) {
                    return;
                }
                let ifaces = std::sync::Arc::clone(&c.ifaces);
                if logout(c, ifaces.as_slice()) {
                    slot.logout_sent = true;
                    slot.logout_session = c.gens.session;
                    slot.last_action = now;
                    slot.stage = PairPrepStage::WaitRelog;
                }
            }
        }
        PairPrepStage::WaitRelog => {
            // Host::run_client probe returns on !ingame before the next
            // observe, so companion_tick never sees the off-world frame.
            // A successful login/reconnect bumps c.gens.session; that
            // change since intentional logout is the delivered signal.
            // last_login_reconnect may already be true from an earlier
            // grant and must not admit stale pre-logout scene2.
            if pair_relog_seen(
                slot.logout_sent,
                slot.saw_logout,
                snap.ingame(),
                slot.logout_session,
                c.gens.session,
            ) {
                slot.saw_logout = true;
                if !snap.ingame() {
                    return;
                }
            }
            if slot.saw_logout && snap.ingame() && snap.scene_state() == 2 && inv_tab {
                slot.stage = PairPrepStage::Seed;
            }
        }
        PairPrepStage::Seed => {
            if !send_ok(slot, now) {
                return;
            }
            if !slot.seed_sent {
                cheat(c, "~clearinv");
                match slot.kind {
                    PairCompanionKind::AirRunner => {
                        cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                        cheat(
                            c,
                            &tele_args(
                                FALADOR_EAST_BANK.level,
                                FALADOR_EAST_BANK.x,
                                FALADOR_EAST_BANK.z,
                            ),
                        );
                    }
                    PairCompanionKind::MuleMule => {
                        cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                        cheat(
                            c,
                            &tele_args(
                                FALADOR_EAST_BANK.level,
                                FALADOR_EAST_BANK.x,
                                FALADOR_EAST_BANK.z,
                            ),
                        );
                    }
                    PairCompanionKind::FlaxSpinner => {
                        cheat(c, "setstat crafting 10");
                        cheat(c, &tele_args(FLAX_MEET.level, FLAX_MEET.x, FLAX_MEET.z));
                    }
                    PairCompanionKind::DuelPeer => {
                        cheat(c, "give bronze_scimitar 1");
                        cheat(
                            c,
                            &tele_args(DUEL_CHALLENGE.level, DUEL_CHALLENGE.x, DUEL_CHALLENGE.z),
                        );
                    }
                }
                slot.seed_sent = true;
                slot.last_action = now;
            }
            slot.stage = PairPrepStage::WaitSeed;
        }
        PairPrepStage::WaitSeed => match slot.kind {
            PairCompanionKind::AirRunner | PairCompanionKind::MuleMule => {
                if pair_near(&snap, FALADOR_EAST_BANK, 8) {
                    slot.stage = PairPrepStage::AckBank;
                }
            }
            PairCompanionKind::FlaxSpinner => {
                if pair_near(&snap, FLAX_MEET, 8)
                    && pair_stat(&snap, CRAFTING_STAT) >= 10
                    && pair_inv_id(&snap, FLAX_ID) == 0
                    && pair_inv_id(&snap, BOW_STRING_ID) == 0
                {
                    slot.stage = PairPrepStage::Idle;
                }
            }
            PairCompanionKind::DuelPeer => {
                if pair_near(&snap, DUEL_CHALLENGE, 8) && pair_inv_any(&snap) {
                    slot.stage = PairPrepStage::Wear;
                }
            }
        },
        PairPrepStage::AckBank => {
            if pair_bank_id(&snap, RUNE_ESSENCE_ID) >= RUNE_ESSENCE_SEED
                && snap.bank_loaded()
                && snap.bank_component_id() >= 0
                && pair_near(&snap, FALADOR_EAST_BANK, 8)
            {
                slot.stage = PairPrepStage::CloseBank;
                return;
            }
            if !send_ok(slot, now) {
                return;
            }
            match Interactions::new(&snap, c).open_nearest_booth() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
        PairPrepStage::CloseBank => {
            if snap.bank_component_id() < 0 {
                slot.stage = PairPrepStage::FirstLoad;
                return;
            }
            if !send_ok(slot, now) {
                return;
            }
            match Interactions::new(&snap, c).close_modal() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
        PairPrepStage::FirstLoad => {
            if !send_ok(slot, now) {
                return;
            }
            if !slot.first_load_sent {
                let n = match slot.kind {
                    PairCompanionKind::AirRunner => PAIR_AIR_FIRST_LOAD,
                    PairCompanionKind::MuleMule => PAIR_MULE_FIRST_LOAD,
                    PairCompanionKind::FlaxSpinner | PairCompanionKind::DuelPeer => 0,
                };
                cheat(c, &format!("give blankrune {n}"));
                cheat(
                    c,
                    &tele_args(
                        MULECRAFTER_AIR_RUINS.level,
                        MULECRAFTER_AIR_RUINS.x,
                        MULECRAFTER_AIR_RUINS.z,
                    ),
                );
                slot.first_load_sent = true;
                slot.last_action = now;
            }
            slot.stage = PairPrepStage::WaitReady;
        }
        PairPrepStage::WaitReady => {
            let want = match slot.kind {
                PairCompanionKind::AirRunner => PAIR_AIR_FIRST_LOAD,
                PairCompanionKind::MuleMule => PAIR_MULE_FIRST_LOAD,
                PairCompanionKind::FlaxSpinner | PairCompanionKind::DuelPeer => 0,
            };
            if pair_near(&snap, MULECRAFTER_AIR_RUINS, 8)
                && pair_inv_id(&snap, RUNE_ESSENCE_ID) >= want
                && snap.bank_component_id() < 0
            {
                slot.stage = PairPrepStage::Idle;
            }
        }
        PairPrepStage::Wear => {
            if !send_ok(slot, now) {
                return;
            }
            pair_wear_first_inv(c, &snap);
            slot.last_action = now;
            slot.stage = PairPrepStage::WaitWear;
        }
        PairPrepStage::WaitWear => {
            if pair_near(&snap, DUEL_CHALLENGE, 8)
                && pair_weapon_equipped(&snap)
                && snap.modals().main < 0
            {
                slot.stage = PairPrepStage::Idle;
            } else if send_ok(slot, now) {
                pair_wear_first_inv(c, &snap);
                slot.last_action = now;
            }
        }
        PairPrepStage::Idle => {}
    }
}

fn send_ok(slot: &PairCompanionSlot, now: Instant) -> bool {
    now.duration_since(slot.last_action) >= Duration::from_millis(400)
}

fn pair_relog_seen(
    logout_sent: bool,
    saw_logout: bool,
    ingame: bool,
    logout_session: u64,
    session: u64,
) -> bool {
    saw_logout || !ingame || (logout_sent && session != logout_session)
}

fn pair_near(snap: &GameSnapshot, dest: WorldTile, radius: i32) -> bool {
    snap.tile().is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs() <= radius && (z - dest.z).abs() <= radius
    })
}

fn pair_inv_id(snap: &GameSnapshot, id: i32) -> i32 {
    snap.inv()
        .iter()
        .filter(|(item_id, _)| *item_id == id)
        .map(|(_, count)| *count)
        .sum()
}

fn pair_inv_any(snap: &GameSnapshot) -> bool {
    snap.inv().iter().any(|(_, count)| *count > 0)
}

fn pair_weapon_equipped(snap: &GameSnapshot) -> bool {
    snap.equipment()
        .iter()
        .any(|item| item.count > 0 && item.def.id > 0)
}

fn pair_wear_first_inv(c: &mut Client, snap: &GameSnapshot) {
    let Some((id, _)) = snap.inv().iter().copied().find(|(_, count)| *count > 0) else {
        return;
    };
    match Interactions::new(snap, c).wear(id) {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
    }
}

fn pair_bank_id(snap: &GameSnapshot, id: i32) -> i32 {
    snap.bank()
        .iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}

fn pair_stat(snap: &GameSnapshot, id: i32) -> i32 {
    snap.stats()
        .iter()
        .find(|row| row.index == id)
        .map(|row| row.base)
        .unwrap_or(0)
}

fn pair_fleet_seed() -> Seed {
    Seed {
        profiles: vec![("test", "test"), ("test2", "test2")],
        mainland: true,
    }
}

fn pair_watch_settings(name: &'static str, start_script: &'static str) -> ScenarioSettings {
    ScenarioSettings {
        full_rate: true,
        only_render_selected: false,
        require_mainland_base: true,
        deadline: SCRIPT_GOLD_DEADLINE,
        start_script: Some(start_script),
        terminal_shot: Some(name),
        nav: gold_script_nav(),
        ..Default::default()
    }
}

fn pair_companion(kind: PairCompanionKind) -> Companion {
    Companion {
        profile: 1,
        per_frame: {
            let mut slot = PairCompanionSlot::new(kind);
            Box::new(move |c| pair_companion_frame(c, &mut slot))
        },
    }
}

/// NatureCrafter Air Master/Runner: two visible slots, shared Start after
/// both native preps. Pair watch supplies complementary mode/partner bags.
pub(crate) fn nature_crafter_air_scenario() -> Scenario {
    let ruins = MULECRAFTER_AIR_RUINS;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Air master talisman at ruins before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give air_talisman 1");
                cheat(c, &tele_args(ruins.level, ruins.x, ruins.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ruins.x,
                z: ruins.z,
                level: ruins.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Air talisman in pack before Start",
            Proof::ItemId {
                id: AIR_TALISMAN_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded essence in pack before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Air runes in pack before Start",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "nature_crafter_air",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::AirRunner)],
        settings: pair_watch_settings("nature_crafter_air", "NatureCrafter"),
    }
}

/// MuleCrafter Air Crafter/Mule: two visible slots, shared Start after both
/// native preps. bankFill=true is the pair_settings default, not this cell.
pub(crate) fn mule_crafter_air_scenario() -> Scenario {
    let ruins = MULECRAFTER_AIR_RUINS;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Mule crafter's exact noted essence bank before Start",
        FALADOR_EAST_BANK,
        vec![NativeSeed {
            unnoted_id: RUNE_ESSENCE_ID,
            debug_alias: "blankrune",
            note_alias: Some("cert_blankrune"),
            quantity: RUNE_ESSENCE_SEED,
            note_id: Some(NOTED_ESSENCE_ID),
        }],
        "runecraft",
        1,
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the Mule crafter essence bank",
        Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the Mule crafter noted essence through the bank window",
        vec![NativeSeed {
            unnoted_id: RUNE_ESSENCE_ID,
            debug_alias: "blankrune",
            note_alias: Some("cert_blankrune"),
            quantity: RUNE_ESSENCE_SEED,
            note_id: Some(NOTED_ESSENCE_ID),
        }],
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "seed Mule crafter talisman and first 27 essence at ruins before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give air_talisman 1");
                cheat(c, &format!("give blankrune {PAIR_MULE_FIRST_LOAD}"));
                cheat(c, &tele_args(ruins.level, ruins.x, ruins.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ruins.x,
                z: ruins.z,
                level: ruins.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Air talisman in pack before Start",
            Proof::ItemId {
                id: AIR_TALISMAN_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one unnoted 1436 load of 27 before Start",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: PAIR_MULE_FIRST_LOAD,
            },
        ),
        (
            "confirm no extra unnoted essence beyond the first load",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: PAIR_MULE_FIRST_LOAD,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Air runes in pack before Start",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "mule_crafter_air",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::MuleMule)],
        settings: pair_watch_settings("mule_crafter_air", "MuleCrafter"),
    }
}

/// FlaxRunner Runner/Spinner: empty packs, runner at the field, spinner at
/// the meet. First flax pack is picked after shared Start.
pub(crate) fn flax_runner_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed empty Flax runner pack at the field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "flax_runner",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::FlaxSpinner)],
        settings: pair_watch_settings("flax_runner", "FlaxRunner"),
    }
}

/// DuelArena both actors: bronze scimitar at the challenge anchor, wear it,
/// then shared Start. Counterpart identity is native witness-owned; the
/// frozen script has target stats and no partner setting.
pub(crate) fn duel_arena_scenario() -> Scenario {
    let dest = DUEL_CHALLENGE;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Duel bronze scimitar at the challenge anchor before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give bronze_scimitar 1");
                cheat(c, &tele_args(dest.level, dest.x, dest.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm bronze scimitar in pack before Start",
        Proof::Item {
            name: "Bronze scimitar",
            count: 1,
        },
    ));
    steps.push(Step {
        name: "wear the seeded bronze scimitar before Start",
        kind: StepKind::Repeat {
            send: Box::new(|c, snap| {
                pair_wear_first_inv(c, snap);
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemAtMost {
                name: "Bronze scimitar",
                count: 0,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    Scenario {
        name: "duel_arena",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::DuelPeer)],
        settings: pair_watch_settings("duel_arena", "Duel Arena Combat Trainer"),
    }
}

