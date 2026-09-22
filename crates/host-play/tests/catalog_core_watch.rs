use std::sync::Arc;

use host_play::catalog_core::{
    firemaker_spec, line_of_sight_dest_vis_alone, line_of_sight_pair_is_blocked,
    line_of_sight_pair_is_open, select_line_of_sight_pairs, BoundedLoc, CoreCase, CoreWatch,
    CoreWatchStatus, FiremakerCycle, LineOfSightHere, LineOfSightIdentity, LineOfSightObservation,
    LineOfSightPair, LineOfSightPairResult, LineOfSightScriptReceipt, LineOfSightTile, Observation,
    RouteInspectHopFact, BRIMHAVEN_INSPECT_BANK, BRIMHAVEN_INSPECT_FIELD, BRIMHAVEN_INSPECT_PIER,
    CERT_RUNE_CHAINBODY_ID, COINS_ID, HIGH_ALCH_MAGIC_XP, LOBSTER_ID, LOS_V2_STOP, LOS_VIS_SCENERY,
    LOS_V_E, LOS_V_W, LOS_WALK_SCENERY, NATURE_RUNE_ID, OAK_LOGS_ID, RUNE_CHAINBODY_HIGH_ALCH_COINS,
    STAFF_OF_FIRE_ID, TINDERBOX_ID, UNIDENTIFIED_GUAM_ID, UNIDENTIFIED_MARENTILL_ID,
    VARROCK_EAST_BANK, VARROCK_WEST_BANK,
};
use host_play::catalog_core::{
    ActorObservation, ActorObservationLos, ActorObservationNpc, ActorObservationNpcFact,
    ActorObservationPacked, ActorObservationPoint, ActorObservationScriptReceipt,
    ActorObservationSelfTarget, FightFieldNpc, FightFieldObservation, FightFieldScriptReceipt,
    HoldSpotObservation, HoldSpotScriptReceipt, RetreatSpotObservation, RetreatSpotScriptReceipt,
    WalkSpotObservation, WalkSpotScriptReceipt, ACTOR_OBSERVATION_V2_STOP, ENTER_LAIR_V2_STOP,
    FIGHT_FIELD_V2_STOP, HOLD_SPOT_V2_STOP, LEAVE_LAIR_RECEIPT_PREFIX, LEAVE_LAIR_V2_STOP,
    RETREAT_SPOT_V2_STOP, WALK_SPOT_V2_STOP, ACQUIRE_KEY_CELL, ACQUIRE_KEY_DEST, ACQUIRE_KEY_LAIR,
    ACQUIRE_KEY_RECEIPT_PREFIX, ACQUIRE_KEY_V2_STOP,
};
use host_play::catalog_core::{
    EnterLairBox, EnterLairObservation, EnterLairScriptReceipt, LeaveLairBox, LeaveLairObservation,
    LeaveLairScriptReceipt, parse_leave_lair_receipt_line, AcquireKeyObservation,
    AcquireKeyScriptReceipt, parse_acquire_key_receipt_line,
};
use host_play::catalog_core::{
    parse_cell_v2_receipt_line, CellV2Observation, CellV2ScriptReceipt, CELL_V2_CELL, CELL_V2_DOOR,
    CELL_V2_LAIR, CELL_V2_RECEIPT_PREFIX, CELL_V2_STOP,
};

fn thiever_observation() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((2661, 3306, 0)),
        ..Observation::default()
    };
    observation.levels.insert("thieving".into(), 50);
    observation.levels.insert("hitpoints".into(), 50);
    observation.effective_levels.insert("thieving".into(), 50);
    observation.effective_levels.insert("hitpoints".into(), 50);
    observation.items.insert("Lobster".into(), 10);
    observation
}

#[test]
fn headed_watch_rejects_wrong_and_invalidates_pre_start_session_baselines() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    let mut wrong = thiever_observation();
    wrong.player = Some("other".into());
    watch.observe("catalogtest", wrong, false);
    assert!(watch.begin_start("catalogtest").is_err());
    let error = watch.failure().expect("wrong account fails the watch");
    assert!(error.contains("fresh account"), "{error}");

    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", thiever_observation(), false);
    watch.observe("catalogtest", Observation::default(), true);
    assert!(
        watch.failure().is_none(),
        "initial login is allowed before Start"
    );
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "the prior-session candidate baseline was invalidated"
    );
    watch.observe("catalogtest", thiever_observation(), false);
    watch.begin_start("catalogtest").unwrap();

    watch.observe("catalogtest", Observation::default(), true);
    assert!(watch
        .failure()
        .expect("a post-Start boundary is terminal")
        .contains("session boundary after Start"));
}

#[test]
fn headed_watch_requires_post_start_core_before_qualifying() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    let baseline = thiever_observation();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    assert!(watch
        .qualify()
        .unwrap_err()
        .contains("no post-Start observations"));

    watch.observe("catalogtest", baseline.clone(), false);
    assert!(watch
        .qualify()
        .unwrap_err()
        .contains("core post-Start delta incomplete"));

    let mut worked = baseline;
    worked.tick += 1;
    worked.xp.insert("thieving".into(), 1);
    worked.items.insert("Coins".into(), 1);
    watch.observe("catalogtest", worked, false);

    assert_eq!(watch.status(), CoreWatchStatus::Qualified);
    let evidence = watch.qualify().expect("full Thiever core qualifies");
    let cached = watch.qualify().expect("terminal receipt remains cached");
    assert!(
        Arc::ptr_eq(&evidence, &cached),
        "terminal polling must reuse one immutable receipt"
    );
    assert_eq!(evidence["case"], "thiever");
    assert_eq!(evidence["post_start_observations"], 2);
}

#[test]
fn start_uses_the_last_published_pre_start_observation() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    let baseline = thiever_observation();
    watch.observe("catalogtest", baseline.clone(), false);

    // begin_start is called immediately before the isolate Start. The first
    // later producer observation is already allowed to contain script work.
    watch.begin_start("catalogtest").unwrap();
    let mut first_after_start = baseline;
    first_after_start.tick += 1;
    first_after_start.xp.insert("thieving".into(), 1);
    first_after_start.items.insert("Coins".into(), 1);
    watch.observe("catalogtest", first_after_start, false);

    let evidence = watch.qualify().expect("first post-Start work qualifies");
    assert_eq!(evidence["post_start_observations"], 1);
    assert!(
        evidence["baseline"]["items"]["Coins"].is_null(),
        "the post-Start coin must not leak into the frozen baseline"
    );
}

fn herb_empty_observation(tick: u32, bank_open: bool, bank_loaded: bool) -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3185, 3440, 0)),
        tick,
        bank_open,
        bank_loaded,
        bank_generation: u64::from(tick),
        ..Observation::default()
    };
    observation.levels.insert("herblore".into(), 20);
    if bank_loaded {
        observation.bank_ids.insert(UNIDENTIFIED_GUAM_ID, 20);
        observation.bank_ids.insert(UNIDENTIFIED_MARENTILL_ID, 0);
    }
    observation
}

#[test]
fn herb_cleaner_start_requires_loaded_seed_then_closed_bank_in_the_same_run() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    watch.observe("catalogtest", herb_empty_observation(1, true, true), false);
    let closed = herb_empty_observation(2, false, false);
    watch.observe("catalogtest", closed.clone(), false);

    watch
        .begin_start("catalogtest")
        .expect("an observed loaded seed followed by a closed-bank baseline starts");
    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["baseline"]["bank_open"], false);
    assert_eq!(evidence["witness"]["baseline"]["bank_loaded"], false);
    assert_eq!(evidence["witness"]["start_preparation"]["guam_count"], 20);
    assert_eq!(
        evidence["witness"]["start_preparation"]["marrentill_count"],
        0
    );

    let missing_seed = CoreWatch::default();
    missing_seed.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    missing_seed.observe("catalogtest", closed.clone(), false);
    assert!(missing_seed.begin_start("catalogtest").is_err());

    let unloaded = CoreWatch::default();
    unloaded.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    let mut unacknowledged = herb_empty_observation(1, true, false);
    unacknowledged.bank_ids.insert(UNIDENTIFIED_GUAM_ID, 20);
    unloaded.observe("catalogtest", unacknowledged, false);
    unloaded.observe("catalogtest", closed.clone(), false);
    assert!(unloaded.begin_start("catalogtest").is_err());

    let wrong_seed = CoreWatch::default();
    wrong_seed.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    let mut nineteen_guam = herb_empty_observation(1, true, true);
    nineteen_guam.bank_ids.insert(UNIDENTIFIED_GUAM_ID, 19);
    wrong_seed.observe("catalogtest", nineteen_guam, false);
    wrong_seed.observe("catalogtest", closed.clone(), false);
    assert!(wrong_seed.begin_start("catalogtest").is_err());

    let wrong_order = CoreWatch::default();
    wrong_order.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    wrong_order.observe("catalogtest", closed.clone(), false);
    wrong_order.observe("catalogtest", herb_empty_observation(3, true, true), false);
    assert!(wrong_order.begin_start("catalogtest").is_err());

    let stale_run = CoreWatch::default();
    stale_run.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    stale_run.observe("catalogtest", herb_empty_observation(1, true, true), false);
    stale_run.observe("catalogtest", closed.clone(), false);
    stale_run.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    stale_run.observe("catalogtest", closed, false);
    assert!(stale_run.begin_start("catalogtest").is_err());

    let stale_session = CoreWatch::default();
    stale_session.configure(CoreCase::HerbCleanerEmptyBank, "catalogtest");
    stale_session.observe("catalogtest", herb_empty_observation(1, true, true), false);
    stale_session.observe("catalogtest", Observation::default(), true);
    stale_session.observe(
        "catalogtest",
        herb_empty_observation(2, false, false),
        false,
    );
    assert!(stale_session.begin_start("catalogtest").is_err());
}

fn fire_observation(tick: u32, logs: i32, xp: i32, bank_open: bool) -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(VARROCK_EAST_BANK),
        tick,
        bank_open,
        bank_loaded: bank_open,
        bank_generation: u64::from(bank_open),
        ..Observation::default()
    };
    observation.item_ids.insert(OAK_LOGS_ID, logs);
    observation.item_ids.insert(TINDERBOX_ID, 1);
    observation.xp.insert("firemaking".into(), xp);
    observation
}

fn fire_observation_with_plot(tick: u32, logs: i32, xp: i32, bank_open: bool) -> Observation {
    let mut observation = fire_observation(tick, logs, xp, bank_open);
    observation.loc_facts.push(BoundedLoc {
        id: 0,
        x: 3250,
        z: 3420,
        level: 0,
        name: Some("Fire".into()),
        open: false,
    });
    observation
}

#[test]
fn firemaker_does_not_reuse_stale_xp_or_old_fire_after_partial_restock() {
    let spec = firemaker_spec(CoreCase::FiremakerOak).unwrap();
    let baseline = fire_observation(1, 0, 1_000, false);
    let mut cycle = FiremakerCycle::default();

    cycle.observe(spec, &baseline, &fire_observation(2, 1, 1_000, false));
    cycle.observe(
        spec,
        &baseline,
        &fire_observation_with_plot(3, 0, 1_015, false),
    );
    let mut deposited = fire_observation(4, 0, 2_000, true);
    deposited.bank_ids.insert(OAK_LOGS_ID, 28);
    cycle.observe(spec, &baseline, &deposited);

    let mut restocked = fire_observation(5, 1, 2_000, true);
    restocked.bank_generation = 1;
    restocked.bank_ids.insert(OAK_LOGS_ID, 27);
    cycle.observe(spec, &baseline, &restocked);

    let mut returned_without_new_burn = fire_observation_with_plot(6, 1, 2_000, false);
    returned_without_new_burn.bank_generation = 2;
    cycle.observe(spec, &baseline, &returned_without_new_burn);
    assert!(
        !cycle.qualified(),
        "close plus stale fire/XP must not qualify"
    );

    let mut actual_burn = fire_observation_with_plot(7, 0, 2_015, false);
    actual_burn.bank_generation = 2;
    cycle.observe(spec, &baseline, &actual_burn);
    assert!(
        cycle.qualified(),
        "a post-restock log loss and XP gain qualifies"
    );
}

fn swarm_baseline() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(VARROCK_WEST_BANK),
        ..Observation::default()
    };
    observation.levels.insert("magic".into(), 70);
    observation.xp.insert("magic".into(), 10_000);
    observation
}

fn swarm_stop_receipt() -> script::ScriptLifecycleReceipt {
    script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: 200,
        reason: "the bank is out of every selected item — stopping".into(),
    }
}

#[test]
fn alcher_swarm_stop_without_recovery_fails_the_headed_watch_with_phase_status() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::AlcherSwarmDrain, "catalogtest");
    let baseline = swarm_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut withdrawn = baseline.clone();
    withdrawn.bank_generation = 2;
    withdrawn.item_ids.insert(CERT_RUNE_CHAINBODY_ID, 20);
    withdrawn.item_ids.insert(NATURE_RUNE_ID, 20);
    watch.observe("catalogtest", withdrawn.clone(), false);

    let mut first = withdrawn;
    first.item_ids.insert(CERT_RUNE_CHAINBODY_ID, 19);
    first.item_ids.insert(NATURE_RUNE_ID, 19);
    first
        .item_ids
        .insert(COINS_ID, RUNE_CHAINBODY_HIGH_ALCH_COINS);
    first.xp.insert("magic".into(), 10_000 + HIGH_ALCH_MAGIC_XP);
    first.equipment_ids.insert(STAFF_OF_FIRE_ID, 1);
    watch.observe("catalogtest", first.clone(), false);
    assert_eq!(watch.status(), CoreWatchStatus::Running);

    let mut stopped = first;
    stopped.script_lifecycle = Some(swarm_stop_receipt());
    watch.observe("catalogtest", stopped, false);

    assert_eq!(watch.status(), CoreWatchStatus::Failed);
    let error = watch.failure().expect("incomplete swarm stop is terminal");
    assert!(
        error.contains("stopped without ordered recovery"),
        "{error}"
    );
    assert!(error.contains("\"firstcast\":true"), "{error}");
    assert!(error.contains("\"swarm_hit\":false"), "{error}");
    assert!(error.contains("\"guardianhold\":false"), "{error}");
    assert!(error.contains("\"fled\":false"), "{error}");
    assert!(error.contains("\"released\":false"), "{error}");
    assert!(error.contains("\"further\":false"), "{error}");
    assert!(error.contains("\"richretired\":false"), "{error}");
    assert!(error.contains("\"poor\":false"), "{error}");
    let evidence = watch.evidence();
    assert_eq!(evidence["phase"], "failed");
    assert!(evidence["witness"]["alcher_swarm_cycle"]["first_cast"].is_object());
    assert!(evidence["witness"]["alcher_swarm_cycle"]["swarm_hit"].is_null());
}

#[test]
fn alcher_swarm_stop_does_not_fail_a_qualified_recovery() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::AlcherSwarmDrain, "catalogtest");
    let baseline = swarm_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut qualified = baseline;
    qualified.script_lifecycle = Some(swarm_stop_receipt());
    // A stop on an unqualified empty run fails; this test only checks that a
    // later stop cannot clobber an already-cached qualification.
    watch.observe("catalogtest", qualified, false);
    assert_eq!(watch.status(), CoreWatchStatus::Failed);

    let keep = CoreWatch::default();
    keep.configure(CoreCase::Thiever, "catalogtest");
    let thiever = thiever_observation();
    keep.observe("catalogtest", thiever.clone(), false);
    keep.begin_start("catalogtest").unwrap();
    let mut worked = thiever;
    worked.tick += 1;
    worked.xp.insert("thieving".into(), 1);
    worked.items.insert("Coins".into(), 1);
    keep.observe("catalogtest", worked, false);
    assert_eq!(keep.status(), CoreWatchStatus::Qualified);
    let mut after_stop = thiever_observation();
    after_stop.tick += 2;
    after_stop.xp.insert("thieving".into(), 1);
    after_stop.items.insert("Coins".into(), 1);
    after_stop.script_lifecycle = Some(swarm_stop_receipt());
    keep.observe("catalogtest", after_stop, false);
    assert_eq!(
        keep.status(),
        CoreWatchStatus::Qualified,
        "a later Stop must not move a different qualified core"
    );
}

