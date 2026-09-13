use host_play::paired_core::{
    AirObservation, AirRole, FlaxObservation, FlaxRole, PairCase, PairWatch, PairWatchStatus,
    StartBarrier, AIR_RUINS, FALADOR_EAST, FLAX_FIELD, FLAX_MEET, MULE_TRADE_CAP, TRADE_CAP,
};

fn air_ready(player: &str, role: AirRole) -> AirObservation {
    AirObservation {
        ingame: true,
        scene_state: 2,
        inventory_tab_available: true,
        player: Some(player.into()),
        tile: Some(AIR_RUINS),
        air_talisman: match role {
            AirRole::Master => 1,
            AirRole::Runner => 0,
        },
        essence_unnoted: match role {
            AirRole::Master => 0,
            AirRole::Runner => TRADE_CAP,
        },
        ..AirObservation::default()
    }
}

fn flax_ready(player: &str, role: FlaxRole) -> FlaxObservation {
    FlaxObservation {
        ingame: true,
        scene_state: 2,
        inventory_tab_available: true,
        player: Some(player.into()),
        tile: Some(match role {
            FlaxRole::Runner => FLAX_FIELD,
            FlaxRole::Spinner => FLAX_MEET,
        }),
        crafting: 1,
        ..FlaxObservation::default()
    }
}

#[test]
fn headed_cells_are_air_mule_flax() {
    assert_eq!(
        PairCase::headed_cells(),
        [PairCase::Air, PairCase::Mule, PairCase::Flax]
    );
    assert!(PairCase::Air.is_headed_cell());
    assert!(!PairCase::Duel.is_headed_cell());
    assert_eq!(
        PairCase::parse("nature_crafter_air").unwrap(),
        PairCase::Air
    );
    assert!(PairCase::parse("mule_crafter").is_err());
}

#[test]
fn shared_start_rejects_one_sided_and_stale_sessions() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Air, "master", "runner");
    watch.observe_air("master", air_ready("master", AirRole::Master), false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);
    assert!(watch.begin_shared_start("master", "runner").is_err());

    watch.observe_air("runner", air_ready("runner", AirRole::Runner), false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
    watch.observe_air("runner", AirObservation::default(), true);
    assert!(
        watch.begin_shared_start("master", "runner").is_err(),
        "pre-Start session boundary must invalidate the candidate"
    );
    watch.observe_air("runner", air_ready("runner", AirRole::Runner), false);
    watch.begin_shared_start("master", "runner").unwrap();
    watch.observe_air("master", AirObservation::default(), true);
    assert!(watch
        .failure()
        .expect("post-Start boundary is terminal")
        .contains("session boundary after Start"));
}

#[test]
fn wrong_counterpart_and_first_transfer_cannot_qualify_full_cycle() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Air, "master", "runner");
    watch.observe_air("master", air_ready("master", AirRole::Master), false);
    watch.observe_air("runner", air_ready("runner", AirRole::Runner), false);
    watch.begin_shared_start("master", "runner").unwrap();

    let mut master = air_ready("master", AirRole::Master);
    master.tick = 1;
    master.trade_offer_open = true;
    master.trade_partner = Some("stranger".into());
    watch.observe_air("master", master, false);
    let error = watch.qualify().unwrap_err();
    assert!(
        error.contains("wrong partner")
            || error.contains("one-sided")
            || error.contains("no further work")
            || error.contains("missing conservation")
            || error.contains("queued-only"),
        "{error}"
    );
}

