use std::sync::Arc;

use host_play::catalog_core::{
    firemaker_spec, BoundedLoc, CoreCase, CoreWatch, CoreWatchStatus, FiremakerCycle, Observation,
    OAK_LOGS_ID, TINDERBOX_ID, UNIDENTIFIED_GUAM_ID, UNIDENTIFIED_MARENTILL_ID, VARROCK_EAST_BANK,
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