use host_play::catalog_core::{
    ARCHERY_TICKET_ID, BRONZE_ARROW_ID, COINS_PER_TRIP, ENTRY_FEE, MAGIC_SHORTBOW_ID,
    RANGING_GUILD_FULL_STOP_NEEDLE, RANGING_GUILD_MERCHANT_STAND, RANGING_GUILD_SEERS_BANK,
    RANGING_GUILD_STAND, RUNE_ARROWS_PER_TRADE, RUNE_ARROW_ID, SEED_KEEP_TICKETS,
    TARGET_RESULT_MODAL, TICKETS_PER_TRADE, VARP_TARGET_COUNT, VARP_TARGET_SCORE,
};

fn ranging_round_baseline() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(RANGING_GUILD_STAND),
        ..Observation::default()
    };
    observation.levels.insert("ranged".into(), 70);
    observation.effective_levels.insert("ranged".into(), 70);
    observation.item_ids.insert(MAGIC_SHORTBOW_ID, 1);
    observation.item_ids.insert(COINS_ID, ENTRY_FEE * 2);
    observation.varps.insert(VARP_TARGET_COUNT, 0);
    observation.varps.insert(VARP_TARGET_SCORE, 0);
    observation
}

fn ranging_redeem_baseline() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(RANGING_GUILD_MERCHANT_STAND),
        ..Observation::default()
    };
    observation.levels.insert("ranged".into(), 70);
    observation.effective_levels.insert("ranged".into(), 70);
    observation.item_ids.insert(MAGIC_SHORTBOW_ID, 1);
    observation
        .item_ids
        .insert(ARCHERY_TICKET_ID, TICKETS_PER_TRADE);
    observation
}

#[test]
fn ranging_guild_round_rejects_seeded_state_and_requires_ordered_second_enter() {
    assert_eq!(
        CoreCase::parse("ranging_guild_round").unwrap(),
        CoreCase::RangingGuildRound
    );
    assert_eq!(CoreCase::RangingGuildRound.card_name(), "RangingGuild");

    let watch = CoreWatch::default();
    watch.configure(CoreCase::RangingGuildRound, "catalogtest");
    let baseline = ranging_round_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "repeating the seeded stand must not qualify"
    );

    let mut paid = baseline.clone();
    paid.item_ids.insert(COINS_ID, ENTRY_FEE);
    paid.varps.insert(VARP_TARGET_COUNT, 1);
    watch.observe("catalogtest", paid.clone(), false);
    assert!(watch.qualify().is_err(), "fee alone is not a round");

    let mut shot = paid.clone();
    shot.varps.insert(VARP_TARGET_COUNT, 2);
    shot.xp.insert("ranged".into(), 8);
    watch.observe("catalogtest", shot.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "a shot without payout is incomplete"
    );

    let mut reset = shot.clone();
    reset.varps.insert(VARP_TARGET_COUNT, 0);
    reset.item_ids.insert(ARCHERY_TICKET_ID, 3);
    watch.observe("catalogtest", reset.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "one payout must not stand in for a further round"
    );

    let mut second = reset;
    second.item_ids.insert(COINS_ID, 0);
    second.varps.insert(VARP_TARGET_COUNT, 1);
    watch.observe("catalogtest", second, false);
    watch.qualify().expect("paid shot payout and second enter");
}

#[test]
fn ranging_guild_round_does_not_treat_seeded_tickets_as_payout() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::RangingGuildRound, "catalogtest");
    let mut seeded = ranging_round_baseline();
    seeded.item_ids.insert(ARCHERY_TICKET_ID, 20);
    watch.observe("catalogtest", seeded.clone(), false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "seeded tickets are not a round baseline"
    );
}

#[test]
fn ranging_guild_redeem_requires_real_ticket_spend_and_arrow_gain() {
    assert_eq!(
        CoreCase::parse("ranging_guild_redeem").unwrap(),
        CoreCase::RangingGuildRedeem
    );
    assert_eq!(CoreCase::RangingGuildRedeem.card_name(), "RangingGuild");

    let start = |observation: Observation| {
        let watch = CoreWatch::default();
        watch.configure(CoreCase::RangingGuildRedeem, "catalogtest");
        watch.observe("catalogtest", observation, false);
        watch.begin_start("catalogtest").unwrap();
        watch
    };
    let baseline = ranging_redeem_baseline();
    let watch = start(baseline.clone());
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seeded 2000 tickets must not count as a spend"
    );

    let modal_only = start(baseline.clone());
    let mut opened = baseline.clone();
    opened.main_modal = 4461;
    modal_only.observe("catalogtest", opened, false);
    assert!(
        modal_only.qualify().is_err(),
        "opening the ticket shop without a trade is not redemption"
    );

    let spent_only = start(baseline.clone());
    let mut spent = baseline.clone();
    spent.item_ids.insert(ARCHERY_TICKET_ID, 0);
    spent_only.observe("catalogtest", spent, false);
    assert!(
        spent_only.qualify().is_err(),
        "ticket loss without rune arrows is not redemption"
    );

    let arrows_only = start(baseline.clone());
    let mut arrows = baseline.clone();
    arrows.item_ids.insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    arrows_only.observe("catalogtest", arrows, false);
    assert!(
        arrows_only.qualify().is_err(),
        "arrows without a ticket spend can be a seed, not a buy"
    );

    let traded = start(baseline.clone());
    let mut buy = baseline;
    buy.item_ids.insert(ARCHERY_TICKET_ID, 0);
    buy.item_ids.insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    traded.observe("catalogtest", buy, false);
    traded
        .qualify()
        .expect("2000 seeded tickets spent for 50 rune arrows");
}

#[test]
fn ranging_guild_redeem_rejects_already_held_arrows_as_baseline() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::RangingGuildRedeem, "catalogtest");
    let mut seeded_arrows = ranging_redeem_baseline();
    seeded_arrows
        .item_ids
        .insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    watch.observe("catalogtest", seeded_arrows, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "held rune arrows are not a redemption baseline"
    );
}

fn ranging_bank_baseline() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(RANGING_GUILD_SEERS_BANK),
        ..Observation::default()
    };
    observation.levels.insert("ranged".into(), 70);
    observation.effective_levels.insert("ranged".into(), 70);
    observation
        .item_ids
        .insert(ARCHERY_TICKET_ID, SEED_KEEP_TICKETS);
    observation
        .item_ids
        .insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    observation.varps.insert(VARP_TARGET_COUNT, 0);
    observation
}

fn ranging_bank_open(
    baseline: &Observation,
    coins_in_bank: i32,
    arrows_in_bank: i32,
) -> Observation {
    let mut now = baseline.clone();
    now.tick += 1;
    now.bank_open = true;
    now.bank_loaded = true;
    now.bank_generation = baseline.bank_generation + 1;
    now.item_ids.insert(RUNE_ARROW_ID, 0);
    now.bank_ids.insert(RUNE_ARROW_ID, arrows_in_bank);
    now.bank_ids.insert(COINS_ID, coins_in_bank);
    now.bank_ids.insert(MAGIC_SHORTBOW_ID, 1);
    now
}

#[test]
fn ranging_guild_bank_rejects_preseed_wrong_bank_and_missing_return() {
    assert_eq!(
        CoreCase::parse("ranging_guild_bank").unwrap(),
        CoreCase::RangingGuildBank
    );
    assert_eq!(CoreCase::RangingGuildBank.card_name(), "RangingGuild");

    let start = |observation: Observation| {
        let watch = CoreWatch::default();
        watch.configure(CoreCase::RangingGuildBank, "catalogtest");
        watch.observe("catalogtest", observation, false);
        watch.begin_start("catalogtest").unwrap();
        watch
    };
    let baseline = ranging_bank_baseline();
    let watch = start(baseline.clone());
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "preseeded Seers KEEP/arrows alone must not qualify"
    );

    let wrong = start(baseline.clone());
    let mut varrock = ranging_bank_open(&baseline, COINS_PER_TRIP, RUNE_ARROWS_PER_TRADE);
    varrock.tile = Some(VARROCK_WEST_BANK);
    varrock.item_ids.insert(COINS_ID, COINS_PER_TRIP);
    varrock.bank_ids.insert(COINS_ID, 0);
    wrong.observe("catalogtest", varrock, false);
    assert!(
        wrong.qualify().is_err(),
        "a loaded booth away from Seers cannot qualify"
    );

    let no_return = start(baseline.clone());
    let mut deposited = ranging_bank_open(&baseline, COINS_PER_TRIP, RUNE_ARROWS_PER_TRADE);
    no_return.observe("catalogtest", deposited.clone(), false);
    deposited.item_ids.insert(COINS_ID, COINS_PER_TRIP);
    deposited.bank_ids.insert(COINS_ID, 0);
    no_return.observe("catalogtest", deposited.clone(), false);
    let mut closed = deposited;
    closed.bank_open = false;
    closed.bank_loaded = false;
    closed.bank_generation += 1;
    closed.bank_ids.clear();
    no_return.observe("catalogtest", closed.clone(), false);
    assert!(
        no_return.qualify().is_err(),
        "deposit, withdraw and close without STAND return is incomplete"
    );

    let mut fee_at_bank = closed;
    fee_at_bank.item_ids.insert(COINS_ID, ENTRY_FEE);
    fee_at_bank.varps.insert(VARP_TARGET_COUNT, 1);
    no_return.observe("catalogtest", fee_at_bank, false);
    assert!(
        no_return.qualify().is_err(),
        "a fee at Seers is not a STAND return"
    );
}

#[test]
fn ranging_guild_bank_requires_keep_coin_identity_close_return_and_fee() {
    let start = |observation: Observation| {
        let watch = CoreWatch::default();
        watch.configure(CoreCase::RangingGuildBank, "catalogtest");
        watch.observe("catalogtest", observation, false);
        watch.begin_start("catalogtest").unwrap();
        watch
    };
    let baseline = ranging_bank_baseline();

    let deposited_tickets = start(baseline.clone());
    let mut lost_keep = ranging_bank_open(&baseline, COINS_PER_TRIP, RUNE_ARROWS_PER_TRADE);
    lost_keep.item_ids.insert(ARCHERY_TICKET_ID, 0);
    deposited_tickets.observe("catalogtest", lost_keep, false);
    assert!(
        deposited_tickets.qualify().is_err(),
        "depositing KEEP tickets is not the rune-arrow KEEP deposit"
    );

    let coins_without_bank = start(baseline.clone());
    let mut gifted = baseline.clone();
    gifted.tick += 1;
    gifted.item_ids.insert(COINS_ID, COINS_PER_TRIP);
    gifted.item_ids.insert(RUNE_ARROW_ID, 0);
    coins_without_bank.observe("catalogtest", gifted, false);
    assert!(
        coins_without_bank.qualify().is_err(),
        "pack coins without a Seers bank session are not withdraw-X"
    );

    let watch = start(baseline.clone());
    let mut deposited = ranging_bank_open(&baseline, COINS_PER_TRIP, RUNE_ARROWS_PER_TRADE);
    watch.observe("catalogtest", deposited.clone(), false);
    assert!(watch.qualify().is_err(), "deposit alone is incomplete");

    deposited.item_ids.insert(COINS_ID, COINS_PER_TRIP);
    deposited.bank_ids.insert(COINS_ID, 0);
    watch.observe("catalogtest", deposited.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "coin withdraw without close and return is incomplete"
    );

    let mut closed = deposited.clone();
    closed.bank_open = false;
    closed.bank_loaded = false;
    closed.bank_generation += 1;
    closed.bank_ids.clear();
    watch.observe("catalogtest", closed.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "close without STAND return is incomplete"
    );

    let mut returned = closed;
    returned.tile = Some(RANGING_GUILD_STAND);
    watch.observe("catalogtest", returned.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "return without a subsequent fee is incomplete"
    );

    let mut fee = returned;
    fee.item_ids.insert(COINS_ID, ENTRY_FEE);
    fee.varps.insert(VARP_TARGET_COUNT, 1);
    watch.observe("catalogtest", fee, false);
    watch
        .qualify()
        .expect("KEEP deposit, coin withdraw, close, STAND return and fee");
}

#[test]
fn ranging_guild_bank_rejects_redeem_stack_and_already_funded_pack() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::RangingGuildBank, "catalogtest");
    let mut redeem_stack = ranging_bank_baseline();
    redeem_stack
        .item_ids
        .insert(ARCHERY_TICKET_ID, TICKETS_PER_TRADE);
    watch.observe("catalogtest", redeem_stack, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "2000 tickets would redeem before banking"
    );

    let funded = CoreWatch::default();
    funded.configure(CoreCase::RangingGuildBank, "catalogtest");
    let mut coins = ranging_bank_baseline();
    coins.item_ids.insert(COINS_ID, COINS_PER_TRIP);
    funded.observe("catalogtest", coins, false);
    assert!(
        funded.begin_start("catalogtest").is_err(),
        "pack coins at Start are not a Seers withdraw baseline"
    );
}

fn ranging_full_baseline() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(RANGING_GUILD_SEERS_BANK),
        ..Observation::default()
    };
    observation.levels.insert("ranged".into(), 70);
    observation.effective_levels.insert("ranged".into(), 70);
    observation
        .item_ids
        .insert(ARCHERY_TICKET_ID, SEED_KEEP_TICKETS);
    observation.varps.insert(VARP_TARGET_COUNT, 0);
    observation
}

fn ranging_full_stop(reason: &str) -> script::ScriptLifecycleReceipt {
    script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: 400,
        reason: reason.into(),
    }
}

fn ranging_full_start(observation: Observation) -> CoreWatch {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::RangingGuildFull, "catalogtest");
    watch.observe("catalogtest", observation, false);
    watch.begin_start("catalogtest").unwrap();
    watch
}

fn ranging_full_earned_tickets(baseline: &Observation) -> Observation {
    let mut now = baseline.clone();
    now.tick += 1;
    now.tile = Some(RANGING_GUILD_STAND);
    now.item_ids
        .insert(ARCHERY_TICKET_ID, TICKETS_PER_TRADE);
    now.item_ids.insert(COINS_ID, ENTRY_FEE);
    now
}

fn ranging_full_bought(from: &Observation) -> Observation {
    let mut now = from.clone();
    now.tick += 1;
    now.tile = Some(RANGING_GUILD_MERCHANT_STAND);
    now.item_ids.insert(ARCHERY_TICKET_ID, 0);
    now.item_ids.insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    now.item_ids.insert(COINS_ID, ENTRY_FEE);
    now
}

fn ranging_full_result_then_continue(from: &Observation) -> (Observation, Observation) {
    let mut opened = from.clone();
    opened.tick += 1;
    opened.tile = Some(RANGING_GUILD_STAND);
    opened.main_modal = TARGET_RESULT_MODAL;
    opened.varps.insert(VARP_TARGET_COUNT, 2);
    let mut closed = opened.clone();
    closed.tick += 1;
    closed.main_modal = -1;
    closed.varps.insert(VARP_TARGET_COUNT, 3);
    (opened, closed)
}