#[test]
fn flax_full_cycle_caches_one_receipt() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Flax, "runner", "spinner");
    watch.observe_flax("runner", flax_ready("runner", FlaxRole::Runner), false);
    watch.observe_flax("spinner", flax_ready("spinner", FlaxRole::Spinner), false);
    watch.begin_shared_start("runner", "spinner").unwrap();

    let mut runner = flax_ready("runner", FlaxRole::Runner);
    let mut spinner = flax_ready("spinner", FlaxRole::Spinner);
    runner.tick = 1;
    spinner.tick = 1;
    runner.tile = Some(FLAX_MEET);
    spinner.tile = Some(FLAX_MEET);
    runner.flax = 24;
    runner.trade_offer_open = true;
    runner.trade_partner = Some("spinner".into());
    spinner.trade_offer_open = true;
    spinner.trade_partner = Some("runner".into());
    watch.observe_flax("runner", runner.clone(), false);
    watch.observe_flax("spinner", spinner.clone(), false);

    runner.trade_offer_open = false;
    runner.trade_confirm_open = true;
    spinner.trade_offer_open = false;
    spinner.trade_confirm_open = true;
    watch.observe_flax("runner", runner.clone(), false);
    watch.observe_flax("spinner", spinner.clone(), false);

    runner.trade_confirm_open = false;
    spinner.trade_confirm_open = false;
    runner.flax = 0;
    spinner.flax = 24;
    watch.observe_flax("runner", runner.clone(), false);
    watch.observe_flax("spinner", spinner.clone(), false);

    spinner.flax = 0;
    spinner.bow_string = 24;
    spinner.crafting_xp = 15;
    watch.observe_flax("spinner", spinner.clone(), false);

    spinner.bank_open = true;
    spinner.bank_loaded = true;
    spinner.tile = Some(host_play::paired_core::FLAX_BANK);
    spinner.bow_string = 0;
    watch.observe_flax("spinner", spinner.clone(), false);

    spinner.bank_open = false;
    spinner.tile = Some(FLAX_MEET);
    watch.observe_flax("spinner", spinner.clone(), false);

    runner.flax = 24;
    runner.trade_offer_open = true;
    runner.trade_partner = Some("spinner".into());
    spinner.trade_offer_open = true;
    spinner.trade_partner = Some("runner".into());
    watch.observe_flax("runner", runner.clone(), false);
    watch.observe_flax("spinner", spinner.clone(), false);
    runner.trade_offer_open = false;
    runner.trade_confirm_open = true;
    spinner.trade_offer_open = false;
    spinner.trade_confirm_open = true;
    watch.observe_flax("runner", runner.clone(), false);
    watch.observe_flax("spinner", spinner.clone(), false);
    runner.trade_confirm_open = false;
    spinner.trade_confirm_open = false;
    runner.flax = 0;
    spinner.flax = 24;
    watch.observe_flax("runner", runner, false);
    watch.observe_flax("spinner", spinner, false);

    assert_eq!(watch.status(), PairWatchStatus::Qualified);
    let first = watch.qualify().expect("full flax cycle qualifies");
    let second = watch.qualify().expect("terminal receipt remains cached");
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(first["case"], "flax");
}

#[test]
fn mule_seed_only_cannot_qualify() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Mule, "crafter", "mule");
    let mut crafter = air_ready("crafter", AirRole::Master);
    crafter.essence_unnoted = MULE_TRADE_CAP;
    crafter.air_talisman = 1;
    let mut mule = air_ready("mule", AirRole::Runner);
    mule.essence_unnoted = MULE_TRADE_CAP;
    mule.air_talisman = 0;
    watch.observe_air("crafter", crafter.clone(), false);
    watch.observe_air("mule", mule.clone(), false);
    watch.begin_shared_start("crafter", "mule").unwrap();
    crafter.tile = Some(FALADOR_EAST);
    watch.observe_air("crafter", crafter, false);
    watch.observe_air("mule", mule, false);
    let error = watch.qualify().unwrap_err();
    assert!(
        error.contains("one-sided")
            || error.contains("missing conservation")
            || error.contains("no further work")
            || error.contains("queued-only"),
        "{error}"
    );
}

