use std::sync::Arc;

use host_play::catalog_core::{
    firemaker_spec, BoundedLoc, CoreCase, CoreWatch, CoreWatchStatus, FiremakerCycle, Observation,
    CERT_RUNE_CHAINBODY_ID, COINS_ID, HIGH_ALCH_MAGIC_XP, NATURE_RUNE_ID, OAK_LOGS_ID,
    RUNE_CHAINBODY_HIGH_ALCH_COINS, STAFF_OF_FIRE_ID, TINDERBOX_ID, UNIDENTIFIED_GUAM_ID,
    UNIDENTIFIED_MARENTILL_ID, VARROCK_EAST_BANK, VARROCK_WEST_BANK,
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