fn ranging_full_bank_bought(from: &Observation, arrows: i32, item: i32) -> Observation {
    let mut now = from.clone();
    now.tick += 1;
    now.tile = Some(RANGING_GUILD_SEERS_BANK);
    now.bank_open = true;
    now.bank_loaded = true;
    now.bank_generation = from.bank_generation + 1;
    now.item_ids.insert(RUNE_ARROW_ID, 0);
    now.bank_ids.insert(item, arrows);
    now.item_ids.insert(COINS_ID, 0);
    now.item_ids.insert(ARCHERY_TICKET_ID, 50);
    now
}

#[test]
fn ranging_guild_full_rejects_preseed_wrong_item_no_shop_buy_no_open_modal_and_deadline_only_stop() {
    assert_eq!(
        CoreCase::parse("ranging_guild_full").unwrap(),
        CoreCase::RangingGuildFull
    );
    assert_eq!(CoreCase::RangingGuildFull.card_name(), "RangingGuild");

    let baseline = ranging_full_baseline();
    let preseed = ranging_full_start(baseline.clone());
    preseed.observe("catalogtest", baseline.clone(), false);
    assert!(
        preseed.qualify().unwrap_err().contains("incomplete"),
        "preseeded Seers KEEP tickets alone must not qualify"
    );

    let seeded_arrows = CoreWatch::default();
    seeded_arrows.configure(CoreCase::RangingGuildFull, "catalogtest");
    let mut packed = baseline.clone();
    packed.item_ids.insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    seeded_arrows.observe("catalogtest", packed, false);
    assert!(
        seeded_arrows.begin_start("catalogtest").is_err(),
        "seeded pack rune arrows are the bank cell, not fullflow"
    );

    let no_buy = ranging_full_start(baseline.clone());
    let mut gifted = baseline.clone();
    gifted.tick += 1;
    gifted.item_ids.insert(RUNE_ARROW_ID, RUNE_ARROWS_PER_TRADE);
    no_buy.observe("catalogtest", gifted.clone(), false);
    let deposited = ranging_full_bank_bought(&gifted, RUNE_ARROWS_PER_TRADE, RUNE_ARROW_ID);
    no_buy.observe("catalogtest", deposited, false);
    assert!(
        no_buy.qualify().is_err(),
        "arrow gift plus bank without a 2000-ticket shop spend is not bought-then-banked"
    );

    let earned = ranging_full_earned_tickets(&baseline);
    let bought = ranging_full_bought(&earned);
    let wrong_item = ranging_full_start(baseline.clone());
    wrong_item.observe("catalogtest", earned.clone(), false);
    wrong_item.observe("catalogtest", bought.clone(), false);
    let bronze = ranging_full_bank_bought(&bought, RUNE_ARROWS_PER_TRADE, BRONZE_ARROW_ID);
    wrong_item.observe("catalogtest", bronze, false);
    assert!(
        wrong_item.qualify().is_err(),
        "banking bronze arrows is not the shop-bought rune-arrow deposit"
    );

    let no_modal = ranging_full_start(baseline.clone());
    no_modal.observe("catalogtest", earned.clone(), false);
    let mut shop_modal = bought.clone();
    shop_modal.main_modal = 4461;
    no_modal.observe("catalogtest", shop_modal, false);
    let banked = ranging_full_bank_bought(&bought, RUNE_ARROWS_PER_TRADE, RUNE_ARROW_ID);
    no_modal.observe("catalogtest", banked.clone(), false);
    let mut empty = banked.clone();
    empty.bank_open = false;
    empty.bank_loaded = false;
    empty.bank_generation += 1;
    empty.item_ids.insert(COINS_ID, 0);
    empty.script_lifecycle = Some(ranging_full_stop(&format!(
        "RangingGuild: {RANGING_GUILD_FULL_STOP_NEEDLE}, 50 tickets held, 50 rune arrows bought"
    )));
    no_modal.observe("catalogtest", empty, false);
    assert!(
        no_modal.qualify().is_err(),
        "ticket-shop 4461 or never opening result modal 446 cannot qualify"
    );

    let bank_close_only = ranging_full_start(baseline.clone());
    bank_close_only.observe("catalogtest", earned.clone(), false);
    bank_close_only.observe("catalogtest", bought.clone(), false);
    let mut opened_only = bought.clone();
    opened_only.tick += 1;
    opened_only.main_modal = TARGET_RESULT_MODAL;
    opened_only.varps.insert(VARP_TARGET_COUNT, 2);
    bank_close_only.observe("catalogtest", opened_only.clone(), false);
    let mut closed_bank = opened_only;
    closed_bank.tick += 1;
    closed_bank.main_modal = -1;
    closed_bank.bank_open = false;
    closed_bank.bank_loaded = false;
    closed_bank.bank_generation += 1;
    bank_close_only.observe("catalogtest", closed_bank.clone(), false);
    let banked_after_close = ranging_full_bank_bought(&closed_bank, RUNE_ARROWS_PER_TRADE, RUNE_ARROW_ID);
    bank_close_only.observe("catalogtest", banked_after_close.clone(), false);
    let mut stopped_without_continue = banked_after_close;
    stopped_without_continue.bank_open = false;
    stopped_without_continue.bank_loaded = false;
    stopped_without_continue.bank_generation += 1;
    stopped_without_continue.item_ids.insert(COINS_ID, 0);
    stopped_without_continue.script_lifecycle = Some(ranging_full_stop(&format!(
        "RangingGuild: {RANGING_GUILD_FULL_STOP_NEEDLE}, 0 tickets held, 50 rune arrows bought"
    )));
    bank_close_only.observe("catalogtest", stopped_without_continue, false);
    assert!(
        bank_close_only.qualify().is_err(),
        "closing the bank after modal 446 is not script continuation"
    );

    let deadline = ranging_full_start(baseline);
    deadline.observe("catalogtest", earned, false);
    deadline.observe("catalogtest", bought.clone(), false);
    let (opened, continued) = ranging_full_result_then_continue(&bought);
    deadline.observe("catalogtest", opened, false);
    deadline.observe("catalogtest", continued.clone(), false);
    deadline.observe("catalogtest", banked.clone(), false);
    let mut empty_only = banked;
    empty_only.bank_open = false;
    empty_only.bank_loaded = false;
    empty_only.bank_generation += 1;
    empty_only.item_ids.insert(COINS_ID, 0);
    empty_only.script_lifecycle = Some(ranging_full_stop("harness deadline"));
    deadline.observe("catalogtest", empty_only, false);
    assert!(
        deadline.qualify().is_err(),
        "a deadline/timeout stop is not the source out-of-coins receipt"
    );
}

#[test]
fn ranging_guild_full_requires_bought_then_same_arrow_bank_result_modal_close_continue_and_honest_stop()
{
    let baseline = ranging_full_baseline();
    let watch = ranging_full_start(baseline.clone());
    let earned = ranging_full_earned_tickets(&baseline);
    let bought = ranging_full_bought(&earned);
    watch.observe("catalogtest", earned, false);
    watch.observe("catalogtest", bought.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "shop buy without bank, modal, and stop is incomplete"
    );

    let (opened, continued) = ranging_full_result_then_continue(&bought);
    watch.observe("catalogtest", opened, false);
    assert!(
        watch.qualify().is_err(),
        "an open result modal without close and continuation is incomplete"
    );
    watch.observe("catalogtest", continued.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "modal close plus continuation without bought-then-banked stop is incomplete"
    );

    let banked = ranging_full_bank_bought(&continued, RUNE_ARROWS_PER_TRADE, RUNE_ARROW_ID);
    watch.observe("catalogtest", banked.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "bought-then-banked without the script stop is incomplete"
    );

    let mut stopped = banked;
    stopped.bank_open = false;
    stopped.bank_loaded = false;
    stopped.bank_generation += 1;
    stopped.item_ids.insert(COINS_ID, 0);
    stopped.script_lifecycle = Some(ranging_full_stop(&format!(
        "RangingGuild: {RANGING_GUILD_FULL_STOP_NEEDLE}, 50 tickets held, 50 rune arrows bought"
    )));
    watch.observe("catalogtest", stopped, false);
    watch
        .qualify()
        .expect("shop buy, same-arrow bank, modal close+continue, and out-of-coins stop");
}

fn brimhaven_v1_baseline() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(BRIMHAVEN_INSPECT_BANK),
        ..Observation::default()
    };
    observation.levels.insert("agility".into(), 30);
    // Session `reset_inspect` already bumped generation and cleared terminals.
    // Missing-terminal defaults are not the live generation.
    observation.route_inspect_live_generation = 1;
    observation.route_inspect_has_terminal = false;
    observation
}

fn with_hop(mut observation: Observation, seq: u64, request_id: u64, ok: bool, loc: &str) -> Observation {
    observation.route_inspect_seq = seq;
    observation.route_inspect_generation = observation.route_inspect_live_generation;
    observation.route_inspect_has_terminal = true;
    observation.route_inspect_request_id = request_id;
    observation.route_inspect_ok = ok;
    observation.route_inspect_hops = vec![RouteInspectHopFact {
        loc_name: loc.into(),
    }];
    observation
}

fn restocked_from(baseline: &Observation) -> Observation {
    let mut restocked = baseline.clone();
    restocked.bank_open = true;
    restocked.bank_loaded = true;
    restocked.bank_generation = baseline.bank_generation + 1;
    restocked.item_ids.insert(LOBSTER_ID, 20);
    restocked.item_ids.insert(COINS_ID, 60);
    restocked.items.insert("Lobster".into(), 20);
    restocked.items.insert("Coins".into(), 60);
    restocked
}

#[test]
fn brimhaven_moss_inspect_v1_seed_and_fallback_cannot_pass() {
    assert_eq!(
        CoreCase::parse("brimhaven_moss_inspect_v1").unwrap(),
        CoreCase::BrimhavenMossInspectV1
    );
    assert_eq!(
        CoreCase::BrimhavenMossInspectV1.card_name(),
        "BrimhavenMossGiants"
    );
    assert!(CoreCase::BrimhavenMossInspectV1.copies_route_inspect());

    let watch = CoreWatch::default();
    watch.configure(CoreCase::BrimhavenMossInspectV1, "catalogtest");
    let baseline = brimhaven_v1_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "repeating the empty-pack bank seed must not qualify"
    );

    let mut field = baseline.clone();
    field.tile = Some(BRIMHAVEN_INSPECT_FIELD);
    watch.observe("catalogtest", field, false);
    assert!(
        watch.qualify().is_err(),
        "field arrival without restock or accepted inspect must not qualify"
    );

    let restocked = restocked_from(&baseline);
    watch.observe("catalogtest", restocked.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "restock alone is not inspect qualification"
    );

    let fallback = with_hop(restocked.clone(), 3, 1, false, "");
    let mut fallback = fallback;
    fallback.tile = Some(BRIMHAVEN_INSPECT_FIELD);
    fallback.route_inspect_reason = "route validation exhausted".into();
    watch.observe("catalogtest", fallback, false);
    assert!(
        watch.qualify().is_err(),
        "frozen fallback without accepted Barnaby inspect must not qualify"
    );

    let stale = with_hop(restocked.clone(), 0, 0, true, "Captain Barnaby");
    watch.observe("catalogtest", stale, false);
    assert!(
        watch.qualify().is_err(),
        "a stale or unpublished inspect seq cannot stand in for a fresh result"
    );
}

#[test]
fn brimhaven_moss_inspect_v1_requires_restock_then_fresh_barnaby_then_walk() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::BrimhavenMossInspectV1, "catalogtest");
    let baseline = brimhaven_v1_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let restocked = restocked_from(&baseline);
    watch.observe("catalogtest", restocked.clone(), false);

    let accepted = with_hop(restocked.clone(), 4, 7, true, "Captain Barnaby");
    watch.observe("catalogtest", accepted.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "accepted inspect without ordinary walk progress is incomplete"
    );

    let mut walked = accepted;
    walked.tile = Some((2669, 3278, 0));
    watch.observe("catalogtest", walked, false);
    watch
        .qualify()
        .expect("ordered restock, fresh Barnaby inspect, and pier-ward walk");
}

#[test]
fn route_inspect_brimhaven_v2_requires_token_then_id0_then_bank_tile() {
    assert_eq!(
        CoreCase::parse("route_inspect_brimhaven_v2_ts").unwrap(),
        CoreCase::RouteInspectBrimhavenV2
    );
    assert!(CoreCase::RouteInspectBrimhavenV2.copies_route_inspect());

    let watch = CoreWatch::default();
    watch.configure(CoreCase::RouteInspectBrimhavenV2, "catalogtest");
    let baseline = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some(BRIMHAVEN_INSPECT_PIER),
        ..Observation::default()
    };
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "pier seed must not qualify"
    );

    let token = with_hop(baseline.clone(), 2, 11, true, "Captain Barnaby");
    watch.observe("catalogtest", token.clone(), false);
    assert!(watch.qualify().is_err(), "token inspect alone is incomplete");

    let snap0 = with_hop(token.clone(), 3, 0, true, "Captain Barnaby");
    watch.observe("catalogtest", snap0.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "id0 result without distinct-tile walk is incomplete"
    );

    let mut arrived = snap0;
    arrived.tile = Some(BRIMHAVEN_INSPECT_BANK);
    watch.observe("catalogtest", arrived, false);
    watch
        .qualify()
        .expect("token, request_id 0, and actual bank tile");
}

#[test]
fn brimhaven_moss_inspect_v1_same_frame_pier_accept_cannot_pass() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::BrimhavenMossInspectV1, "catalogtest");
    let baseline = brimhaven_v1_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", restocked_from(&baseline), false);

    let mut accept_at_pier = with_hop(restocked_from(&baseline), 4, 7, true, "Captain Barnaby");
    accept_at_pier.tile = Some(BRIMHAVEN_INSPECT_PIER);
    watch.observe("catalogtest", accept_at_pier.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "same-frame accept already at the pier is not later walk progress"
    );

    let mut later = accept_at_pier;
    later.tile = Some((2675, 3275, 0));
    watch.observe("catalogtest", later, false);
    watch
        .qualify()
        .expect("a later observation with actual tile change can qualify");
}

#[test]
fn brimhaven_moss_inspect_v1_wrong_boat_and_old_generation_cannot_pass() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::BrimhavenMossInspectV1, "catalogtest");
    let baseline = brimhaven_v1_baseline();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let restocked = restocked_from(&baseline);
    watch.observe("catalogtest", restocked.clone(), false);

    let wrong_boat = with_hop(restocked.clone(), 4, 7, true, "Captain Thresnor");
    watch.observe("catalogtest", wrong_boat.clone(), false);
    let mut wrong_walk = wrong_boat;
    wrong_walk.tile = Some((2669, 3278, 0));
    watch.observe("catalogtest", wrong_walk, false);
    assert!(
        watch.qualify().is_err(),
        "wrong-boat hop cannot stand in for accepted Barnaby"
    );

    let mut stale_gen = with_hop(restocked.clone(), 9, 7, true, "Captain Barnaby");
    stale_gen.route_inspect_generation = 0;
    watch.observe("catalogtest", stale_gen.clone(), false);
    let mut stale_walk = stale_gen;
    stale_walk.tile = Some((2669, 3278, 0));
    watch.observe("catalogtest", stale_walk, false);
    assert!(
        watch.qualify().is_err(),
        "old generation after reset_inspect cannot qualify"
    );

    let id0 = with_hop(restocked.clone(), 5, 0, true, "Captain Barnaby");
    watch.observe("catalogtest", id0.clone(), false);
    let mut id0_walk = id0;
    id0_walk.tile = Some((2669, 3278, 0));
    watch.observe("catalogtest", id0_walk, false);
    assert!(
        watch.qualify().is_err(),
        "v1 requires a registered request identity, not request_id 0"
    );
}