#[test]
fn nature_air_login_and_display_names_share_start() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Air, "live1felgp_0", "live1felgp_1");
    watch.observe_air(
        "live1felgp_0",
        air_ready("Live1felgp 0", AirRole::Master),
        false,
    );
    watch.observe_air(
        "live1felgp_1",
        air_ready("Live1felgp 1", AirRole::Runner),
        false,
    );
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
    watch
        .begin_shared_start("live1felgp_0", "live1felgp_1")
        .expect("login underscores and native spaced display names are one account");

    let mut master = air_ready("Live1felgp 0", AirRole::Master);
    let mut runner = air_ready("Live1felgp 1", AirRole::Runner);
    master.tick = 1;
    runner.tick = 1;
    master.trade_offer_open = true;
    runner.trade_offer_open = true;
    master.trade_partner = Some("Live1felgp 1".into());
    runner.trade_partner = Some("Live1felgp 0".into());
    watch.observe_air("live1felgp_0", master, false);
    watch.observe_air("live1felgp_1", runner, false);
    let error = watch.qualify().unwrap_err();
    assert!(
        !error.contains("mixed identities"),
        "display names must not mark mixed identity: {error}"
    );
    assert!(
        !error.contains("wrong partner"),
        "native partner display names must match login counterparts: {error}"
    );
}

#[test]
fn nature_air_rejects_missing_empty_wrong_identity_and_wrong_partner() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Air, "live1felgp_0", "live1felgp_1");
    let runner = air_ready("Live1felgp 1", AirRole::Runner);

    let mut missing = air_ready("Live1felgp 0", AirRole::Master);
    missing.player = None;
    watch.observe_air("live1felgp_0", missing, false);
    watch.observe_air("live1felgp_1", runner.clone(), false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);
    assert!(watch
        .begin_shared_start("live1felgp_0", "live1felgp_1")
        .is_err());

    let mut empty = air_ready("Live1felgp 0", AirRole::Master);
    empty.player = Some(String::new());
    watch.observe_air("live1felgp_0", empty, false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);

    watch.observe_air(
        "live1felgp_0",
        air_ready("Live1felgp 1", AirRole::Master),
        false,
    );
    assert_eq!(
        watch.barrier(),
        StartBarrier::Wait,
        "runner display name must not prepare the master slot"
    );

    watch.observe_air(
        "live1felgp_0",
        air_ready("Live1felgp 0", AirRole::Master),
        false,
    );
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
    watch
        .begin_shared_start("live1felgp_0", "live1felgp_1")
        .unwrap();

    let mut master = air_ready("Live1felgp 0", AirRole::Master);
    let mut runner = air_ready("Live1felgp 1", AirRole::Runner);
    master.tick = 1;
    runner.tick = 1;
    master.trade_offer_open = true;
    master.trade_partner = Some("Live1felgp 9".into());
    watch.observe_air("live1felgp_0", master, false);
    watch.observe_air("live1felgp_1", runner, false);
    let error = watch.qualify().unwrap_err();
    assert!(error.contains("wrong partner"), "{error}");
}

#[test]
fn mule_and_flax_login_display_names_share_start() {
    let mule = PairWatch::default();
    mule.configure(PairCase::Mule, "live1felgp_0", "live1felgp_1");
    let mut crafter = air_ready("Live1felgp 0", AirRole::Master);
    crafter.essence_unnoted = MULE_TRADE_CAP;
    let mut pack = air_ready("Live1felgp 1", AirRole::Runner);
    pack.essence_unnoted = MULE_TRADE_CAP;
    pack.air_talisman = 0;
    mule.observe_air("live1felgp_0", crafter, false);
    mule.observe_air("live1felgp_1", pack, false);
    assert_eq!(mule.barrier(), StartBarrier::StartBoth);
    mule.begin_shared_start("live1felgp_0", "live1felgp_1")
        .expect("mule login/display pair must share Start");

    let flax = PairWatch::default();
    flax.configure(PairCase::Flax, "live1felgp_0", "live1felgp_1");
    flax.observe_flax(
        "live1felgp_0",
        flax_ready("Live1felgp 0", FlaxRole::Runner),
        false,
    );
    flax.observe_flax(
        "live1felgp_1",
        flax_ready("Live1felgp 1", FlaxRole::Spinner),
        false,
    );
    assert_eq!(flax.barrier(), StartBarrier::StartBoth);
    flax.begin_shared_start("live1felgp_0", "live1felgp_1")
        .expect("flax login/display pair must share Start");
}