#[test]
fn brimhaven_moss_inspect_v1_generation_zero_is_legitimate_before_reset() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::BrimhavenMossInspectV1, "catalogtest");
    let mut baseline = brimhaven_v1_baseline();
    baseline.route_inspect_live_generation = 0;
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", restocked_from(&baseline), false);

    let accepted = with_hop(restocked_from(&baseline), 1, 3, true, "Captain Barnaby");
    watch.observe("catalogtest", accepted.clone(), false);
    assert!(watch.qualify().is_err(), "accept still needs later walk");

    let mut walked = accepted;
    walked.tile = Some((2669, 3278, 0));
    watch.observe("catalogtest", walked, false);
    watch
        .qualify()
        .expect("generation 0 is a real first-session InspectNav generation");
}

#[test]
fn brimhaven_moss_inspect_v1_generation_transition_uses_new_ring_seq() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::BrimhavenMossInspectV1, "catalogtest");
    let mut baseline = brimhaven_v1_baseline();
    baseline.route_inspect_live_generation = 0;
    baseline.route_inspect_has_terminal = true;
    baseline.route_inspect_seq = 8;
    baseline.route_inspect_generation = 0;
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut restocked = restocked_from(&baseline);
    restocked.route_inspect_live_generation = 1;
    restocked.route_inspect_has_terminal = false;
    restocked.route_inspect_seq = 0;
    restocked.route_inspect_generation = 0;
    watch.observe("catalogtest", restocked.clone(), false);

    let accepted = with_hop(restocked, 1, 4, true, "Captain Barnaby");
    watch.observe("catalogtest", accepted.clone(), false);
    let mut walked = accepted;
    walked.tile = Some((2669, 3278, 0));
    watch.observe("catalogtest", walked, false);
    watch
        .qualify()
        .expect("after reset, seq 1 on the new generation is fresh even if the old ring was 8");
}

fn prayer_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3222, 3222, 0)),
        ..Observation::default()
    };
    observation.levels.insert("prayer".into(), 43);
    observation.effective_levels.insert("prayer".into(), 43);
    for index in 83..=97 {
        observation.varps.insert(index, 0);
    }
    observation
}

fn prayer_on(mut observation: Observation) -> Observation {
    observation.tick += 1;
    observation.varps.insert(97, 1);
    observation
}

fn prayer_all_off(mut observation: Observation) -> Observation {
    observation.tick += 1;
    for index in 83..=97 {
        observation.varps.insert(index, 0);
    }
    observation
}

fn prayer_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

#[test]
fn prayer_v2_requires_baseline_off_then_on_then_off_and_named_stop() {
    let case = CoreCase::parse("prayer_v2_ts").expect("named prayer v2 cell");
    assert!(case.copies_prayer_varps());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = prayer_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seeded all-off must not qualify"
    );

    let on = prayer_on(baseline.clone());
    watch.observe("catalogtest", on.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "latched ON without later all-off is incomplete"
    );

    let off = prayer_all_off(on);
    watch.observe("catalogtest", off.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "all-off without the exact helper stop is incomplete"
    );

    watch.observe(
        "catalogtest",
        prayer_stop(off, "prayer v2 qualification complete"),
        false,
    );
    watch
        .qualify()
        .expect("ordered off, observed ON, later all-off, and named stop");
}

#[test]
fn prayer_v1_stop_without_on_and_missing_varps_cannot_pass() {
    let case = CoreCase::parse("prayer_v1_ts").expect("named prayer v1 cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = prayer_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let stopped = prayer_stop(baseline.clone(), "prayer v1 qualification complete");
    watch.observe("catalogtest", stopped, false);
    assert!(
        watch.qualify().is_err(),
        "clean stop without a latched ON cannot pass"
    );

    watch.configure(case, "catalogtest");
    let mut missing = baseline.clone();
    missing.varps.remove(&90);
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing prayer varps must not look like all-off"
    );
}

#[test]
fn prayer_unready_and_outoforder_cannot_pass() {
    let case = CoreCase::parse("prayer_v2_ts").expect("named prayer v2 cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = prayer_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut no_points = prayer_ready();
    no_points.effective_levels.insert("prayer".into(), 0);
    watch.observe("catalogtest", no_points, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "zero prayer points cannot start"
    );

    watch.configure(case, "catalogtest");
    let mut low_base = prayer_ready();
    low_base.levels.insert("prayer".into(), 40);
    watch.observe("catalogtest", low_base, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "base 40 is below Protect from Melee 43"
    );

    watch.configure(case, "catalogtest");
    let baseline = prayer_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let off = prayer_all_off(baseline.clone());
    watch.observe("catalogtest", off.clone(), false);
    watch.observe(
        "catalogtest",
        prayer_stop(off, "prayer v2 qualification complete"),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "later all-off+stop without a prior ON latch is out of order"
    );
}

fn los_identity() -> LineOfSightIdentity {
    LineOfSightIdentity {
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
    }
}

fn los_open_pair() -> LineOfSightPair {
    LineOfSightPair {
        from: LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        src: 0,
        dst: 0,
        mask: 0x1000,
    }
}

fn los_blocked_pair() -> LineOfSightPair {
    LineOfSightPair {
        from: LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3202,
            z: 3201,
            level: 0,
        },
        src: 0,
        dst: LOS_V_W,
        mask: LOS_V_W,
    }
}

fn los_pair_result(pair: LineOfSightPair, v2: bool, v1: bool) -> LineOfSightPairResult {
    LineOfSightPairResult {
        from: pair.from,
        to: pair.to,
        src: pair.src,
        dst: pair.dst,
        mask: pair.mask,
        v2,
        v1,
    }
}

fn los_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.los = LineOfSightObservation {
        available: true,
        identity: los_identity(),
        here: Some(LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        }),
        here_flag: Some(0),
        ..LineOfSightObservation::default()
    };
    observation
}

fn los_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let open = los_open_pair();
    let blocked = los_blocked_pair();
    observation.los.host_open = Some(open);
    observation.los.host_blocked = Some(blocked);
    observation.los.receipt = Some(LineOfSightScriptReceipt {
        identity: los_identity(),
        here: LineOfSightHere {
            x: 3201,
            z: 3201,
            level: 0,
            flag: 0,
        },
        open: los_pair_result(open, true, true),
        blocked: los_pair_result(blocked, false, false),
    });
    observation
}

fn los_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

#[test]
fn line_of_sight_selection_uses_v_mask_not_walk_or_dest_vis() {
    let here = LineOfSightTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let vis_only = |x: i32, z: i32| {
        if x == 7 && z == 5 {
            Some(LOS_VIS_SCENERY)
        } else {
            Some(0)
        }
    };
    assert!(
        select_line_of_sight_pairs(here, vis_only).is_err(),
        "destination VIS_SCENERY alone is not a blocked pair"
    );

    let walk_dest = |x: i32, z: i32| {
        if x == 6 && z == 5 {
            Some(LOS_WALK_SCENERY)
        } else {
            Some(0)
        }
    };
    assert!(
        select_line_of_sight_pairs(here, walk_dest).is_err(),
        "destination WALK_SCENERY is not an entering V-wall"
    );

    let flags = |x: i32, z: i32| {
        if x == 6 && z == 5 {
            Some(LOS_V_W)
        } else {
            Some(0)
        }
    };
    let (open, blocked) = select_line_of_sight_pairs(here, flags).expect("V-wall pair");
    assert!(line_of_sight_pair_is_open(&open));
    assert!(line_of_sight_pair_is_blocked(&blocked));
    assert!(!line_of_sight_dest_vis_alone(&blocked));
    assert_eq!(blocked.to.x, 6);
    assert_eq!(blocked.mask, LOS_V_W);
}

#[test]
fn line_of_sight_rejects_source_scenery_blocked_despite_entering_v() {
    // Matches the R2 root disqualification shape: dest carries entering V_E
    // (and more), but source WALK_SCENERY makes the helper return false before
    // wall tracing — not a wall-specific negative.
    let scenery_src = LOS_WALK_SCENERY | 0x210080;
    let pair = LineOfSightPair {
        from: LineOfSightTile {
            x: 3217,
            z: 3217,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3216,
            z: 3217,
            level: 0,
        },
        src: scenery_src,
        dst: LOS_V_E | 0x8,
        mask: LOS_V_E,
    };
    assert!(
        !line_of_sight_pair_is_blocked(&pair),
        "source WALK_SCENERY + entering V is not a wall-specific blocked pair"
    );

    let here = LineOfSightTile {
        x: 5,
        z: 5,
        level: 0,
    };
    // Only candidate: from (6,5) has WALK_SCENERY, dest (7,5) has V_W.
    let scenery_only = |x: i32, z: i32| {
        if x == 6 && z == 5 {
            Some(LOS_WALK_SCENERY)
        } else if x == 7 && z == 5 {
            Some(LOS_V_W)
        } else {
            Some(0)
        }
    };
    assert!(
        select_line_of_sight_pairs(here, scenery_only).is_err(),
        "source-scenery-only negative must fail the fixture honestly"
    );

    // Earlier scenery+V candidate at r=1, clear-source V-wall at r=2.
    // Selector must skip the scenery source and latch the clear wall pair.
    let skip_to_clear = |x: i32, z: i32| {
        if x == 6 && z == 5 {
            Some(LOS_WALK_SCENERY)
        } else if x == 7 && z == 5 {
            Some(LOS_V_W)
        } else if x == 7 && z == 6 {
            Some(LOS_V_W)
        } else {
            Some(0)
        }
    };
    let (open, blocked) =
        select_line_of_sight_pairs(here, skip_to_clear).expect("clear-source V-wall pair");
    assert!(line_of_sight_pair_is_open(&open));
    assert!(line_of_sight_pair_is_blocked(&blocked));
    assert_eq!(blocked.src & LOS_WALK_SCENERY, 0);
    assert_eq!(blocked.from.x, 6);
    assert_eq!(blocked.from.z, 6);
    assert_eq!(blocked.to.x, 7);
    assert_eq!(blocked.to.z, 6);
    assert_eq!(blocked.mask, LOS_V_W);
}

#[test]
fn line_of_sight_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("line_of_sight_v2_ts").expect("named los cell");
    assert!(case.copies_line_of_sight());
    assert!(!case.copies_prayer_varps());
    assert!(!case.copies_route_inspect());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = los_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = los_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe("catalogtest", los_stop(joined, LOS_V2_STOP), false);
    watch
        .qualify()
        .expect("host pairs, joined receipt, and named stop");
}

#[test]
fn line_of_sight_rejects_noquery_negative_identity_and_wrong_stop() {
    let case = CoreCase::parse("line_of_sight_v2_ts").expect("named los cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = los_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        los_stop(baseline.clone(), LOS_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a post-Start query receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut negative = los_joined(baseline.clone());
    if let Some(receipt) = negative.los.receipt.as_mut() {
        receipt.open.v2 = false;
        receipt.open.v1 = false;
    }
    watch.observe("catalogtest", negative, false);
    assert!(
        watch.qualify().is_err(),
        "negative-only answers cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = los_joined(baseline.clone());
    if let Some(receipt) = mismatch.los.receipt.as_mut() {
        receipt.identity.base_x = 0;
    }
    mismatch.script_lifecycle = los_stop(mismatch.clone(), LOS_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "receipt identity mismatch cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = los_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe(
        "catalogtest",
        los_stop(joined, "some other stop"),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "wrong named stop cannot pass"
    );
}

#[test]
fn line_of_sight_rejects_walk_mask_dest_vis_unready_and_missing() {
    let case = CoreCase::parse("line_of_sight_v2_ts").expect("named los cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = los_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = los_ready();
    missing.los.available = false;
    missing.los.here_flag = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );

    watch.configure(case, "catalogtest");
    let baseline = los_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut walk_blocked = los_joined(baseline.clone());
    let walk_pair = LineOfSightPair {
        from: LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3202,
            z: 3201,
            level: 0,
        },
        src: 0,
        dst: LOS_WALK_SCENERY,
        mask: LOS_V_W,
    };
    walk_blocked.los.host_blocked = Some(walk_pair);
    if let Some(receipt) = walk_blocked.los.receipt.as_mut() {
        receipt.blocked = los_pair_result(walk_pair, false, false);
    }
    walk_blocked.script_lifecycle = los_stop(walk_blocked.clone(), LOS_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", walk_blocked, false);
    assert!(
        watch.qualify().is_err(),
        "WALK_SCENERY destination is not an entering V-wall"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut vis = los_joined(baseline.clone());
    let vis_pair = LineOfSightPair {
        from: LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3202,
            z: 3201,
            level: 0,
        },
        src: 0,
        dst: LOS_VIS_SCENERY,
        mask: LOS_V_W,
    };
    vis.los.host_blocked = Some(vis_pair);
    if let Some(receipt) = vis.los.receipt.as_mut() {
        receipt.blocked = los_pair_result(vis_pair, false, false);
    }
    vis.script_lifecycle = los_stop(vis.clone(), LOS_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", vis, false);
    assert!(
        watch.qualify().is_err(),
        "destination VIS_SCENERY alone is not a blocked pair"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut scenery_src = los_joined(baseline);
    let scenery_pair = LineOfSightPair {
        from: LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3202,
            z: 3201,
            level: 0,
        },
        // R2 shape: source WALK_SCENERY set, dest still carries entering V.
        src: LOS_WALK_SCENERY | 0x210080,
        dst: LOS_V_W | 0x8,
        mask: LOS_V_W,
    };
    scenery_src.los.host_blocked = Some(scenery_pair);
    if let Some(receipt) = scenery_src.los.receipt.as_mut() {
        receipt.blocked = los_pair_result(scenery_pair, false, false);
    }
    scenery_src.script_lifecycle =
        los_stop(scenery_src.clone(), LOS_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", scenery_src, false);
    assert!(
        watch.qualify().is_err(),
        "source WALK_SCENERY blocked pair cannot join Core even with entering V"
    );
}

#[test]
fn line_of_sight_join_then_host_drift_retains_coherent_evidence() {
    let case = CoreCase::parse("line_of_sight_v2_ts").expect("named los cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = los_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let open = los_open_pair();
    let blocked = los_blocked_pair();
    let joined = los_joined(baseline.clone());
    watch.observe("catalogtest", joined.clone(), false);

    // Later host selection / identity drift without a new joined receipt.
    let mut drifted = joined.clone();
    drifted.tick += 1;
    drifted.los.identity.base_x = 3100;
    drifted.los.host_open = Some(LineOfSightPair {
        from: LineOfSightTile {
            x: 3210,
            z: 3210,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3209,
            z: 3210,
            level: 0,
        },
        src: 0,
        dst: 0,
        mask: LOS_V_E,
    });
    drifted.los.host_blocked = Some(LineOfSightPair {
        from: LineOfSightTile {
            x: 3210,
            z: 3210,
            level: 0,
        },
        to: LineOfSightTile {
            x: 3211,
            z: 3210,
            level: 0,
        },
        src: 0,
        dst: LOS_V_W,
        mask: LOS_V_W,
    });
    // Stale paint receipt no longer joins the drifted host pairs.
    drifted.los.receipt = joined.los.receipt;
    watch.observe("catalogtest", drifted.clone(), false);

    // Named stop on a later frame that still carries drifted host selection
    // (multi-frame post-stop paint retention path). Must not freeze host=P'.
    watch.observe("catalogtest", los_stop(drifted, LOS_V2_STOP), false);
    let evidence = watch
        .qualify()
        .expect("coherent historical join + named stop must still qualify");
    let cycle = evidence
        .get("line_of_sight_cycle")
        .expect("line_of_sight_cycle in evidence");
    let host_open: LineOfSightPair =
        serde_json::from_value(cycle.get("host_open").cloned().unwrap())
            .expect("host_open");
    let host_blocked: LineOfSightPair =
        serde_json::from_value(cycle.get("host_blocked").cloned().unwrap())
            .expect("host_blocked");
    let identity: LineOfSightIdentity =
        serde_json::from_value(cycle.get("identity").cloned().unwrap())
            .expect("identity");
    let receipt: LineOfSightScriptReceipt =
        serde_json::from_value(cycle.get("receipt").cloned().unwrap())
            .expect("receipt");
    assert_eq!(host_open, open, "host_open must stay the joined witness");
    assert_eq!(
        host_blocked, blocked,
        "host_blocked must stay the joined witness"
    );
    assert_eq!(identity, los_identity(), "identity must stay the joined witness");
    assert_eq!(receipt.open.pair(), open);
    assert_eq!(receipt.blocked.pair(), blocked);
    assert_eq!(receipt.identity, identity);
}

#[test]
fn line_of_sight_rejects_mismatched_here_and_here_flag() {
    let case = CoreCase::parse("line_of_sight_v2_ts").expect("named los cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = los_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut bad_here = los_joined(baseline.clone());
    if let Some(receipt) = bad_here.los.receipt.as_mut() {
        receipt.here.x = 9999;
    }
    bad_here.script_lifecycle = los_stop(bad_here.clone(), LOS_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", bad_here, false);
    assert!(
        watch.qualify().is_err(),
        "receipt.here tile must join host here"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut bad_flag = los_joined(baseline);
    if let Some(receipt) = bad_flag.los.receipt.as_mut() {
        receipt.here.flag = 0x7f;
    }
    bad_flag.script_lifecycle = los_stop(bad_flag.clone(), LOS_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", bad_flag, false);
    assert!(
        watch.qualify().is_err(),
        "receipt.here.flag must join host here_flag"
    );
}

fn actor_identity() -> LineOfSightIdentity {
    LineOfSightIdentity {
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
    }
}

fn actor_npc() -> ActorObservationNpc {
    ActorObservationNpc {
        index: 7,
        name: Some("Goblin".into()),
        size: 4,
        tile_x: 3201,
        tile_z: 3205,
        nx: 3205,
        nz: 3201,
        level: 0,
    }
}

fn actor_receipt(npc: &ActorObservationNpc, los: bool) -> ActorObservationScriptReceipt {
    ActorObservationScriptReceipt {
        identity: actor_identity(),
        here: LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        npc: ActorObservationNpcFact {
            index: npc.index,
            name: npc.name.clone(),
            size: npc.size,
            tile: ActorObservationPoint {
                x: npc.tile_x,
                z: npc.tile_z,
            },
            network: ActorObservationPoint {
                x: npc.nx,
                z: npc.nz,
            },
            level: npc.level,
        },
        packed: ActorObservationPacked {
            size: npc.size,
            nx: npc.nx,
            nz: npc.nz,
        },
        rendered: ActorObservationPoint {
            x: npc.tile_x,
            z: npc.tile_z,
        },
        self_target: ActorObservationSelfTarget {
            kind: 1,
            index: 7,
        },
        los: ActorObservationLos { v2: los, v1: los },
    }
}

fn actor_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.actor = ActorObservation {
        available: true,
        identity: actor_identity(),
        here: Some(LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        }),
        ..ActorObservation::default()
    };
    observation
}

fn actor_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let npc = actor_npc();
    observation.actor.npc = Some(npc.clone());
    observation.actor.host_los = Some(true);
    observation.actor.self_target_kind = 1;
    observation.actor.self_target_index = 7;
    observation.actor.receipt = Some(actor_receipt(&npc, true));
    observation
}

fn actor_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

#[test]
fn actor_observation_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("actor_observation_v2_ts").expect("named actor cell");
    assert!(case.copies_actor_observation());
    assert!(!case.copies_line_of_sight());
    assert!(!case.copies_prayer_varps());
    assert!(!case.copies_route_inspect());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = actor_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = actor_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe(
        "catalogtest",
        actor_stop(joined, ACTOR_OBSERVATION_V2_STOP),
        false,
    );
    watch
        .qualify()
        .expect("host npc, joined receipt, and named stop");
}

#[test]
fn actor_observation_rejects_noquery_nonpc_identity_and_wrong_stop() {
    let case = CoreCase::parse("actor_observation_v2_ts").expect("named actor cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = actor_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        actor_stop(baseline.clone(), ACTOR_OBSERVATION_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a post-Start query receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut no_npc = actor_joined(baseline.clone());
    no_npc.actor.npc = None;
    no_npc.actor.receipt = None;
    no_npc.script_lifecycle = actor_stop(no_npc.clone(), ACTOR_OBSERVATION_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", no_npc, false);
    assert!(watch.qualify().is_err(), "no-NPC success cannot pass");

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = actor_joined(baseline.clone());
    if let Some(receipt) = mismatch.actor.receipt.as_mut() {
        receipt.npc.network.x = 0;
        receipt.packed.nx = 0;
    }
    mismatch.script_lifecycle =
        actor_stop(mismatch.clone(), ACTOR_OBSERVATION_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "Core/script packed vs rendered disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut identity = actor_joined(baseline.clone());
    if let Some(receipt) = identity.actor.receipt.as_mut() {
        receipt.identity.base_x = 0;
    }
    identity.script_lifecycle =
        actor_stop(identity.clone(), ACTOR_OBSERVATION_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", identity, false);
    assert!(
        watch.qualify().is_err(),
        "receipt identity mismatch cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = actor_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", actor_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");
}

#[test]
fn actor_observation_rejects_unready_and_missing_scene() {
    let case = CoreCase::parse("actor_observation_v2_ts").expect("named actor cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = actor_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = actor_ready();
    missing.actor.available = false;
    missing.actor.here = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );
}

#[test]
fn actor_observation_join_agrees_on_nearest_not_array_first() {
    let case = CoreCase::parse("actor_observation_v2_ts").expect("named actor cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = actor_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let array_first = ActorObservationNpc {
        index: 3,
        name: Some("Far".into()),
        size: 1,
        tile_x: 3208,
        tile_z: 3208,
        nx: 3208,
        nz: 3208,
        level: 0,
    };
    let nearest = ActorObservationNpc {
        index: 11,
        name: Some("Near".into()),
        size: 2,
        tile_x: 3202,
        tile_z: 3202,
        nx: 3203,
        nz: 3201,
        level: 0,
    };

    let mut mismatch = baseline.clone();
    mismatch.tick += 1;
    mismatch.actor.npc = Some(array_first);
    mismatch.actor.host_los = Some(true);
    mismatch.actor.self_target_kind = 1;
    mismatch.actor.self_target_index = 7;
    mismatch.actor.receipt = Some(actor_receipt(&nearest, true));
    mismatch.script_lifecycle =
        actor_stop(mismatch.clone(), ACTOR_OBSERVATION_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "array-first host npc vs nearest File receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut joined = baseline;
    joined.tick += 1;
    joined.actor.npc = Some(nearest.clone());
    joined.actor.host_los = Some(true);
    joined.actor.self_target_kind = 1;
    joined.actor.self_target_index = 7;
    joined.actor.receipt = Some(actor_receipt(&nearest, true));
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe(
        "catalogtest",
        actor_stop(joined, ACTOR_OBSERVATION_V2_STOP),
        false,
    );
    watch
        .qualify()
        .expect("File receipt and Core join agree on nearest index 11");
}

fn fight_identity() -> LineOfSightIdentity {
    LineOfSightIdentity {
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
    }
}

fn fight_npc() -> FightFieldNpc {
    FightFieldNpc {
        index: 7,
        size: 4,
        tile_x: 3201,
        tile_z: 3205,
        nx: 3205,
        nz: 3201,
        level: 0,
    }
}

fn fight_receipt(
    npc: &FightFieldNpc,
    los_network: bool,
    los_tile: bool,
) -> FightFieldScriptReceipt {
    FightFieldScriptReceipt {
        index: npc.index,
        size: npc.size,
        tile: ActorObservationPoint {
            x: npc.tile_x,
            z: npc.tile_z,
        },
        network_origin: ActorObservationPoint {
            x: npc.nx,
            z: npc.nz,
        },
        los_network,
        los_tile,
        kind: None,
        effect: None,
    }
}

fn fight_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.fight = FightFieldObservation {
        available: true,
        identity: fight_identity(),
        here: Some(LineOfSightTile {
            x: 3201,
            z: 3201,
            level: 0,
        }),
        ..FightFieldObservation::default()
    };
    observation
}

fn fight_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let npc = fight_npc();
    observation.fight.npc = Some(npc.clone());
    observation.fight.host_los_network = Some(true);
    observation.fight.host_los_tile = Some(false);
    observation.fight.receipt = Some(fight_receipt(&npc, true, false));
    observation
}

fn fight_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

#[test]
fn fight_field_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("fight_field_v2_ts").expect("named fight field cell");
    assert!(case.copies_fight_field());
    assert!(!case.copies_actor_observation());
    assert!(!case.copies_line_of_sight());
    assert!(!case.copies_prayer_varps());
    assert!(!case.copies_route_inspect());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = fight_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = fight_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe(
        "catalogtest",
        fight_stop(joined, FIGHT_FIELD_V2_STOP),
        false,
    );
    watch
        .qualify()
        .expect("host npc, joined receipt, and named stop");
}

#[test]
fn fight_field_rejects_noquery_nonpc_identity_packed_and_wrong_stop() {
    let case = CoreCase::parse("fight_field_v2_ts").expect("named fight field cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = fight_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        fight_stop(baseline.clone(), FIGHT_FIELD_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a post-Start query receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut no_npc = fight_joined(baseline.clone());
    no_npc.fight.npc = None;
    no_npc.fight.receipt = None;
    no_npc.script_lifecycle =
        fight_stop(no_npc.clone(), FIGHT_FIELD_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", no_npc, false);
    assert!(watch.qualify().is_err(), "no-NPC success cannot pass");

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = fight_joined(baseline.clone());
    if let Some(receipt) = mismatch.fight.receipt.as_mut() {
        receipt.network_origin.x = 0;
    }
    mismatch.script_lifecycle =
        fight_stop(mismatch.clone(), FIGHT_FIELD_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "Core/script packed vs rendered disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut identity = fight_joined(baseline.clone());
    if let Some(receipt) = identity.fight.receipt.as_mut() {
        receipt.index = 99;
    }
    identity.script_lifecycle =
        fight_stop(identity.clone(), FIGHT_FIELD_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", identity, false);
    assert!(
        watch.qualify().is_err(),
        "receipt identity mismatch cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = fight_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", fight_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");
}

#[test]
fn fight_field_rejects_unready_and_missing_scene() {
    let case = CoreCase::parse("fight_field_v2_ts").expect("named fight field cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = fight_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = fight_ready();
    missing.fight.available = false;
    missing.fight.here = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );
}

#[test]
fn fight_field_join_agrees_on_nearest_not_array_first() {
    let case = CoreCase::parse("fight_field_v2_ts").expect("named fight field cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = fight_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let array_first = FightFieldNpc {
        index: 3,
        size: 1,
        tile_x: 3208,
        tile_z: 3208,
        nx: 3208,
        nz: 3208,
        level: 0,
    };
    let nearest = FightFieldNpc {
        index: 11,
        size: 2,
        tile_x: 3202,
        tile_z: 3202,
        nx: 3203,
        nz: 3201,
        level: 0,
    };

    let mut mismatch = baseline.clone();
    mismatch.tick += 1;
    mismatch.fight.npc = Some(array_first);
    mismatch.fight.host_los_network = Some(true);
    mismatch.fight.host_los_tile = Some(false);
    mismatch.fight.receipt = Some(fight_receipt(&nearest, true, false));
    mismatch.script_lifecycle =
        fight_stop(mismatch.clone(), FIGHT_FIELD_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "array-first host npc vs nearest File receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut joined = baseline;
    joined.tick += 1;
    joined.fight.npc = Some(nearest.clone());
    joined.fight.host_los_network = Some(true);
    joined.fight.host_los_tile = Some(false);
    joined.fight.receipt = Some(fight_receipt(&nearest, true, false));
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe(
        "catalogtest",
        fight_stop(joined, FIGHT_FIELD_V2_STOP),
        false,
    );
    watch
        .qualify()
        .expect("File receipt and Core join agree on nearest index 11");
}

#[test]
fn fight_field_rejects_forged_npc_attack_step() {
    let case = CoreCase::parse("fight_field_v2_ts").expect("named fight field cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = fight_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut npc_kind = fight_joined(baseline.clone());
    if let Some(receipt) = npc_kind.fight.receipt.as_mut() {
        receipt.kind = Some("npc".into());
    }
    npc_kind.script_lifecycle =
        fight_stop(npc_kind.clone(), FIGHT_FIELD_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", npc_kind, false);
    assert!(
        watch.qualify().is_err(),
        "forged npc fightNext step cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut attack = fight_joined(baseline);
    if let Some(receipt) = attack.fight.receipt.as_mut() {
        receipt.effect = Some("Attack".into());
    }
    attack.script_lifecycle = fight_stop(attack.clone(), FIGHT_FIELD_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", attack, false);
    assert!(
        watch.qualify().is_err(),
        "forged Attack effect cannot pass"
    );
}

fn hold_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3201,
        z: 3201,
        level: 0,
    }
}

fn hold_dest() -> LineOfSightTile {
    LineOfSightTile {
        x: 3203,
        z: 3201,
        level: 0,
    }
}

fn hold_receipt(here: LineOfSightTile, dest: LineOfSightTile, kind: &str) -> HoldSpotScriptReceipt {
    HoldSpotScriptReceipt {
        here,
        dest,
        kind: kind.into(),
    }
}

fn hold_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.hold = HoldSpotObservation {
        available: true,
        here: Some(hold_here()),
        ..HoldSpotObservation::default()
    };
    observation
}

fn hold_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = hold_here();
    let dest = hold_dest();
    observation.hold.here = Some(here);
    observation.hold.dest = Some(dest);
    observation.hold.receipt = Some(hold_receipt(here, dest, "walk"));
    observation
}

fn hold_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

#[test]
fn hold_spot_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("hold_spot_v2_ts").expect("named hold spot cell");
    assert!(case.copies_hold_spot());
    assert!(!case.copies_fight_field());
    assert!(!case.copies_actor_observation());
    assert!(!case.copies_line_of_sight());
    assert!(!case.copies_prayer_varps());
    assert!(!case.copies_route_inspect());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = hold_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = hold_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe("catalogtest", hold_stop(joined, HOLD_SPOT_V2_STOP), false);
    watch
        .qualify()
        .expect("host here, joined dest/kind receipt, and named stop");
}

#[test]
fn hold_spot_rejects_noquery_nodest_already_on_dest_and_wrong_stop() {
    let case = CoreCase::parse("hold_spot_v2_ts").expect("named hold spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = hold_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        hold_stop(baseline.clone(), HOLD_SPOT_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a post-Start query receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut no_dest = hold_joined(baseline.clone());
    no_dest.hold.dest = None;
    no_dest.hold.receipt = None;
    no_dest.script_lifecycle = hold_stop(no_dest.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", no_dest, false);
    assert!(watch.qualify().is_err(), "no-dest success cannot pass");

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut already = hold_joined(baseline.clone());
    let here = hold_here();
    already.hold.dest = Some(here);
    if let Some(receipt) = already.hold.receipt.as_mut() {
        receipt.dest = here;
    }
    already.script_lifecycle = hold_stop(already.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", already, false);
    assert!(
        watch.qualify().is_err(),
        "already-on-dest success cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = hold_joined(baseline.clone());
    if let Some(receipt) = mismatch.hold.receipt.as_mut() {
        receipt.here.x = 0;
    }
    mismatch.script_lifecycle = hold_stop(mismatch.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "Core/script here disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = hold_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", hold_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");
}

#[test]
fn hold_spot_rejects_unready_and_missing_scene() {
    let case = CoreCase::parse("hold_spot_v2_ts").expect("named hold spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = hold_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = hold_ready();
    missing.hold.available = false;
    missing.hold.here = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );
}

#[test]
fn hold_spot_join_agrees_on_here_dest_not_host_here_alone() {
    let case = CoreCase::parse("hold_spot_v2_ts").expect("named hold spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = hold_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut mismatch = hold_joined(baseline.clone());
    if let Some(receipt) = mismatch.hold.receipt.as_mut() {
        receipt.dest = LineOfSightTile {
            x: 3210,
            z: 3210,
            level: 0,
        };
    }
    mismatch.script_lifecycle = hold_stop(mismatch.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "host dest vs File dest disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = hold_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", hold_stop(joined, HOLD_SPOT_V2_STOP), false);
    watch
        .qualify()
        .expect("File receipt and Core join agree on here and dest");
}

#[test]
fn hold_spot_rejects_forged_walk_to_npc_attack() {
    let case = CoreCase::parse("hold_spot_v2_ts").expect("named hold spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = hold_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut walk_to = hold_joined(baseline.clone());
    if let Some(receipt) = walk_to.hold.receipt.as_mut() {
        receipt.kind = "walk-to".into();
    }
    walk_to.script_lifecycle = hold_stop(walk_to.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", walk_to, false);
    assert!(
        watch.qualify().is_err(),
        "forged walk-to holdNext step cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut npc_kind = hold_joined(baseline.clone());
    if let Some(receipt) = npc_kind.hold.receipt.as_mut() {
        receipt.kind = "npc".into();
    }
    npc_kind.script_lifecycle = hold_stop(npc_kind.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", npc_kind, false);
    assert!(
        watch.qualify().is_err(),
        "forged npc holdNext step cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut attack = hold_joined(baseline);
    if let Some(receipt) = attack.hold.receipt.as_mut() {
        receipt.kind = "Attack".into();
    }
    attack.script_lifecycle = hold_stop(attack.clone(), HOLD_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", attack, false);
    assert!(
        watch.qualify().is_err(),
        "forged Attack holdNext step cannot pass"
    );
}

fn retreat_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3201,
        z: 3201,
        level: 0,
    }
}

fn retreat_dest() -> LineOfSightTile {
    LineOfSightTile {
        x: 3203,
        z: 3201,
        level: 0,
    }
}

fn retreat_receipt(
    here: LineOfSightTile,
    dest: LineOfSightTile,
    kind: &str,
) -> RetreatSpotScriptReceipt {
    RetreatSpotScriptReceipt {
        here,
        dest,
        kind: kind.into(),
    }
}

fn retreat_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.retreat = RetreatSpotObservation {
        available: true,
        here: Some(retreat_here()),
        ..RetreatSpotObservation::default()
    };
    observation
}

fn retreat_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = retreat_here();
    let dest = retreat_dest();
    observation.retreat.here = Some(here);
    observation.retreat.dest = Some(dest);
    observation.retreat.receipt = Some(retreat_receipt(here, dest, "walk-to"));
    observation
}

fn retreat_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

#[test]
fn retreat_spot_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("retreat_spot_v2_ts").expect("named retreat spot cell");
    assert!(case.copies_retreat_spot());
    assert!(!case.copies_hold_spot());
    assert!(!case.copies_fight_field());
    assert!(!case.copies_actor_observation());
    assert!(!case.copies_line_of_sight());
    assert!(!case.copies_prayer_varps());
    assert!(!case.copies_route_inspect());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = retreat_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = retreat_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe(
        "catalogtest",
        retreat_stop(joined, RETREAT_SPOT_V2_STOP),
        false,
    );
    watch
        .qualify()
        .expect("host here, joined dest/kind receipt, and named stop");
}

#[test]
fn retreat_spot_rejects_noquery_nodest_already_on_dest_and_wrong_stop() {
    let case = CoreCase::parse("retreat_spot_v2_ts").expect("named retreat spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = retreat_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        retreat_stop(baseline.clone(), RETREAT_SPOT_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a post-Start query receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut no_dest = retreat_joined(baseline.clone());
    no_dest.retreat.dest = None;
    no_dest.retreat.receipt = None;
    no_dest.script_lifecycle =
        retreat_stop(no_dest.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", no_dest, false);
    assert!(watch.qualify().is_err(), "no-dest success cannot pass");

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut already = retreat_joined(baseline.clone());
    let here = retreat_here();
    already.retreat.dest = Some(here);
    if let Some(receipt) = already.retreat.receipt.as_mut() {
        receipt.dest = here;
    }
    already.script_lifecycle =
        retreat_stop(already.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", already, false);
    assert!(
        watch.qualify().is_err(),
        "already-on-dest success cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = retreat_joined(baseline.clone());
    if let Some(receipt) = mismatch.retreat.receipt.as_mut() {
        receipt.here.x = 0;
    }
    mismatch.script_lifecycle =
        retreat_stop(mismatch.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "Core/script here disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = retreat_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe(
        "catalogtest",
        retreat_stop(joined, "some other stop"),
        false,
    );
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");
}

#[test]
fn retreat_spot_rejects_unready_and_missing_scene() {
    let case = CoreCase::parse("retreat_spot_v2_ts").expect("named retreat spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = retreat_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = retreat_ready();
    missing.retreat.available = false;
    missing.retreat.here = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );
}

#[test]
fn retreat_spot_join_agrees_on_here_dest_not_host_here_alone() {
    let case = CoreCase::parse("retreat_spot_v2_ts").expect("named retreat spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = retreat_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut mismatch = retreat_joined(baseline.clone());
    if let Some(receipt) = mismatch.retreat.receipt.as_mut() {
        receipt.dest = LineOfSightTile {
            x: 3210,
            z: 3210,
            level: 0,
        };
    }
    mismatch.script_lifecycle =
        retreat_stop(mismatch.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "host dest vs File dest disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = retreat_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe(
        "catalogtest",
        retreat_stop(joined, RETREAT_SPOT_V2_STOP),
        false,
    );
    watch
        .qualify()
        .expect("File receipt and Core join agree on here and dest");
}

#[test]
fn retreat_spot_rejects_forged_walk_npc_attack() {
    let case = CoreCase::parse("retreat_spot_v2_ts").expect("named retreat spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = retreat_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut walk_kind = retreat_joined(baseline.clone());
    if let Some(receipt) = walk_kind.retreat.receipt.as_mut() {
        receipt.kind = "walk".into();
    }
    walk_kind.script_lifecycle =
        retreat_stop(walk_kind.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", walk_kind, false);
    assert!(
        watch.qualify().is_err(),
        "forged walk retreatNext step cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut npc_kind = retreat_joined(baseline.clone());
    if let Some(receipt) = npc_kind.retreat.receipt.as_mut() {
        receipt.kind = "npc".into();
    }
    npc_kind.script_lifecycle =
        retreat_stop(npc_kind.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", npc_kind, false);
    assert!(
        watch.qualify().is_err(),
        "forged npc retreatNext step cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut attack = retreat_joined(baseline);
    if let Some(receipt) = attack.retreat.receipt.as_mut() {
        receipt.kind = "Attack".into();
    }
    attack.script_lifecycle =
        retreat_stop(attack.clone(), RETREAT_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", attack, false);
    assert!(
        watch.qualify().is_err(),
        "forged Attack retreatNext step cannot pass"
    );
}

fn walk_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3201,
        z: 3201,
        level: 0,
    }
}

fn walk_dest() -> LineOfSightTile {
    LineOfSightTile {
        x: 3214,
        z: 3201,
        level: 0,
    }
}

fn walk_receipt(here: LineOfSightTile, dest: LineOfSightTile, kind: &str) -> WalkSpotScriptReceipt {
    WalkSpotScriptReceipt {
        here,
        dest,
        kind: kind.into(),
    }
}

fn walk_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.walk = WalkSpotObservation {
        available: true,
        here: Some(walk_here()),
        ..WalkSpotObservation::default()
    };
    observation
}

fn walk_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = walk_here();
    let dest = walk_dest();
    observation.walk.here = Some(here);
    observation.walk.dest = Some(dest);
    observation.walk.receipt = Some(walk_receipt(here, dest, "walk"));
    observation
}

fn walk_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

fn walk_with_dest(observation: Observation, dest: LineOfSightTile, kind: &str) -> Observation {
    let mut joined = walk_joined(observation);
    joined.walk.dest = Some(dest);
    if let Some(receipt) = joined.walk.receipt.as_mut() {
        receipt.dest = dest;
        receipt.kind = kind.into();
    }
    joined
}

#[test]
fn walk_spot_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("walk_spot_v2_ts").expect("named walk spot cell");
    assert!(case.copies_walk_spot());
    assert!(!case.copies_hold_spot());
    assert!(!case.copies_retreat_spot());
    assert!(!case.copies_fight_field());
    assert!(!case.copies_actor_observation());
    assert!(!case.copies_line_of_sight());
    assert!(!case.copies_prayer_varps());
    assert!(!case.copies_route_inspect());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = walk_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = walk_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe("catalogtest", walk_stop(joined, WALK_SPOT_V2_STOP), false);
    watch
        .qualify()
        .expect("host here, joined dest/kind receipt, and named stop");
}

#[test]
fn walk_spot_rejects_noquery_nodest_within_12_and_wrong_stop() {
    let case = CoreCase::parse("walk_spot_v2_ts").expect("named walk spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = walk_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        walk_stop(baseline.clone(), WALK_SPOT_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a post-Start query receipt cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut no_dest = walk_joined(baseline.clone());
    no_dest.walk.dest = None;
    no_dest.walk.receipt = None;
    no_dest.script_lifecycle = walk_stop(no_dest.clone(), WALK_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", no_dest, false);
    assert!(watch.qualify().is_err(), "no-dest success cannot pass");

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let hold_back = walk_with_dest(
        baseline.clone(),
        LineOfSightTile {
            x: 3203,
            z: 3201,
            level: 0,
        },
        "walk",
    );
    let hold_back = walk_stop(hold_back, WALK_SPOT_V2_STOP);
    watch.observe("catalogtest", hold_back, false);
    assert!(
        watch.qualify().is_err(),
        "Chebyshev 2 Hold walk-back cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let within_12 = walk_with_dest(
        baseline.clone(),
        LineOfSightTile {
            x: 3213,
            z: 3201,
            level: 0,
        },
        "walk",
    );
    let within_12 = walk_stop(within_12, WALK_SPOT_V2_STOP);
    watch.observe("catalogtest", within_12, false);
    assert!(
        watch.qualify().is_err(),
        "Chebyshev 12 is Hold's window and cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let already = walk_stop(
        walk_with_dest(baseline.clone(), walk_here(), "walk"),
        WALK_SPOT_V2_STOP,
    );
    watch.observe("catalogtest", already, false);
    assert!(
        watch.qualify().is_err(),
        "already-on-dest success cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = walk_joined(baseline.clone());
    if let Some(receipt) = mismatch.walk.receipt.as_mut() {
        receipt.here.x = 0;
    }
    mismatch.script_lifecycle = walk_stop(mismatch.clone(), WALK_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "Core/script here disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = walk_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", walk_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");
}

#[test]
fn walk_spot_rejects_unready_and_missing_scene() {
    let case = CoreCase::parse("walk_spot_v2_ts").expect("named walk spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = walk_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = walk_ready();
    missing.walk.available = false;
    missing.walk.here = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );
}

#[test]
fn walk_spot_join_agrees_on_here_dest_not_host_here_alone() {
    let case = CoreCase::parse("walk_spot_v2_ts").expect("named walk spot cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = walk_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();

    let mut mismatch = walk_joined(baseline.clone());
    if let Some(receipt) = mismatch.walk.receipt.as_mut() {
        receipt.dest = LineOfSightTile {
            x: 3221,
            z: 3201,
            level: 0,
        };
    }
    mismatch.script_lifecycle = walk_stop(mismatch.clone(), WALK_SPOT_V2_STOP).script_lifecycle;
    watch.observe("catalogtest", mismatch, false);
    assert!(
        watch.qualify().is_err(),
        "host dest vs File dest disagreement cannot pass"
    );

    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = walk_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", walk_stop(joined, WALK_SPOT_V2_STOP), false);
    watch
        .qualify()
        .expect("File receipt and Core join agree on here and dest");
}

#[test]
fn walk_spot_rejects_forged_walk_to_walk_near_npc_attack_and_set_safespot() {
    let case = CoreCase::parse("walk_spot_v2_ts").expect("named walk spot cell");
    let baseline = walk_ready();
    for (kind, why) in [
        ("walk-to", "forged walk-to cannot pass"),
        ("walk-near", "forged walk-near cannot pass"),
        ("npc", "forged npc cannot pass"),
        ("Attack", "forged Attack cannot pass"),
        ("set-safespot", "forged set-safespot cannot pass"),
        ("status", "status preamble is not the walk command"),
    ] {
        let watch = CoreWatch::default();
        watch.configure(case, "catalogtest");
        watch.observe("catalogtest", baseline.clone(), false);
        watch.begin_start("catalogtest").unwrap();
        let forged = walk_stop(walk_with_dest(baseline.clone(), walk_dest(), kind), WALK_SPOT_V2_STOP);
        watch.observe("catalogtest", forged, false);
        assert!(watch.qualify().is_err(), "{why}");
    }
}

fn enter_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3201,
        z: 3201,
        level: 0,
    }
}

fn enter_approach() -> LineOfSightTile {
    LineOfSightTile {
        x: 3209,
        z: 3201,
        level: 0,
    }
}

fn enter_box() -> EnterLairBox {
    EnterLairBox {
        min_x: 3209,
        max_x: 3217,
        min_z: 3201,
        max_z: 3201,
        level: 0,
    }
}

fn enter_receipt(
    here: LineOfSightTile,
    approach: LineOfSightTile,
    kind: &str,
    area: EnterLairBox,
) -> EnterLairScriptReceipt {
    EnterLairScriptReceipt {
        here,
        approach,
        kind: kind.into(),
        discriminator: "gateless".into(),
        radius: 0,
        allow_teleports: false,
        allow_wilderness: false,
        allow_bank_fetch: false,
        area,
        key: "enter-lair".into(),
    }
}

fn enter_ready() -> Observation {
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((3201, 3201, 0)),
        ..Observation::default()
    };
    observation.enter = EnterLairObservation {
        available: true,
        here: Some(enter_here()),
        ..EnterLairObservation::default()
    };
    observation
}

fn enter_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = enter_here();
    let approach = enter_approach();
    let area = enter_box();
    let receipt = enter_receipt(here, approach, "walk", area);
    observation.enter.here = Some(here);
    observation.enter.approach = Some(approach);
    observation.enter.radius = Some(receipt.radius);
    observation.enter.allow_teleports = Some(receipt.allow_teleports);
    observation.enter.allow_wilderness = Some(receipt.allow_wilderness);
    observation.enter.allow_bank_fetch = Some(receipt.allow_bank_fetch);
    observation.enter.area = Some(area);
    observation.enter.receipt = Some(receipt);
    observation
}

fn enter_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

fn enter_with(
    observation: Observation,
    approach: LineOfSightTile,
    area: EnterLairBox,
    kind: &str,
) -> Observation {
    let mut joined = enter_joined(observation);
    joined.enter.approach = Some(approach);
    joined.enter.area = Some(area);
    if let Some(receipt) = joined.enter.receipt.as_mut() {
        receipt.approach = approach;
        receipt.area = area;
        receipt.kind = kind.into();
    }
    joined
}

#[test]
fn enter_lair_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("enter_lair_v2_ts").expect("named enter lair cell");
    assert!(case.copies_enter_lair());
    assert!(!case.copies_walk_spot());
    assert!(!case.copies_hold_spot());
    assert!(!case.copies_retreat_spot());
    assert!(!case.copies_fight_field());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = enter_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );

    let joined = enter_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );

    watch.observe("catalogtest", enter_stop(joined, ENTER_LAIR_V2_STOP), false);
    watch
        .qualify()
        .expect("host here, joined approach, gateless walk radius 0, and named stop");
}

#[test]
fn enter_lair_accepts_chebyshev_12_and_rejects_walk_gate_and_hold_back() {
    let case = CoreCase::parse("enter_lair_v2_ts").expect("named enter lair cell");
    let baseline = enter_ready();

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let twelve = enter_with(
        baseline.clone(),
        LineOfSightTile {
            x: 3213,
            z: 3201,
            level: 0,
        },
        enter_box(),
        "walk",
    );
    watch.observe("catalogtest", enter_stop(twelve, ENTER_LAIR_V2_STOP), false);
    watch
        .qualify()
        .expect("Chebyshev 12 is inside 8-16 and must not copy Walk's > 12 gate");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let hold_back = enter_with(
        baseline.clone(),
        LineOfSightTile {
            x: 3203,
            z: 3201,
            level: 0,
        },
        EnterLairBox {
            min_x: 3203,
            max_x: 3210,
            min_z: 3201,
            max_z: 3201,
            level: 0,
        },
        "walk",
    );
    watch.observe("catalogtest", enter_stop(hold_back, ENTER_LAIR_V2_STOP), false);
    assert!(
        watch.qualify().is_err(),
        "Chebyshev 2 Hold walk-back cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let skip = enter_with(
        baseline,
        LineOfSightTile {
            x: 3202,
            z: 3201,
            level: 0,
        },
        EnterLairBox {
            min_x: 3202,
            max_x: 3210,
            min_z: 3201,
            max_z: 3201,
            level: 0,
        },
        "walk",
    );
    watch.observe("catalogtest", enter_stop(skip, ENTER_LAIR_V2_STOP), false);
    assert!(
        watch.qualify().is_err(),
        "Chebyshev 1 is the approach skip and cannot pass"
    );
}

#[test]
fn enter_lair_rejects_in_area_missing_receipt_flags_and_kbd() {
    let case = CoreCase::parse("enter_lair_v2_ts").expect("named enter lair cell");
    let baseline = enter_ready();

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        enter_stop(baseline.clone(), ENTER_LAIR_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "named stop without a receipt cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut inside = enter_joined(baseline.clone());
    let inside_box = EnterLairBox {
        min_x: 3190,
        max_x: 3220,
        min_z: 3190,
        max_z: 3220,
        level: 0,
    };
    inside.enter.area = Some(inside_box);
    if let Some(receipt) = inside.enter.receipt.as_mut() {
        receipt.area = inside_box;
    }
    watch.observe("catalogtest", enter_stop(inside, ENTER_LAIR_V2_STOP), false);
    assert!(
        watch.qualify().is_err(),
        "already-inArea success cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut flagged = enter_joined(baseline.clone());
    if let Some(receipt) = flagged.enter.receipt.as_mut() {
        receipt.allow_teleports = true;
    }
    flagged.enter.allow_teleports = Some(true);
    watch.observe("catalogtest", enter_stop(flagged, ENTER_LAIR_V2_STOP), false);
    assert!(
        watch.qualify().is_err(),
        "allow_teleports true cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut radius = enter_joined(baseline.clone());
    if let Some(receipt) = radius.enter.receipt.as_mut() {
        receipt.radius = 2;
    }
    radius.enter.radius = Some(2);
    watch.observe("catalogtest", enter_stop(radius, ENTER_LAIR_V2_STOP), false);
    assert!(watch.qualify().is_err(), "nonzero radius cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut kbd = enter_joined(baseline.clone());
    if let Some(receipt) = kbd.enter.receipt.as_mut() {
        receipt.key = "kbd-lair".into();
    }
    watch.observe("catalogtest", enter_stop(kbd, ENTER_LAIR_V2_STOP), false);
    assert!(watch.qualify().is_err(), "kbd-lair cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let kbd_tile = enter_with(
        baseline,
        LineOfSightTile {
            x: 3017,
            z: 3849,
            level: 0,
        },
        EnterLairBox {
            min_x: 3017,
            max_x: 3033,
            min_z: 3849,
            max_z: 3849,
            level: 0,
        },
        "walk",
    );
    let mut kbd_tile = kbd_tile;
    kbd_tile.enter.here = Some(LineOfSightTile {
        x: 3009,
        z: 3849,
        level: 0,
    });
    kbd_tile.tile = Some((3009, 3849, 0));
    if let Some(receipt) = kbd_tile.enter.receipt.as_mut() {
        receipt.here = LineOfSightTile {
            x: 3009,
            z: 3849,
            level: 0,
        };
    }
    watch.observe("catalogtest", enter_stop(kbd_tile, ENTER_LAIR_V2_STOP), false);
    assert!(
        watch.qualify().is_err(),
        "(3017, 3849) cannot pass"
    );
}

#[test]
fn enter_lair_rejects_forged_kinds_and_disagreement() {
    let case = CoreCase::parse("enter_lair_v2_ts").expect("named enter lair cell");
    let baseline = enter_ready();
    for (kind, why) in [
        ("walk-near", "forged walk-near cannot pass"),
        ("walk-to", "forged walk-to cannot pass"),
        ("npc", "forged npc cannot pass"),
        ("loc", "forged loc cannot pass"),
        ("use-on", "forged use-on cannot pass"),
        ("bank-open", "forged bank-open cannot pass"),
        ("answer", "forged answer cannot pass"),
        ("Attack", "forged Attack cannot pass"),
    ] {
        let watch = CoreWatch::default();
        watch.configure(case, "catalogtest");
        watch.observe("catalogtest", baseline.clone(), false);
        watch.begin_start("catalogtest").unwrap();
        let forged = enter_stop(
            enter_with(baseline.clone(), enter_approach(), enter_box(), kind),
            ENTER_LAIR_V2_STOP,
        );
        watch.observe("catalogtest", forged, false);
        assert!(watch.qualify().is_err(), "{why}");
    }

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut mismatch = enter_joined(baseline.clone());
    if let Some(receipt) = mismatch.enter.receipt.as_mut() {
        receipt.approach.x = 3214;
    }
    watch.observe("catalogtest", enter_stop(mismatch, ENTER_LAIR_V2_STOP), false);
    assert!(
        watch.qualify().is_err(),
        "host approach vs File approach disagreement cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = enter_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", enter_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");
}

#[test]
fn enter_lair_rejects_unready_scene() {
    let case = CoreCase::parse("enter_lair_v2_ts").expect("named enter lair cell");
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut unready = enter_ready();
    unready.scene_state = 1;
    watch.observe("catalogtest", unready, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "scene_state!=2 is unready"
    );

    watch.configure(case, "catalogtest");
    let mut missing = enter_ready();
    missing.enter.available = false;
    missing.enter.here = None;
    watch.observe("catalogtest", missing, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "missing SceneView cannot start"
    );
}

fn leave_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3201,
        z: 3201,
        level: 0,
    }
}

fn leave_walk_out() -> LineOfSightTile {
    LineOfSightTile {
        x: 3209,
        z: 3201,
        level: 0,
    }
}

fn leave_box() -> LeaveLairBox {
    LeaveLairBox {
        min_x: 3199,
        max_x: 3203,
        min_z: 3199,
        max_z: 3203,
        level: 0,
    }
}

fn leave_receipt(
    here: LineOfSightTile,
    walk_out: LineOfSightTile,
    kind: &str,
) -> LeaveLairScriptReceipt {
    LeaveLairScriptReceipt {
        here,
        walk_out,
        kind: kind.into(),
        radius: 3,
        discriminator: "gateless".into(),
    }
}

fn leave_ready() -> Observation {
    let here = leave_here();
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((here.x, here.z, here.level)),
        ..Observation::default()
    };
    observation.leave = LeaveLairObservation {
        available: true,
        here: Some(here),
        area: Some(leave_box()),
        ..LeaveLairObservation::default()
    };
    observation
}

fn leave_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = leave_here();
    let walk_out = leave_walk_out();
    let receipt = leave_receipt(here, walk_out, "walk-near");
    observation.leave.here = Some(here);
    observation.leave.walk_out = Some(walk_out);
    observation.leave.radius = Some(receipt.radius);
    observation.leave.area = Some(leave_box());
    observation.leave.receipt = Some(receipt);
    observation
}

fn leave_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

fn leave_with(
    observation: Observation,
    walk_out: LineOfSightTile,
    area: LeaveLairBox,
    kind: &str,
) -> Observation {
    let mut joined = leave_joined(observation);
    joined.leave.walk_out = Some(walk_out);
    joined.leave.area = Some(area);
    if let Some(receipt) = joined.leave.receipt.as_mut() {
        receipt.walk_out = walk_out;
        receipt.kind = kind.into();
    }
    joined
}

#[test]
fn leave_lair_v2_requires_joined_receipt_and_named_stop() {
    let case = CoreCase::parse("leave_lair_v2_ts").expect("named leave lair cell");
    assert!(case.copies_leave_lair());
    assert!(!case.copies_enter_lair());
    assert!(!case.copies_walk_spot());
    assert!(!case.copies_hold_spot());
    assert!(!case.copies_retreat_spot());
    assert!(!case.copies_fight_field());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = leave_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );
    let joined = leave_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );
    watch.observe("catalogtest", leave_stop(joined, LEAVE_LAIR_V2_STOP), false);
    watch
        .qualify()
        .expect("gateless walk-near radius 3 and named stop");
}

#[test]
fn leave_lair_rejects_hold_range_kinds_claim_and_forbidden_paint() {
    let case = CoreCase::parse("leave_lair_v2_ts").expect("named leave lair cell");
    let baseline = leave_ready();

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let hold = leave_with(
        baseline.clone(),
        LineOfSightTile {
            x: 3203,
            z: 3201,
            level: 0,
        },
        leave_box(),
        "walk-near",
    );
    watch.observe("catalogtest", leave_stop(hold, LEAVE_LAIR_V2_STOP), false);
    assert!(watch.qualify().is_err(), "Chebyshev 2 cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let twelve = leave_with(
        baseline.clone(),
        LineOfSightTile {
            x: 3213,
            z: 3201,
            level: 0,
        },
        leave_box(),
        "walk-near",
    );
    watch.observe("catalogtest", leave_stop(twelve, LEAVE_LAIR_V2_STOP), false);
    watch
        .qualify()
        .expect("Chebyshev 12 must not copy Walk's > 12 gate");

    for kind in ["walk", "teleport", "loc", "yield", "aborted", "kbd"] {
        let watch = CoreWatch::default();
        watch.configure(case, "catalogtest");
        watch.observe("catalogtest", baseline.clone(), false);
        watch.begin_start("catalogtest").unwrap();
        let forged = leave_stop(
            leave_with(baseline.clone(), leave_walk_out(), leave_box(), kind),
            LEAVE_LAIR_V2_STOP,
        );
        watch.observe("catalogtest", forged, false);
        assert!(watch.qualify().is_err(), "{kind} cannot pass");
    }

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        leave_stop(baseline.clone(), LEAVE_LAIR_V2_STOP),
        false,
    );
    assert!(watch.qualify().is_err(), "missing receipt cannot pass");

    let here = leave_here();
    let walk_out = leave_walk_out();
    let ok = format!(
        "{LEAVE_LAIR_RECEIPT_PREFIX}{{\"here\":{{\"x\":{},\"z\":{},\"level\":0}},\"walkOut\":{{\"x\":{},\"z\":{},\"level\":0}},\"kind\":\"walk-near\",\"radius\":3,\"discriminator\":\"gateless\"}}",
        here.x, here.z, walk_out.x, walk_out.z
    );
    assert!(parse_leave_lair_receipt_line(&ok).is_some());
    assert!(parse_leave_lair_receipt_line(&format!("{ok} !inArea")).is_none());
    assert!(parse_leave_lair_receipt_line(
        &ok.replace("\"radius\":3", "\"radius\":3,\"inArea\":false")
    )
    .is_none());
    assert!(parse_leave_lair_receipt_line(
        &ok.replace("\"radius\":3", "\"radius\":3,\"key\":\"kbd-lair\"")
    )
    .is_none());
    assert!(parse_leave_lair_receipt_line(&ok.replace("\"radius\":3", "\"radius\":3,\"locId\":1765"))
        .is_none());
    assert!(parse_leave_lair_receipt_line(
        &ok.replace("\"radius\":3", "\"radius\":3,\"allow_teleports\":false")
    )
    .is_none());
}

#[test]
fn leave_lair_rejects_wrong_stop_box_and_forbidden_tiles() {
    let case = CoreCase::parse("leave_lair_v2_ts").expect("named leave lair cell");
    let baseline = leave_ready();

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = leave_joined(baseline.clone());
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", leave_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let outside_here = LeaveLairBox {
        min_x: 4000,
        max_x: 4010,
        min_z: 4000,
        max_z: 4010,
        level: 0,
    };
    let outside = leave_with(baseline.clone(), leave_walk_out(), outside_here, "walk-near");
    watch.observe("catalogtest", leave_stop(outside, LEAVE_LAIR_V2_STOP), false);
    assert!(watch.qualify().is_err(), "here outside the box cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let contains_out = LeaveLairBox {
        min_x: 3100,
        max_x: 3300,
        min_z: 3100,
        max_z: 3300,
        level: 0,
    };
    let inside_out = leave_with(baseline.clone(), leave_walk_out(), contains_out, "walk-near");
    watch.observe("catalogtest", leave_stop(inside_out, LEAVE_LAIR_V2_STOP), false);
    assert!(watch.qualify().is_err(), "walkOut inside the box cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut edge = leave_joined(baseline);
    let edgeville = LineOfSightTile {
        x: 3094,
        z: 3493,
        level: 0,
    };
    edge.tile = Some((edgeville.x, edgeville.z, edgeville.level));
    edge.leave.here = Some(edgeville);
    edge.leave.area = Some(LeaveLairBox {
        min_x: edgeville.x - 2,
        max_x: edgeville.x + 2,
        min_z: edgeville.z - 2,
        max_z: edgeville.z + 2,
        level: 0,
    });
    edge.leave.walk_out = Some(LineOfSightTile {
        x: edgeville.x + 8,
        z: edgeville.z,
        level: 0,
    });
    if let Some(receipt) = edge.leave.receipt.as_mut() {
        receipt.here = edgeville;
        receipt.walk_out = edge.leave.walk_out.unwrap();
    }
    watch.observe("catalogtest", leave_stop(edge, LEAVE_LAIR_V2_STOP), false);
    assert!(watch.qualify().is_err(), "Edgeville cannot pass");
}

fn acquire_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3222,
        z: 3218,
        level: 0,
    }
}

fn acquire_receipt(here: LineOfSightTile, dest: LineOfSightTile, kind: &str) -> AcquireKeyScriptReceipt {
    AcquireKeyScriptReceipt {
        here,
        dest,
        kind: kind.into(),
        radius: 1,
        discriminator: "corridor".into(),
    }
}

fn acquire_ready() -> Observation {
    let here = acquire_here();
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((here.x, here.z, here.level)),
        ..Observation::default()
    };
    observation.acquire = AcquireKeyObservation {
        available: true,
        here: Some(here),
        cell: Some(ACQUIRE_KEY_CELL),
        boxes: vec![ACQUIRE_KEY_LAIR],
        ..AcquireKeyObservation::default()
    };
    observation
}

fn acquire_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = acquire_here();
    let dest = ACQUIRE_KEY_DEST;
    let receipt = acquire_receipt(here, dest, "walk-near");
    observation.tile = Some((here.x, here.z, here.level));
    observation.acquire.here = Some(here);
    observation.acquire.dest = Some(dest);
    observation.acquire.radius = Some(receipt.radius);
    observation.acquire.cell = Some(ACQUIRE_KEY_CELL);
    observation.acquire.boxes = vec![ACQUIRE_KEY_LAIR];
    observation.acquire.receipt = Some(receipt);
    observation
}

fn acquire_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

fn acquire_with(observation: Observation, dest: LineOfSightTile, kind: &str) -> Observation {
    let mut joined = acquire_joined(observation);
    joined.acquire.dest = Some(dest);
    if let Some(receipt) = joined.acquire.receipt.as_mut() {
        receipt.dest = dest;
        receipt.kind = kind.into();
    }
    joined
}

#[test]
fn acquire_key_v2_requires_corridor_receipt_and_named_stop() {
    let case = CoreCase::parse("acquire_key_v2_ts").expect("named acquire key cell");
    assert!(case.copies_acquire_key());
    assert!(!case.copies_leave_lair());
    assert!(!case.copies_enter_lair());
    assert!(!case.copies_walk_spot());
    assert!(!case.copies_hold_spot());
    assert!(!case.copies_retreat_spot());
    assert!(!case.copies_fight_field());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = acquire_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );
    let joined = acquire_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );
    watch.observe("catalogtest", acquire_stop(joined, ACQUIRE_KEY_V2_STOP), false);
    watch
        .qualify()
        .expect("corridor walk-near radius 1 and named stop");
}

#[test]
fn acquire_key_rejects_wrong_kinds_dest_radius_and_forbidden_paint() {
    let case = CoreCase::parse("acquire_key_v2_ts").expect("named acquire key cell");
    let baseline = acquire_ready();

    for kind in ["npc", "obj", "leave", "yield", "walk", "aborted", "kbd"] {
        let watch = CoreWatch::default();
        watch.configure(case, "catalogtest");
        watch.observe("catalogtest", baseline.clone(), false);
        watch.begin_start("catalogtest").unwrap();
        let forged = acquire_stop(
            acquire_with(baseline.clone(), ACQUIRE_KEY_DEST, kind),
            ACQUIRE_KEY_V2_STOP,
        );
        watch.observe("catalogtest", forged, false);
        assert!(watch.qualify().is_err(), "{kind} cannot pass");
    }

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let wrong_dest = LineOfSightTile {
        x: 2932,
        z: 9690,
        level: 0,
    };
    watch.observe(
        "catalogtest",
        acquire_stop(
            acquire_with(baseline.clone(), wrong_dest, "walk-near"),
            ACQUIRE_KEY_V2_STOP,
        ),
        false,
    );
    assert!(watch.qualify().is_err(), "dest must be the corridor tile");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut radius = acquire_joined(baseline.clone());
    radius.acquire.radius = Some(3);
    if let Some(receipt) = radius.acquire.receipt.as_mut() {
        receipt.radius = 3;
    }
    watch.observe("catalogtest", acquire_stop(radius, ACQUIRE_KEY_V2_STOP), false);
    assert!(watch.qualify().is_err(), "radius 3 cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe(
        "catalogtest",
        acquire_stop(baseline.clone(), ACQUIRE_KEY_V2_STOP),
        false,
    );
    assert!(watch.qualify().is_err(), "missing receipt cannot pass");

    let here = acquire_here();
    let dest = ACQUIRE_KEY_DEST;
    let ok = format!(
        "{ACQUIRE_KEY_RECEIPT_PREFIX}{{\"here\":{{\"x\":{},\"z\":{},\"level\":0}},\"dest\":{{\"x\":{},\"z\":{},\"level\":0}},\"kind\":\"walk-near\",\"radius\":1,\"discriminator\":\"corridor\"}}",
        here.x, here.z, dest.x, dest.z
    );
    assert!(parse_acquire_key_receipt_line(&ok).is_some());
    assert!(parse_acquire_key_receipt_line(&format!("{ok} 1591")).is_none());
    assert!(parse_acquire_key_receipt_line(
        &ok.replace("\"radius\":1", "\"radius\":1,\"held\":1591")
    )
    .is_none());
    assert!(parse_acquire_key_receipt_line(
        &ok.replace("\"radius\":1", "\"radius\":1,\"key\":\"kbd-lair\"")
    )
    .is_none());
    assert!(parse_acquire_key_receipt_line(
        &ok.replace("\"radius\":1", "\"radius\":1,\"locId\":1765")
    )
    .is_none());
    assert!(parse_acquire_key_receipt_line(
        &ok.replace("\"radius\":1", "\"radius\":1,\"allow_teleports\":false")
    )
    .is_none());
    assert!(parse_acquire_key_receipt_line(
        &ok.replace("\"kind\":\"walk-near\"", "\"kind\":\"npc\"")
    )
    .is_none());
}

#[test]
fn acquire_key_rejects_wrong_stop_cell_and_projected_lair() {
    let case = CoreCase::parse("acquire_key_v2_ts").expect("named acquire key cell");
    let baseline = acquire_ready();

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = acquire_joined(baseline.clone());
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", acquire_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut inside_cell = acquire_ready();
    let cell_here = LineOfSightTile {
        x: 2931,
        z: 9686,
        level: 0,
    };
    inside_cell.tile = Some((cell_here.x, cell_here.z, cell_here.level));
    inside_cell.acquire.here = Some(cell_here);
    watch.observe("catalogtest", inside_cell, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "here inside the CELL box cannot start"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut inside_lair = acquire_joined(baseline.clone());
    let lair_here = LineOfSightTile {
        x: 50,
        z: 50,
        level: 0,
    };
    inside_lair.tile = Some((lair_here.x, lair_here.z, lair_here.level));
    inside_lair.acquire.here = Some(lair_here);
    if let Some(receipt) = inside_lair.acquire.receipt.as_mut() {
        receipt.here = lair_here;
    }
    watch.observe(
        "catalogtest",
        acquire_stop(inside_lair, ACQUIRE_KEY_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "here inside the projected lair box cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut cell_as_boxes = acquire_joined(baseline);
    cell_as_boxes.acquire.boxes = vec![ACQUIRE_KEY_CELL];
    watch.observe(
        "catalogtest",
        acquire_stop(cell_as_boxes, ACQUIRE_KEY_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "CELL projected as boxes cannot pass"
    );
}

fn cell_here() -> LineOfSightTile {
    LineOfSightTile {
        x: 3222,
        z: 3218,
        level: 0,
    }
}

fn cell_receipt(here: LineOfSightTile, kind: &str) -> CellV2ScriptReceipt {
    CellV2ScriptReceipt {
        here,
        kind: kind.into(),
        discriminator: "key-call".into(),
    }
}

fn cell_ready() -> Observation {
    let here = cell_here();
    let mut observation = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((here.x, here.z, here.level)),
        ..Observation::default()
    };
    observation.cell = CellV2Observation {
        available: true,
        here: Some(here),
        cell: Some(CELL_V2_CELL),
        boxes: vec![CELL_V2_LAIR],
        ..CellV2Observation::default()
    };
    observation
}

fn cell_joined(mut observation: Observation) -> Observation {
    observation.tick += 1;
    let here = cell_here();
    let receipt = cell_receipt(here, "key");
    observation.tile = Some((here.x, here.z, here.level));
    observation.cell.here = Some(here);
    observation.cell.cell = Some(CELL_V2_CELL);
    observation.cell.boxes = vec![CELL_V2_LAIR];
    observation.cell.door_walk = false;
    observation.cell.receipt = Some(receipt);
    observation
}

fn cell_stop(mut observation: Observation, reason: &str) -> Observation {
    observation.tick += 1;
    observation.script_lifecycle = Some(script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: observation.tick as u64,
        reason: reason.into(),
    });
    observation
}

fn cell_with_kind(observation: Observation, kind: &str) -> Observation {
    let mut joined = cell_joined(observation);
    if let Some(receipt) = joined.cell.receipt.as_mut() {
        receipt.kind = kind.into();
    }
    joined
}

#[test]
fn cell_v2_requires_key_call_receipt_and_named_stop() {
    let case = CoreCase::parse("cell_v2_ts").expect("named cell");
    assert!(case.copies_cell());
    assert!(!case.copies_acquire_key());
    assert!(!case.copies_leave_lair());
    assert!(!case.copies_enter_lair());
    assert!(!case.copies_walk_spot());
    assert!(!case.copies_hold_spot());
    assert!(!case.copies_retreat_spot());
    assert!(!case.copies_fight_field());
    assert!(!CoreCase::parse("acquire_key_v2_ts")
        .expect("key")
        .copies_cell());
    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let baseline = cell_ready();
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    watch.observe("catalogtest", baseline.clone(), false);
    assert!(
        watch.qualify().unwrap_err().contains("incomplete"),
        "seed-only scene identity must not qualify"
    );
    let joined = cell_joined(baseline);
    watch.observe("catalogtest", joined.clone(), false);
    assert!(
        watch.qualify().is_err(),
        "joined receipt without the named helper stop is incomplete"
    );
    watch.observe("catalogtest", cell_stop(joined, CELL_V2_STOP), false);
    watch
        .qualify()
        .expect("key-call kind key and named stop");
}

#[test]
fn cell_v2_rejects_walk_to_door_dusty_claim_kbd_and_wrong_kinds() {
    let case = CoreCase::parse("cell_v2_ts").expect("named cell");
    let baseline = cell_ready();

    for kind in [
        "walk",
        "walk-near",
        "walk-to",
        "use-on",
        "npc",
        "loc",
        "leave",
        "yield",
        "aborted",
    ] {
        let watch = CoreWatch::default();
        watch.configure(case, "catalogtest");
        watch.observe("catalogtest", baseline.clone(), false);
        watch.begin_start("catalogtest").unwrap();
        watch.observe(
            "catalogtest",
            cell_stop(cell_with_kind(baseline.clone(), kind), CELL_V2_STOP),
            false,
        );
        assert!(watch.qualify().is_err(), "{kind} cannot pass");
    }

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut door_walk = cell_joined(baseline.clone());
    door_walk.cell.door_walk = true;
    door_walk.tile = Some((CELL_V2_DOOR.x, CELL_V2_DOOR.z, CELL_V2_DOOR.level));
    door_walk.cell.here = Some(CELL_V2_DOOR);
    if let Some(receipt) = door_walk.cell.receipt.as_mut() {
        receipt.here = CELL_V2_DOOR;
        receipt.kind = "walk".into();
    }
    watch.observe(
        "catalogtest",
        cell_stop(door_walk, CELL_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "walk to (2931, 9690) cannot pass"
    );

    let here = cell_here();
    let ok = format!(
        "{CELL_V2_RECEIPT_PREFIX}{{\"here\":{{\"x\":{},\"z\":{},\"level\":0}},\"kind\":\"key\",\"discriminator\":\"key-call\"}}",
        here.x, here.z
    );
    assert!(parse_cell_v2_receipt_line(&ok).is_some());
    assert!(parse_cell_v2_receipt_line(&format!("{ok} 1590")).is_none());
    assert!(parse_cell_v2_receipt_line(
        &ok.replace("\"kind\":\"key\"", "\"kind\":\"key\",\"item\":1590")
    )
    .is_none());
    assert!(parse_cell_v2_receipt_line(&format!(
        "{CELL_V2_RECEIPT_PREFIX}{{\"here\":{{\"x\":{},\"z\":{},\"level\":0}},\"kind\":\"walk\",\"x\":2931,\"z\":9690,\"discriminator\":\"key-call\"}}",
        here.x, here.z
    ))
    .is_none());
    assert!(parse_cell_v2_receipt_line(&format!(
        "{CELL_V2_RECEIPT_PREFIX}{{\"here\":{{\"x\":{},\"z\":{},\"level\":0}},\"kind\":\"key\",\"discriminator\":\"key-call\",\"dest\":{{\"x\":2931,\"z\":9690,\"level\":0}}}}",
        here.x, here.z
    ))
    .is_none());
    assert!(parse_cell_v2_receipt_line(
        &ok.replace("\"kind\":\"key\"", "\"kind\":\"key\",\"key\":\"kbd-lair\"")
    )
    .is_none());
    assert!(parse_cell_v2_receipt_line(
        &ok.replace("\"kind\":\"key\"", "\"kind\":\"key\",\"locId\":1765")
    )
    .is_none());
    assert!(parse_cell_v2_receipt_line(
        &ok.replace("\"kind\":\"key\"", "\"kind\":\"walk\"")
    )
    .is_none());
}

#[test]
fn cell_v2_rejects_missing_stop_cell_box_and_projected_lair() {
    let case = CoreCase::parse("cell_v2_ts").expect("named cell");
    let baseline = cell_ready();

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let joined = cell_joined(baseline.clone());
    watch.observe("catalogtest", joined.clone(), false);
    watch.observe("catalogtest", cell_stop(joined, "some other stop"), false);
    assert!(watch.qualify().is_err(), "wrong named stop cannot pass");

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    let mut inside_cell = cell_ready();
    let inside = LineOfSightTile {
        x: 2931,
        z: 9686,
        level: 0,
    };
    inside_cell.tile = Some((inside.x, inside.z, inside.level));
    inside_cell.cell.here = Some(inside);
    watch.observe("catalogtest", inside_cell, false);
    assert!(
        watch.begin_start("catalogtest").is_err(),
        "here inside the CELL box cannot start"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut inside_lair = cell_joined(baseline.clone());
    let lair_here = LineOfSightTile {
        x: 50,
        z: 50,
        level: 0,
    };
    inside_lair.tile = Some((lair_here.x, lair_here.z, lair_here.level));
    inside_lair.cell.here = Some(lair_here);
    if let Some(receipt) = inside_lair.cell.receipt.as_mut() {
        receipt.here = lair_here;
    }
    watch.observe(
        "catalogtest",
        cell_stop(inside_lair, CELL_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "here inside the projected lair box cannot pass"
    );

    let watch = CoreWatch::default();
    watch.configure(case, "catalogtest");
    watch.observe("catalogtest", baseline.clone(), false);
    watch.begin_start("catalogtest").unwrap();
    let mut cell_as_boxes = cell_joined(baseline);
    cell_as_boxes.cell.boxes = vec![CELL_V2_CELL];
    watch.observe(
        "catalogtest",
        cell_stop(cell_as_boxes, CELL_V2_STOP),
        false,
    );
    assert!(
        watch.qualify().is_err(),
        "CELL projected as boxes cannot pass"
    );
}
