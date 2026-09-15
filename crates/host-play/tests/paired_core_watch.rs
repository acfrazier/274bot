use host_play::paired_core::{
    pair_settings, AirClaim, AirExchangeStage, AirObservation, AirPairWitness, AirRole,
    AirSlotRecord, DuelObservation, FlaxExchangeStage, FlaxObservation, FlaxRole, FlaxSlotRecord,
    MuleClaim, MuleExchangeStage, MulePairWitness, MuleRole, MuleSlotRecord, PairCase, PairWatch,
    PairWatchStatus, StartBarrier, AIR_RUINS, DUEL_CHALLENGE_ANCHOR, FALADOR_EAST, FLAX_FIELD,
    FLAX_MEET, MULE_TRADE_CAP, TRADE_CAP,
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
        crafting: match role {
            FlaxRole::Runner => 1,
            FlaxRole::Spinner => 10,
        },
        ..FlaxObservation::default()
    }
}

#[test]
fn headed_cells_are_air_mule_flax_duel() {
    assert_eq!(
        PairCase::headed_cells(),
        [
            PairCase::Air,
            PairCase::Mule,
            PairCase::Flax,
            PairCase::Duel
        ]
    );
    assert!(PairCase::Air.is_headed_cell());
    assert!(PairCase::Duel.is_headed_cell());
    assert_eq!(
        PairCase::parse("nature_crafter_air").unwrap(),
        PairCase::Air
    );
    assert_eq!(PairCase::parse("duel_arena").unwrap(), PairCase::Duel);
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
fn flax_spinner_requires_ten_crafting_but_runner_keeps_low_stat() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Flax, "runner", "spinner");
    watch.observe_flax("runner", flax_ready("runner", FlaxRole::Runner), false);
    let mut spinner = flax_ready("spinner", FlaxRole::Spinner);
    spinner.crafting = 9;
    watch.observe_flax("spinner", spinner, false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);
    assert!(watch.begin_shared_start("runner", "spinner").is_err());

    watch.observe_flax("spinner", flax_ready("spinner", FlaxRole::Spinner), false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
    watch
        .begin_shared_start("runner", "spinner")
        .expect("Crafting 10 admits spinner while Runner remains valid at 1");
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

#[test]
fn pair_settings_inject_native_display_partners_from_login_keys() {
    let login_a = "livev40xor_0";
    let login_b = "livev40xor_1";
    let screen_a = client::util::JString::to_screen_name(login_a);
    let screen_b = client::util::JString::to_screen_name(login_b);
    assert_eq!(screen_a, "Livev40xor 0");
    assert_eq!(screen_b, "Livev40xor 1");
    assert_ne!(screen_a, screen_b);
    assert_ne!(screen_a, login_a);
    assert_ne!(screen_b, login_b);

    for case in [PairCase::Air, PairCase::Mule, PairCase::Flax] {
        let bag_a = pair_settings(case, &[], 0, login_a, login_b).unwrap();
        let bag_b = pair_settings(case, &[], 1, login_b, login_a).unwrap();
        assert_eq!(
            bag_a.get("partner").and_then(|v| v.as_str()),
            Some(screen_b.as_str()),
            "{case:?} slot 0 partner must be the native display name, not the login key"
        );
        assert_eq!(
            bag_b.get("partner").and_then(|v| v.as_str()),
            Some(screen_a.as_str()),
            "{case:?} slot 1 partner must be the native display name, not the login key"
        );
        assert_ne!(
            bag_a.get("partner"),
            bag_b.get("partner"),
            "{case:?} slots must keep distinct partners"
        );

        let already = pair_settings(case, &[], 0, &screen_a, &screen_b).unwrap();
        assert_eq!(
            already.get("partner").and_then(|v| v.as_str()),
            Some(screen_b.as_str()),
            "{case:?} already-display partner names stay native display names"
        );
    }

    let watch = PairWatch::default();
    watch.configure(PairCase::Air, login_a, login_b);
    watch.observe_air(login_a, air_ready(&screen_a, AirRole::Master), false);
    watch.observe_air(login_b, air_ready(&screen_b, AirRole::Runner), false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
    watch
        .begin_shared_start(login_a, login_b)
        .expect("exact login slot keys still share Start after partner screen-name injection");
}

#[test]
fn freeze_pair_carries_installed_bags_and_rejects_wrong_partners() {
    let login_a = "livev40xor_0";
    let login_b = "livev40xor_1";
    let bag_a = pair_settings(PairCase::Air, &[], 0, login_a, login_b).unwrap();
    let bag_b = pair_settings(PairCase::Air, &[], 1, login_b, login_a).unwrap();
    assert_eq!(bag_a.get("mode").and_then(|v| v.as_str()), Some("Master"));
    assert_eq!(bag_b.get("mode").and_then(|v| v.as_str()), Some("Runner"));

    let watch = PairWatch::default();
    watch.configure(PairCase::Air, login_a, login_b);
    watch
        .install_prepared_settings(login_a, bag_a.clone(), login_b, bag_b.clone())
        .unwrap();
    watch.observe_air(login_a, air_ready("Livev40xor 0", AirRole::Master), false);
    watch.observe_air(login_b, air_ready("Livev40xor 1", AirRole::Runner), false);
    watch.begin_shared_start(login_a, login_b).unwrap();

    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["master"]["settings"]["mode"], "Master");
    assert_eq!(evidence["witness"]["runner"]["settings"]["mode"], "Runner");
    assert_eq!(
        evidence["witness"]["master"]["settings"]["partner"],
        bag_a["partner"]
    );
    assert_eq!(
        evidence["witness"]["runner"]["settings"]["partner"],
        bag_b["partner"]
    );
    assert_ne!(
        evidence["witness"]["master"]["settings"],
        serde_json::json!({})
    );

    let wrong = PairWatch::default();
    wrong.configure(PairCase::Air, login_a, login_b);
    let mut swapped_a = bag_a.clone();
    swapped_a.insert("partner".into(), bag_a["partner"].clone());
    let mut swapped_b = bag_b.clone();
    swapped_b.insert("partner".into(), bag_a["partner"].clone());
    let error = wrong
        .install_prepared_settings(login_a, swapped_a, login_b, swapped_b)
        .unwrap_err();
    assert!(
        error.contains("reciprocal account"),
        "same-side partner must be rejected: {error}"
    );
}

fn duel_ready(player: &str) -> DuelObservation {
    DuelObservation {
        ingame: true,
        scene_state: 2,
        inventory_tab_available: true,
        player: Some(player.into()),
        tile: Some(DUEL_CHALLENGE_ANCHOR),
        tick: 0,
        attack_xp: 0,
        strength_xp: 0,
        defence_xp: 0,
        hitpoints_xp: 0,
        in_combat: false,
        in_challenge_area: true,
        in_fight_pen: false,
        main_modal: -1,
        duel_offer_open: false,
        duel_confirm_open: false,
        duel_win_open: false,
        duel_partner: None,
        waiting_for_other: false,
        weapon_equipped: true,
        peer_visible: true,
    }
}

fn duel_watch() -> PairWatch {
    let watch = PairWatch::default();
    watch.configure(PairCase::Duel, "alice", "bob");
    watch
}

fn observe_first_combat(watch: &PairWatch) {
    let mut a = duel_ready("alice");
    let mut b = duel_ready("bob");
    a.tick = 1;
    b.tick = 1;
    a.duel_offer_open = true;
    b.duel_offer_open = true;
    a.duel_partner = Some("bob".into());
    b.duel_partner = Some("alice".into());
    watch.observe_duel("alice", a.clone(), false);
    watch.observe_duel("bob", b.clone(), false);

    a.duel_offer_open = false;
    b.duel_offer_open = false;
    a.duel_confirm_open = true;
    b.duel_confirm_open = true;
    watch.observe_duel("alice", a.clone(), false);
    watch.observe_duel("bob", b.clone(), false);

    a.duel_confirm_open = false;
    b.duel_confirm_open = false;
    a.in_challenge_area = false;
    b.in_challenge_area = false;
    a.in_fight_pen = true;
    b.in_fight_pen = true;
    a.in_combat = true;
    b.in_combat = true;
    a.attack_xp = 12;
    b.attack_xp = 8;
    watch.observe_duel("alice", a, false);
    watch.observe_duel("bob", b, false);
}

#[test]
fn duel_settings_carry_targets_and_reject_a_partner_key() {
    let bag = pair_settings(PairCase::Duel, &[], 0, "alice", "bob").unwrap();
    assert!(
        bag.get("partner").is_none(),
        "Duel schema has no partner setting: {bag:?}"
    );
    let watch = duel_watch();
    watch
        .install_prepared_settings("alice", bag.clone(), "bob", bag.clone())
        .expect("target bags without partner install");

    let mut with_partner = bag.clone();
    with_partner.insert("partner".into(), serde_json::json!("bob"));
    let error = watch
        .install_prepared_settings("alice", with_partner, "bob", bag)
        .unwrap_err();
    assert!(
        error.contains("must not carry partner"),
        "injected Duel partner must be rejected: {error}"
    );
}

#[test]
fn duel_barrier_needs_both_prepared_baselines() {
    let watch = duel_watch();
    watch.observe_duel("alice", duel_ready("alice"), false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);
    assert!(watch.begin_shared_start("alice", "bob").is_err());

    watch.observe_duel("bob", duel_ready("bob"), false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
    watch.begin_shared_start("alice", "bob").unwrap();
}

#[test]
fn duel_rejects_wrong_identity_modal_and_unequipped_baselines() {
    let watch = duel_watch();
    let mut modal = duel_ready("alice");
    modal.duel_offer_open = true;
    watch.observe_duel("alice", modal, false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);

    let mut stranger = duel_ready("alice");
    stranger.player = Some("stranger".into());
    watch.observe_duel("alice", stranger, false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);

    let mut bare = duel_ready("alice");
    bare.weapon_equipped = false;
    watch.observe_duel("alice", bare, false);
    assert_eq!(watch.barrier(), StartBarrier::Wait);

    watch.observe_duel("alice", duel_ready("alice"), false);
    assert_eq!(watch.barrier(), StartBarrier::StartBoth);
}

#[test]
fn duel_session_boundary_fails_and_keeps_witness() {
    let watch = duel_watch();
    watch.observe_duel("alice", duel_ready("alice"), false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    watch.begin_shared_start("alice", "bob").unwrap();
    watch.observe_duel("alice", duel_ready("alice"), true);
    let error = watch.failure().expect("post-Start boundary is terminal");
    assert!(error.contains("session boundary after Start"), "{error}");
    let evidence = watch.evidence();
    assert_eq!(evidence["phase"], "failed");
    assert_eq!(evidence["witness"]["case"], "duel");
    assert_eq!(evidence["witness"]["a"]["account"], "alice");
    assert_eq!(evidence["witness"]["b"]["account"], "bob");
}

#[test]
fn duel_first_combat_qualifies_without_status_upgrading_to_full_cycle() {
    let watch = duel_watch();
    watch.observe_duel("alice", duel_ready("alice"), false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    watch.begin_shared_start("alice", "bob").unwrap();
    observe_first_combat(&watch);

    assert_eq!(
        watch.status(),
        PairWatchStatus::Running,
        "FirstCombat must not silently become a full-cycle Qualified status"
    );
    let evidence = watch
        .qualify()
        .expect("supported first combat is a terminal Duel claim");
    assert_eq!(evidence["case"], "duel");
    assert_eq!(evidence["claim"], "first-combat");
    assert_eq!(evidence["full"], serde_json::Value::Null);
    assert_eq!(watch.status(), PairWatchStatus::Qualified);
    watch.clear();
    watch.clear();
    assert_eq!(watch.status(), PairWatchStatus::Disabled);
}

#[test]
fn duel_full_cycle_still_requires_reset_and_further_combat() {
    let watch = duel_watch();
    watch.observe_duel("alice", duel_ready("alice"), false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    watch.begin_shared_start("alice", "bob").unwrap();
    observe_first_combat(&watch);
    assert!(
        watch.qualify().is_ok(),
        "first combat is accepted at the Duel terminal claim"
    );

    let watch = duel_watch();
    watch.observe_duel("alice", duel_ready("alice"), false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    watch.begin_shared_start("alice", "bob").unwrap();
    observe_first_combat(&watch);

    let mut a = duel_ready("alice");
    let mut b = duel_ready("bob");
    a.tick = 4;
    b.tick = 4;
    a.attack_xp = 12;
    b.attack_xp = 8;
    a.in_fight_pen = false;
    b.in_fight_pen = false;
    a.in_challenge_area = true;
    b.in_challenge_area = true;
    watch.observe_duel("alice", a.clone(), false);
    watch.observe_duel("bob", b.clone(), false);

    a.in_challenge_area = false;
    b.in_challenge_area = false;
    a.in_fight_pen = true;
    b.in_fight_pen = true;
    a.in_combat = true;
    b.in_combat = true;
    a.attack_xp = 20;
    b.attack_xp = 14;
    watch.observe_duel("alice", a, false);
    watch.observe_duel("bob", b, false);

    assert_eq!(watch.status(), PairWatchStatus::Qualified);
    let evidence = watch
        .qualify()
        .expect("reset and further combat is full-cycle");
    assert_eq!(evidence["claim"], "reset-and-further");
}

#[test]
fn duel_wrong_observed_opponent_does_not_qualify() {
    let watch = duel_watch();
    watch.observe_duel("alice", duel_ready("alice"), false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    watch.begin_shared_start("alice", "bob").unwrap();

    let mut a = duel_ready("alice");
    a.tick = 1;
    a.duel_offer_open = true;
    a.duel_partner = Some("stranger".into());
    watch.observe_duel("alice", a, false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    let error = watch.qualify().unwrap_err();
    assert!(error.contains("wrong partner"), "{error}");
}

const TRADE_OFFER_ACCEPT: i32 = 3420;
const TRADE_CONFIRM_ACCEPT: i32 = 3546;
const FLAX_LOAD: i32 = 28;

fn flax_slot(role: FlaxRole) -> FlaxSlotRecord {
    let (account, partner) = match role {
        FlaxRole::Runner => ("runner", "spinner"),
        FlaxRole::Spinner => ("spinner", "runner"),
    };
    FlaxSlotRecord::new(
        role,
        account.into(),
        account.into(),
        partner.into(),
        serde_json::Map::new(),
        flax_ready(account, role),
    )
}

fn flax_trade(
    role: FlaxRole,
    tick: u32,
    flax: i32,
    offer: bool,
    confirm: bool,
    partner: Option<&str>,
    accept: i32,
    mine: i32,
) -> FlaxObservation {
    let player = match role {
        FlaxRole::Runner => "runner",
        FlaxRole::Spinner => "spinner",
    };
    FlaxObservation {
        tick,
        flax,
        tile: Some(FLAX_MEET),
        trade_offer_open: offer,
        trade_confirm_open: confirm,
        trade_partner: partner.map(str::to_string),
        trade_accept_id: accept,
        trade_mine_flax: mine,
        ..flax_ready(player, role)
    }
}

fn flax_offer(role: FlaxRole, tick: u32, flax: i32, mine: i32) -> FlaxObservation {
    let partner = match role {
        FlaxRole::Runner => "spinner",
        FlaxRole::Spinner => "runner",
    };
    flax_trade(
        role,
        tick,
        flax,
        true,
        false,
        Some(partner),
        TRADE_OFFER_ACCEPT,
        mine,
    )
}

fn flax_inactive(role: FlaxRole, tick: u32, flax: i32) -> FlaxObservation {
    flax_trade(role, tick, flax, false, false, None, -1, 0)
}

fn flax_confirm(role: FlaxRole, tick: u32, flax: i32) -> FlaxObservation {
    let partner = match role {
        FlaxRole::Runner => "spinner",
        FlaxRole::Spinner => "runner",
    };
    flax_trade(
        role,
        tick,
        flax,
        false,
        true,
        Some(partner),
        TRADE_CONFIRM_ACCEPT,
        0,
    )
}

fn flax_close(role: FlaxRole, tick: u32, flax: i32) -> FlaxObservation {
    flax_trade(role, tick, flax, false, false, None, -1, 0)
}

fn mule_baseline(role: MuleRole) -> AirObservation {
    let player = match role {
        MuleRole::Crafter => "crafter",
        MuleRole::Mule => "mule",
    };
    AirObservation {
        ingame: true,
        scene_state: 2,
        inventory_tab_available: true,
        player: Some(player.into()),
        tile: Some(AIR_RUINS),
        air_talisman: match role {
            MuleRole::Crafter => 1,
            MuleRole::Mule => 0,
        },
        essence_unnoted: match role {
            MuleRole::Crafter => 0,
            MuleRole::Mule => MULE_TRADE_CAP,
        },
        ..AirObservation::default()
    }
}

fn mule_slot(role: MuleRole) -> MuleSlotRecord {
    let (account, partner) = match role {
        MuleRole::Crafter => ("crafter", "mule"),
        MuleRole::Mule => ("mule", "crafter"),
    };
    MuleSlotRecord::new(
        role,
        account.into(),
        account.into(),
        partner.into(),
        serde_json::Map::new(),
        mule_baseline(role),
    )
}

fn mule_trade(
    role: MuleRole,
    tick: u32,
    essence: i32,
    air: i32,
    offer: bool,
    confirm: bool,
    partner: Option<&str>,
    accept: i32,
    mine: i32,
) -> AirObservation {
    AirObservation {
        tick,
        essence_unnoted: essence,
        air_runes: air,
        tile: Some(AIR_RUINS),
        trade_offer_open: offer,
        trade_confirm_open: confirm,
        trade_partner: partner.map(str::to_string),
        trade_accept_id: accept,
        trade_mine_essence: mine,
        ..mule_baseline(role)
    }
}

fn mule_offer(role: MuleRole, tick: u32, essence: i32, air: i32, mine: i32) -> AirObservation {
    let partner = match role {
        MuleRole::Crafter => "mule",
        MuleRole::Mule => "crafter",
    };
    mule_trade(
        role,
        tick,
        essence,
        air,
        true,
        false,
        Some(partner),
        TRADE_OFFER_ACCEPT,
        mine,
    )
}

fn mule_inactive(role: MuleRole, tick: u32, essence: i32, air: i32) -> AirObservation {
    mule_trade(role, tick, essence, air, false, false, None, -1, 0)
}

fn mule_confirm(role: MuleRole, tick: u32, essence: i32, air: i32) -> AirObservation {
    let partner = match role {
        MuleRole::Crafter => "mule",
        MuleRole::Mule => "crafter",
    };
    mule_trade(
        role,
        tick,
        essence,
        air,
        false,
        true,
        Some(partner),
        TRADE_CONFIRM_ACCEPT,
        0,
    )
}

fn mule_close(role: MuleRole, tick: u32, essence: i32, air: i32) -> AirObservation {
    mule_trade(role, tick, essence, air, false, false, None, -1, 0)
}

fn assert_flax_transfer(runner: &FlaxSlotRecord, spinner: &FlaxSlotRecord, events: u32, qty: i32) {
    assert!(runner.saw_offer_with_partner && spinner.saw_offer_with_partner);
    assert!(runner.saw_confirm_with_partner && spinner.saw_confirm_with_partner);
    assert_eq!(runner.partner_transfer_events, events);
    assert_eq!(spinner.partner_transfer_events, events);
    assert_eq!(runner.transferred_out, qty);
    assert_eq!(spinner.transferred_in, qty);
    assert_eq!(runner.exchange_stage, FlaxExchangeStage::Offer);
    assert_eq!(spinner.exchange_stage, FlaxExchangeStage::Offer);
}

fn assert_flax_no_transfer(runner: &FlaxSlotRecord, spinner: &FlaxSlotRecord, confirm: bool) {
    assert_eq!(runner.saw_confirm_with_partner, confirm);
    assert_eq!(spinner.saw_confirm_with_partner, confirm);
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(spinner.partner_transfer_events, 0);
    assert_eq!(runner.transferred_out, 0);
    assert_eq!(spinner.transferred_in, 0);
}

fn assert_mule_transfer(crafter: &MuleSlotRecord, mule: &MuleSlotRecord, events: u32, qty: i32) {
    assert!(crafter.saw_offer_with_partner && mule.saw_offer_with_partner);
    assert!(crafter.saw_confirm_with_partner && mule.saw_confirm_with_partner);
    assert_eq!(crafter.partner_transfer_events, events);
    assert_eq!(mule.partner_transfer_events, events);
    assert_eq!(crafter.transferred_in, qty);
    assert_eq!(mule.transferred_out, qty);
    assert_eq!(crafter.air_transferred_out, qty);
    assert_eq!(mule.air_transferred_in, qty);
    assert_eq!(crafter.exchange_stage, MuleExchangeStage::Offer);
    assert_eq!(mule.exchange_stage, MuleExchangeStage::Offer);
}

fn assert_mule_no_transfer(crafter: &MuleSlotRecord, mule: &MuleSlotRecord, confirm: bool) {
    assert_eq!(crafter.saw_confirm_with_partner, confirm);
    assert_eq!(mule.saw_confirm_with_partner, confirm);
    assert_eq!(crafter.partner_transfer_events, 0);
    assert_eq!(mule.partner_transfer_events, 0);
    assert_eq!(crafter.transferred_in, 0);
    assert_eq!(mule.transferred_out, 0);
}

fn flax_gap_close(runner: &mut FlaxSlotRecord, spinner: &mut FlaxSlotRecord) {
    runner.observe(flax_offer(FlaxRole::Runner, 94, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 94, 0, 0));
    runner.observe(flax_inactive(FlaxRole::Runner, 98, 0));
    spinner.observe(flax_inactive(FlaxRole::Spinner, 98, 0));
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(spinner.partner_transfer_events, 0);
    assert!(!runner.saw_confirm_with_partner);
    runner.observe(flax_confirm(FlaxRole::Runner, 98, 0));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 98, 0));
    runner.observe(flax_close(FlaxRole::Runner, 100, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 100, FLAX_LOAD));
}

fn mule_gap_close(crafter: &mut MuleSlotRecord, mule: &mut MuleSlotRecord) {
    crafter.observe(mule_offer(MuleRole::Crafter, 94, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 94, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_inactive(MuleRole::Crafter, 98, 0, 0));
    mule.observe(mule_inactive(MuleRole::Mule, 98, 0, 0));
    assert_eq!(crafter.partner_transfer_events, 0);
    assert_eq!(mule.partner_transfer_events, 0);
    crafter.observe(mule_confirm(MuleRole::Crafter, 98, 0, 0));
    mule.observe(mule_confirm(MuleRole::Mule, 98, 0, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 100, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 100, 0, MULE_TRADE_CAP));
}

#[test]
fn flax_observed_gap_counts_one_transfer_for_both_actors() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    flax_gap_close(&mut runner, &mut spinner);
    assert_flax_transfer(&runner, &spinner, 1, FLAX_LOAD);
}

#[test]
fn mule_observed_gap_counts_one_transfer_for_both_actors() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
}

#[test]
fn flax_direct_confirm_without_gap_still_counts() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_close(FlaxRole::Runner, 3, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 3, FLAX_LOAD));
    assert_flax_transfer(&runner, &spinner, 1, FLAX_LOAD);
}

#[test]
fn mule_direct_confirm_without_gap_still_counts() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
    mule.observe(mule_confirm(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 3, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 3, 0, MULE_TRADE_CAP));
    assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
}

#[test]
fn flax_repeated_offer_does_not_rebind_or_count_window_staging() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_offer(FlaxRole::Runner, 1, 0, FLAX_LOAD));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    assert_eq!(runner.exchange_stage, FlaxExchangeStage::Confirm);
    runner.observe(flax_confirm(FlaxRole::Runner, 2, 0));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_close(FlaxRole::Runner, 3, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 3, FLAX_LOAD));
    assert_flax_transfer(&runner, &spinner, 1, FLAX_LOAD);
}

#[test]
fn mule_repeated_offer_does_not_rebind_or_count_window_staging() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, 0, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, 0, 0, MULE_TRADE_CAP));
    assert_eq!(mule.exchange_stage, MuleExchangeStage::Confirm);
    crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, 0));
    mule.observe(mule_confirm(MuleRole::Mule, 2, 0, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 3, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 3, 0, MULE_TRADE_CAP));
    assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
}

#[test]
fn flax_gap_then_new_offer_rebinds_and_does_not_count_old() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_inactive(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_inactive(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_offer(FlaxRole::Runner, 3, 20, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 3, 0, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 4, 20));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 4, 0));
    runner.observe(flax_close(FlaxRole::Runner, 5, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 5, 20));
    assert_flax_transfer(&runner, &spinner, 1, 20);
}

#[test]
fn mule_gap_then_new_offer_rebinds_and_does_not_count_old() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_inactive(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
    mule.observe(mule_inactive(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
    crafter.observe(mule_offer(MuleRole::Crafter, 3, 0, 20, 0));
    mule.observe(mule_offer(MuleRole::Mule, 3, 20, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 4, 0, 20));
    mule.observe(mule_confirm(MuleRole::Mule, 4, 20, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 5, 20, 0));
    mule.observe(mule_close(MuleRole::Mule, 5, 0, 20));
    assert_mule_transfer(&crafter, &mule, 1, 20);
}

#[test]
fn flax_skipped_offer_later_confirm_drops_without_latch() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_inactive(FlaxRole::Runner, 2, 0));
    spinner.observe(flax_inactive(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 3, 0));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 3, 0));
    runner.observe(flax_close(FlaxRole::Runner, 4, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 4, FLAX_LOAD));
    assert_flax_no_transfer(&runner, &spinner, false);
    assert_eq!(runner.exchange_stage, FlaxExchangeStage::Offer);
}

#[test]
fn mule_skipped_offer_later_confirm_drops_without_latch() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_inactive(MuleRole::Crafter, 2, 0, 0));
    mule.observe(mule_inactive(MuleRole::Mule, 2, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 3, 0, 0));
    mule.observe(mule_confirm(MuleRole::Mule, 3, 0, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 4, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 4, 0, MULE_TRADE_CAP));
    assert_mule_no_transfer(&crafter, &mule, false);
    assert_eq!(mule.exchange_stage, MuleExchangeStage::Offer);
}

#[test]
fn flax_partnerless_confirm_drops_without_count() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_trade(
        FlaxRole::Runner,
        2,
        FLAX_LOAD,
        false,
        true,
        None,
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    spinner.observe(flax_trade(
        FlaxRole::Spinner,
        2,
        0,
        false,
        true,
        None,
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    runner.observe(flax_close(FlaxRole::Runner, 3, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 3, FLAX_LOAD));
    assert_flax_no_transfer(&runner, &spinner, false);
}

#[test]
fn mule_partnerless_confirm_drops_without_count() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_trade(
        MuleRole::Crafter,
        2,
        0,
        MULE_TRADE_CAP,
        false,
        true,
        None,
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    mule.observe(mule_trade(
        MuleRole::Mule,
        2,
        MULE_TRADE_CAP,
        0,
        false,
        true,
        None,
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    crafter.observe(mule_close(MuleRole::Crafter, 3, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 3, 0, MULE_TRADE_CAP));
    assert_mule_no_transfer(&crafter, &mule, false);
}

#[test]
fn flax_wrong_partner_drops_without_count() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_trade(
        FlaxRole::Runner,
        2,
        FLAX_LOAD,
        false,
        true,
        Some("stranger"),
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    spinner.observe(flax_trade(
        FlaxRole::Spinner,
        2,
        0,
        false,
        true,
        Some("stranger"),
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    runner.observe(flax_close(FlaxRole::Runner, 3, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 3, FLAX_LOAD));
    assert!(runner.saw_wrong_partner);
    assert!(spinner.saw_wrong_partner);
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(spinner.partner_transfer_events, 0);
}

#[test]
fn mule_wrong_partner_drops_without_count() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_trade(
        MuleRole::Crafter,
        2,
        0,
        MULE_TRADE_CAP,
        false,
        true,
        Some("stranger"),
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    mule.observe(mule_trade(
        MuleRole::Mule,
        2,
        MULE_TRADE_CAP,
        0,
        false,
        true,
        Some("stranger"),
        TRADE_CONFIRM_ACCEPT,
        0,
    ));
    crafter.observe(mule_close(MuleRole::Crafter, 3, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 3, 0, MULE_TRADE_CAP));
    assert!(crafter.saw_wrong_partner);
    assert!(mule.saw_wrong_partner);
    assert_eq!(crafter.partner_transfer_events, 0);
    assert_eq!(mule.partner_transfer_events, 0);
}

#[test]
fn flax_same_close_tick_delayed_inv_counts_once() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_close(FlaxRole::Runner, 5, FLAX_LOAD));
    spinner.observe(flax_close(FlaxRole::Spinner, 5, 0));
    assert_eq!(runner.partner_transfer_events, 0);
    runner.observe(flax_close(FlaxRole::Runner, 5, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 5, FLAX_LOAD));
    assert_flax_transfer(&runner, &spinner, 1, FLAX_LOAD);
}

#[test]
fn mule_same_close_tick_delayed_inv_counts_once() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
    mule.observe(mule_confirm(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 5, 0, MULE_TRADE_CAP));
    mule.observe(mule_close(MuleRole::Mule, 5, MULE_TRADE_CAP, 0));
    assert_eq!(mule.partner_transfer_events, 0);
    crafter.observe(mule_close(MuleRole::Crafter, 5, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 5, 0, MULE_TRADE_CAP));
    assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
}

#[test]
fn flax_later_tick_unrelated_delta_is_refused() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_close(FlaxRole::Runner, 5, FLAX_LOAD));
    spinner.observe(flax_close(FlaxRole::Spinner, 5, 0));
    runner.observe(flax_close(FlaxRole::Runner, 6, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 6, FLAX_LOAD));
    assert_flax_no_transfer(&runner, &spinner, true);
}

#[test]
fn mule_later_tick_unrelated_delta_is_refused() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
    mule.observe(mule_confirm(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 5, 0, MULE_TRADE_CAP));
    mule.observe(mule_close(MuleRole::Mule, 5, MULE_TRADE_CAP, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 6, MULE_TRADE_CAP, 0));
    mule.observe(mule_close(MuleRole::Mule, 6, 0, MULE_TRADE_CAP));
    assert_mule_no_transfer(&crafter, &mule, true);
}

#[test]
fn flax_confirmed_close_without_conservation_does_not_count() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_close(FlaxRole::Runner, 5, FLAX_LOAD));
    spinner.observe(flax_close(FlaxRole::Spinner, 5, 0));
    runner.observe(flax_close(FlaxRole::Runner, 6, 4));
    spinner.observe(flax_close(FlaxRole::Spinner, 6, 9));
    assert_flax_no_transfer(&runner, &spinner, true);
}

#[test]
fn mule_confirmed_close_without_conservation_does_not_count() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
    mule.observe(mule_confirm(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 5, 0, MULE_TRADE_CAP));
    mule.observe(mule_close(MuleRole::Mule, 5, MULE_TRADE_CAP, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 6, 3, 4));
    mule.observe(mule_close(MuleRole::Mule, 6, 5, 6));
    assert_mule_no_transfer(&crafter, &mule, true);
}

#[test]
fn flax_gap_path_dedups_extra_pack_motion() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    flax_gap_close(&mut runner, &mut spinner);
    runner.observe(flax_close(FlaxRole::Runner, 101, 4));
    spinner.observe(flax_close(FlaxRole::Spinner, 101, 32));
    assert_flax_transfer(&runner, &spinner, 1, FLAX_LOAD);
}

#[test]
fn mule_gap_path_dedups_extra_pack_motion() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    crafter.observe(mule_close(MuleRole::Crafter, 101, 30, 2));
    mule.observe(mule_close(MuleRole::Mule, 101, 2, 30));
    assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
}

#[test]
fn flax_new_offer_frame_does_not_count_old_episode() {
    let mut runner = flax_slot(FlaxRole::Runner);
    let mut spinner = flax_slot(FlaxRole::Spinner);
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_confirm(FlaxRole::Runner, 2, 0));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_offer(FlaxRole::Runner, 3, 0, 20));
    spinner.observe(flax_offer(FlaxRole::Spinner, 3, 0, 0));
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(spinner.partner_transfer_events, 0);
    assert_eq!(runner.exchange_stage, FlaxExchangeStage::Confirm);
    runner.observe(flax_confirm(FlaxRole::Runner, 4, 0));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 4, 0));
    runner.observe(flax_close(FlaxRole::Runner, 5, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 5, 20));
    assert_flax_transfer(&runner, &spinner, 1, 20);
}

#[test]
fn mule_new_offer_frame_does_not_count_old_episode() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
    mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, 0));
    mule.observe(mule_confirm(MuleRole::Mule, 2, 0, 0));
    crafter.observe(mule_offer(MuleRole::Crafter, 3, 0, 20, 0));
    mule.observe(mule_offer(MuleRole::Mule, 3, 0, 0, 20));
    assert_eq!(crafter.partner_transfer_events, 0);
    assert_eq!(mule.partner_transfer_events, 0);
    assert_eq!(mule.exchange_stage, MuleExchangeStage::Confirm);
    crafter.observe(mule_confirm(MuleRole::Crafter, 4, 0, 20));
    mule.observe(mule_confirm(MuleRole::Mule, 4, 0, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 5, 20, 0));
    mule.observe(mule_close(MuleRole::Mule, 5, 0, 20));
    assert_mule_transfer(&crafter, &mule, 1, 20);
}

#[test]
fn flax_and_mule_session_boundary_after_start_is_terminal() {
    let flax = PairWatch::default();
    flax.configure(PairCase::Flax, "runner", "spinner");
    flax.observe_flax("runner", flax_ready("runner", FlaxRole::Runner), false);
    flax.observe_flax("spinner", flax_ready("spinner", FlaxRole::Spinner), false);
    flax.begin_shared_start("runner", "spinner").unwrap();
    flax.observe_flax("runner", flax_ready("runner", FlaxRole::Runner), true);
    let flax_error = flax
        .failure()
        .expect("flax post-Start boundary is terminal");
    assert!(
        flax_error.contains("session boundary after Start"),
        "{flax_error}"
    );

    let mule = PairWatch::default();
    mule.configure(PairCase::Mule, "crafter", "mule");
    let mut crafter = mule_baseline(MuleRole::Crafter);
    crafter.essence_unnoted = MULE_TRADE_CAP;
    let mut pack = mule_baseline(MuleRole::Mule);
    pack.essence_unnoted = MULE_TRADE_CAP;
    mule.observe_air("crafter", crafter, false);
    mule.observe_air("mule", pack, false);
    mule.begin_shared_start("crafter", "mule").unwrap();
    mule.observe_air("crafter", mule_baseline(MuleRole::Crafter), true);
    let mule_error = mule
        .failure()
        .expect("mule post-Start boundary is terminal");
    assert!(
        mule_error.contains("session boundary after Start"),
        "{mule_error}"
    );
}

#[test]
fn flax_gap_transfer_does_not_qualify_without_further_cycle() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Flax, "runner", "spinner");
    watch.observe_flax("runner", flax_ready("runner", FlaxRole::Runner), false);
    watch.observe_flax("spinner", flax_ready("spinner", FlaxRole::Spinner), false);
    watch.begin_shared_start("runner", "spinner").unwrap();
    watch.observe_flax(
        "runner",
        flax_offer(FlaxRole::Runner, 94, FLAX_LOAD, 0),
        false,
    );
    watch.observe_flax("spinner", flax_offer(FlaxRole::Spinner, 94, 0, 0), false);
    watch.observe_flax("runner", flax_inactive(FlaxRole::Runner, 98, 0), false);
    watch.observe_flax("spinner", flax_inactive(FlaxRole::Spinner, 98, 0), false);
    watch.observe_flax("runner", flax_confirm(FlaxRole::Runner, 98, 0), false);
    watch.observe_flax("spinner", flax_confirm(FlaxRole::Spinner, 98, 0), false);
    watch.observe_flax("runner", flax_close(FlaxRole::Runner, 100, 0), false);
    watch.observe_flax(
        "spinner",
        flax_close(FlaxRole::Spinner, 100, FLAX_LOAD),
        false,
    );
    let error = watch.qualify().unwrap_err();
    assert!(
        error.contains("no further work")
            || error.contains("bank")
            || error.contains("Make-X")
            || error.contains("seed-only"),
        "{error}"
    );
    let evidence = watch.evidence();
    assert_eq!(
        evidence["witness"]["runner"]["saw_confirm_with_partner"],
        true
    );
    assert_eq!(
        evidence["witness"]["spinner"]["saw_confirm_with_partner"],
        true
    );
    assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 1);
    assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 1);
}

#[test]
fn mule_gap_transfer_does_not_qualify_without_further_cycle() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Mule, "crafter", "mule");
    let mut crafter = mule_baseline(MuleRole::Crafter);
    crafter.essence_unnoted = MULE_TRADE_CAP;
    let mut pack = mule_baseline(MuleRole::Mule);
    pack.essence_unnoted = MULE_TRADE_CAP;
    watch.observe_air("crafter", crafter, false);
    watch.observe_air("mule", pack, false);
    watch.begin_shared_start("crafter", "mule").unwrap();
    watch.observe_air(
        "crafter",
        mule_offer(MuleRole::Crafter, 94, 0, MULE_TRADE_CAP, 0),
        false,
    );
    watch.observe_air(
        "mule",
        mule_offer(MuleRole::Mule, 94, MULE_TRADE_CAP, 0, 0),
        false,
    );
    watch.observe_air("crafter", mule_inactive(MuleRole::Crafter, 98, 0, 0), false);
    watch.observe_air("mule", mule_inactive(MuleRole::Mule, 98, 0, 0), false);
    watch.observe_air("crafter", mule_confirm(MuleRole::Crafter, 98, 0, 0), false);
    watch.observe_air("mule", mule_confirm(MuleRole::Mule, 98, 0, 0), false);
    watch.observe_air(
        "crafter",
        mule_close(MuleRole::Crafter, 100, MULE_TRADE_CAP, 0),
        false,
    );
    watch.observe_air(
        "mule",
        mule_close(MuleRole::Mule, 100, 0, MULE_TRADE_CAP),
        false,
    );
    let error = watch.qualify().unwrap_err();
    assert!(
        error.contains("no further work")
            || error.contains("bank")
            || error.contains("missing conservation")
            || error.contains("seed-only")
            || error.contains("post-exchange"),
        "{error}"
    );
    let evidence = watch.evidence();
    assert_eq!(
        evidence["witness"]["crafter"]["saw_confirm_with_partner"],
        true
    );
    assert_eq!(
        evidence["witness"]["mule"]["saw_confirm_with_partner"],
        true
    );
    assert_eq!(evidence["witness"]["crafter"]["partner_transfer_events"], 1);
    assert_eq!(evidence["witness"]["mule"]["partner_transfer_events"], 1);
}

#[test]
fn flax_partnerless_new_offer_invalidates_previous_episode() {
    for confirmed in [true, false] {
        for missing_partner in [None, Some("")] {
            let mut runner = flax_slot(FlaxRole::Runner);
            let mut spinner = flax_slot(FlaxRole::Spinner);
            runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
            spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
            if confirmed {
                runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
                spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
            } else {
                runner.observe(flax_inactive(FlaxRole::Runner, 2, FLAX_LOAD));
                spinner.observe(flax_inactive(FlaxRole::Spinner, 2, 0));
            }
            let mut runner_offer = flax_offer(FlaxRole::Runner, 3, FLAX_LOAD, 0);
            let mut spinner_offer = flax_offer(FlaxRole::Spinner, 3, 0, 0);
            runner_offer.trade_partner = missing_partner.map(str::to_owned);
            spinner_offer.trade_partner = missing_partner.map(str::to_owned);
            runner.observe(runner_offer);
            spinner.observe(spinner_offer);
            // A late label on the same offer cannot reconstruct an unbound episode.
            runner.observe(flax_offer(FlaxRole::Runner, 4, FLAX_LOAD, 0));
            spinner.observe(flax_offer(FlaxRole::Spinner, 4, 0, 0));
            runner.observe(flax_confirm(FlaxRole::Runner, 5, FLAX_LOAD));
            spinner.observe(flax_confirm(FlaxRole::Spinner, 5, 0));
            runner.observe(flax_close(FlaxRole::Runner, 6, 0));
            spinner.observe(flax_close(FlaxRole::Spinner, 6, FLAX_LOAD));
            assert_flax_no_transfer(&runner, &spinner, confirmed);
            // A separate, fully observed named offer still starts fresh.
            runner.observe(flax_offer(FlaxRole::Runner, 7, FLAX_LOAD, 0));
            spinner.observe(flax_offer(FlaxRole::Spinner, 7, 0, 0));
            runner.observe(flax_confirm(FlaxRole::Runner, 8, FLAX_LOAD));
            spinner.observe(flax_confirm(FlaxRole::Spinner, 8, 0));
            runner.observe(flax_close(FlaxRole::Runner, 9, 0));
            spinner.observe(flax_close(FlaxRole::Spinner, 9, FLAX_LOAD));
            assert_flax_transfer(&runner, &spinner, 1, FLAX_LOAD);
        }
    }
}

#[test]
fn mule_partnerless_new_offer_invalidates_previous_episode() {
    for confirmed in [true, false] {
        for missing_partner in [None, Some("")] {
            let mut crafter = mule_slot(MuleRole::Crafter);
            let mut mule = mule_slot(MuleRole::Mule);
            crafter.observe(mule_offer(MuleRole::Crafter, 1, 0, MULE_TRADE_CAP, 0));
            mule.observe(mule_offer(MuleRole::Mule, 1, MULE_TRADE_CAP, 0, 0));
            if confirmed {
                crafter.observe(mule_confirm(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
                mule.observe(mule_confirm(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
            } else {
                crafter.observe(mule_inactive(MuleRole::Crafter, 2, 0, MULE_TRADE_CAP));
                mule.observe(mule_inactive(MuleRole::Mule, 2, MULE_TRADE_CAP, 0));
            }
            let mut crafter_offer = mule_offer(MuleRole::Crafter, 3, 0, MULE_TRADE_CAP, 0);
            let mut mule_row = mule_offer(MuleRole::Mule, 3, MULE_TRADE_CAP, 0, 0);
            crafter_offer.trade_partner = missing_partner.map(str::to_owned);
            mule_row.trade_partner = missing_partner.map(str::to_owned);
            crafter.observe(crafter_offer);
            mule.observe(mule_row);
            // A late label on the same offer cannot reconstruct an unbound episode.
            crafter.observe(mule_offer(MuleRole::Crafter, 4, 0, MULE_TRADE_CAP, 0));
            mule.observe(mule_offer(MuleRole::Mule, 4, MULE_TRADE_CAP, 0, 0));
            crafter.observe(mule_confirm(MuleRole::Crafter, 5, 0, MULE_TRADE_CAP));
            mule.observe(mule_confirm(MuleRole::Mule, 5, MULE_TRADE_CAP, 0));
            crafter.observe(mule_close(MuleRole::Crafter, 6, MULE_TRADE_CAP, 0));
            mule.observe(mule_close(MuleRole::Mule, 6, 0, MULE_TRADE_CAP));
            assert_mule_no_transfer(&crafter, &mule, confirmed);
            // A separate, fully observed named offer still starts fresh.
            crafter.observe(mule_offer(MuleRole::Crafter, 7, 0, MULE_TRADE_CAP, 0));
            mule.observe(mule_offer(MuleRole::Mule, 7, MULE_TRADE_CAP, 0, 0));
            crafter.observe(mule_confirm(MuleRole::Crafter, 8, 0, MULE_TRADE_CAP));
            mule.observe(mule_confirm(MuleRole::Mule, 8, MULE_TRADE_CAP, 0));
            crafter.observe(mule_close(MuleRole::Crafter, 9, MULE_TRADE_CAP, 0));
            mule.observe(mule_close(MuleRole::Mule, 9, 0, MULE_TRADE_CAP));
            assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
        }
    }
}

const AIR_TEMPLE: (i32, i32, i32) = (2841, 4834, 0);
const AIR_CRAFT_XP: i32 = 125;
const MULE_CRAFT_XP: i32 = 135;

fn air_slot(role: AirRole) -> AirSlotRecord {
    let (account, partner) = match role {
        AirRole::Master => ("master", "runner"),
        AirRole::Runner => ("runner", "master"),
    };
    AirSlotRecord::new(
        role,
        account.into(),
        account.into(),
        partner.into(),
        serde_json::Map::new(),
        air_ready(account, role),
    )
}

fn air_trade(
    role: AirRole,
    tick: u32,
    essence: i32,
    offer: bool,
    confirm: bool,
    partner: Option<&str>,
    accept: i32,
    mine: i32,
) -> AirObservation {
    let player = match role {
        AirRole::Master => "master",
        AirRole::Runner => "runner",
    };
    AirObservation {
        tick,
        essence_unnoted: essence,
        tile: Some(AIR_RUINS),
        trade_offer_open: offer,
        trade_confirm_open: confirm,
        trade_partner: partner.map(str::to_string),
        trade_accept_id: accept,
        trade_mine_essence: mine,
        ..air_ready(player, role)
    }
}

fn air_offer(role: AirRole, tick: u32, essence: i32, mine: i32) -> AirObservation {
    let partner = match role {
        AirRole::Master => "runner",
        AirRole::Runner => "master",
    };
    air_trade(
        role,
        tick,
        essence,
        true,
        false,
        Some(partner),
        TRADE_OFFER_ACCEPT,
        mine,
    )
}

fn air_inactive(role: AirRole, tick: u32, essence: i32) -> AirObservation {
    air_trade(role, tick, essence, false, false, None, -1, 0)
}

fn air_confirm(role: AirRole, tick: u32, essence: i32) -> AirObservation {
    let partner = match role {
        AirRole::Master => "runner",
        AirRole::Runner => "master",
    };
    air_trade(
        role,
        tick,
        essence,
        false,
        true,
        Some(partner),
        TRADE_CONFIRM_ACCEPT,
        0,
    )
}

fn air_close(role: AirRole, tick: u32, essence: i32) -> AirObservation {
    air_trade(role, tick, essence, false, false, None, -1, 0)
}

fn air_temple(role: AirRole, tick: u32, essence: i32, air: i32, xp: i32) -> AirObservation {
    let player = match role {
        AirRole::Master => "master",
        AirRole::Runner => "runner",
    };
    AirObservation {
        tick,
        essence_unnoted: essence,
        air_runes: air,
        runecraft_xp: xp,
        tile: Some(AIR_TEMPLE),
        in_temple: true,
        ..air_ready(player, role)
    }
}

fn air_keep(mut observation: AirObservation, air: i32, xp: i32) -> AirObservation {
    observation.air_runes = air;
    observation.runecraft_xp = xp;
    observation
}

fn nature_first_close(master: &mut AirSlotRecord, runner: &mut AirSlotRecord) {
    master.observe(air_offer(AirRole::Master, 10, 0, 0));
    runner.observe(air_offer(AirRole::Runner, 10, TRADE_CAP, 0));
    master.observe(air_inactive(AirRole::Master, 14, 0));
    runner.observe(air_inactive(AirRole::Runner, 14, 0));
    master.observe(air_confirm(AirRole::Master, 14, 0));
    runner.observe(air_confirm(AirRole::Runner, 14, 0));
    master.observe(air_close(AirRole::Master, 16, TRADE_CAP));
    runner.observe(air_close(AirRole::Runner, 16, 0));
}

fn nature_first_craft(master: &mut AirSlotRecord) {
    master.observe(air_temple(AirRole::Master, 20, 0, 0, 0));
    master.observe(air_temple(AirRole::Master, 21, 0, 0, AIR_CRAFT_XP));
    master.observe(air_temple(AirRole::Master, 22, 0, TRADE_CAP, AIR_CRAFT_XP));
}

fn nature_runner_bank_return(runner: &mut AirSlotRecord, tick: u32, essence: i32) {
    let mut bank = air_close(AirRole::Runner, tick, essence);
    bank.tile = Some(FALADOR_EAST);
    bank.bank_open = true;
    bank.bank_loaded = true;
    runner.observe(bank);
    runner.observe(air_close(AirRole::Runner, tick + 2, essence));
}

fn mule_temple(role: MuleRole, tick: u32, essence: i32, air: i32, xp: i32) -> AirObservation {
    AirObservation {
        tick,
        essence_unnoted: essence,
        air_runes: air,
        runecraft_xp: xp,
        tile: Some(AIR_TEMPLE),
        in_temple: true,
        ..mule_baseline(role)
    }
}

fn assert_air_transfer(master: &AirSlotRecord, runner: &AirSlotRecord, events: u32, qty: i32) {
    assert!(master.saw_offer_with_partner && runner.saw_offer_with_partner);
    assert!(master.saw_confirm_with_partner && runner.saw_confirm_with_partner);
    assert_eq!(master.partner_transfer_events, events);
    assert_eq!(runner.partner_transfer_events, events);
    assert_eq!(master.transferred_in, qty);
    assert_eq!(runner.transferred_out, qty);
    assert_eq!(master.exchange_stage, AirExchangeStage::Offer);
    assert_eq!(runner.exchange_stage, AirExchangeStage::Offer);
}

#[test]
fn nature_first25_bankrefill_second_offer_gap_is_not_full_cycle() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    nature_first_craft(&mut master);
    nature_runner_bank_return(&mut runner, 40, TRADE_CAP);
    assert_air_transfer(&master, &runner, 1, TRADE_CAP);
    runner.observe(air_offer(AirRole::Runner, 50, TRADE_CAP, 0));
    master.observe(air_keep(
        air_offer(AirRole::Master, 50, 0, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_offer(AirRole::Runner, 50, 0, TRADE_CAP));
    master.observe(air_keep(
        air_inactive(AirRole::Master, 54, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_inactive(AirRole::Runner, 54, 0));
    assert_eq!(master.partner_transfer_events, 1);
    assert_eq!(runner.partner_transfer_events, 1);
    assert_eq!(master.transferred_in, TRADE_CAP);
    assert_eq!(runner.transferred_out, TRADE_CAP);
    assert_eq!(master.transferred_out, 0);
    assert_eq!(runner.transferred_in, 0);
    assert_eq!(master.post_transfer_craft_events, 1);
    assert_eq!(master.craft_events, 1);
    assert!(runner.restock_withdraw && runner.returned_to_ruins);
    let pair = AirPairWitness { master, runner };
    assert_eq!(
        pair.qualify_supported().unwrap(),
        AirClaim::FirstTransferCraft
    );
    let err = pair
        .qualify_full_cycle()
        .expect_err("first 25 craft plus restock plus second-offer staging is not a second cycle");
    assert!(
        err.contains("no further work") || err.contains("second"),
        "{err}"
    );
}

#[test]
fn nature_second_receipt_and_fresh_second_craft_qualifies_full_cycle() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    nature_first_craft(&mut master);
    nature_runner_bank_return(&mut runner, 40, TRADE_CAP);
    master.observe(air_keep(
        air_offer(AirRole::Master, 60, 0, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_offer(AirRole::Runner, 60, TRADE_CAP, 0));
    master.observe(air_keep(
        air_confirm(AirRole::Master, 62, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_confirm(AirRole::Runner, 62, 0));
    master.observe(air_keep(
        air_close(AirRole::Master, 64, TRADE_CAP),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_close(AirRole::Runner, 64, 0));
    master.observe(air_temple(AirRole::Master, 70, 0, TRADE_CAP, AIR_CRAFT_XP));
    master.observe(air_temple(
        AirRole::Master,
        71,
        0,
        TRADE_CAP,
        AIR_CRAFT_XP * 2,
    ));
    master.observe(air_temple(
        AirRole::Master,
        72,
        0,
        TRADE_CAP * 2,
        AIR_CRAFT_XP * 2,
    ));
    assert_air_transfer(&master, &runner, 2, TRADE_CAP * 2);
    assert!(runner.second_transfer_after_bank_return);
    assert_eq!(master.post_transfer_craft_events, 2);
    let pair = AirPairWitness { master, runner };
    assert_eq!(
        pair.qualify_full_cycle().unwrap(),
        AirClaim::BankReturnSecondCycle
    );
}

#[test]
fn nature_no_second_craft_after_second_receipt_is_not_full_cycle() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    nature_first_craft(&mut master);
    nature_runner_bank_return(&mut runner, 40, TRADE_CAP);
    master.observe(air_keep(
        air_offer(AirRole::Master, 60, 0, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_offer(AirRole::Runner, 60, TRADE_CAP, 0));
    master.observe(air_keep(
        air_confirm(AirRole::Master, 62, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_confirm(AirRole::Runner, 62, 0));
    master.observe(air_keep(
        air_close(AirRole::Master, 64, TRADE_CAP),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    runner.observe(air_close(AirRole::Runner, 64, 0));
    let pair = AirPairWitness { master, runner };
    assert_eq!(
        pair.qualify_supported().unwrap(),
        AirClaim::FirstTransferCraft
    );
    let err = pair
        .qualify_full_cycle()
        .expect_err("second receipt without a fresh craft is not a full cycle");
    assert!(
        err.contains("no further work") || err.contains("second"),
        "{err}"
    );
}

#[test]
fn nature_staging_is_not_actual_close() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    master.observe(air_offer(AirRole::Master, 1, 0, 0));
    runner.observe(air_offer(AirRole::Runner, 1, TRADE_CAP, 0));
    runner.observe(air_offer(AirRole::Runner, 1, 0, TRADE_CAP));
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(runner.transferred_out, 0);
    assert_eq!(master.transferred_in, 0);
}

#[test]
fn nature_missing_or_wrong_partner_does_not_count() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    let mut master_offer = air_offer(AirRole::Master, 1, 0, 0);
    master_offer.trade_partner = None;
    master.observe(master_offer);
    runner.observe(air_offer(AirRole::Runner, 1, TRADE_CAP, 0));
    master.observe(air_confirm(AirRole::Master, 2, 0));
    runner.observe(air_confirm(AirRole::Runner, 2, 0));
    master.observe(air_close(AirRole::Master, 3, TRADE_CAP));
    runner.observe(air_close(AirRole::Runner, 3, 0));
    assert_eq!(master.partner_transfer_events, 0);
    assert_eq!(runner.partner_transfer_events, 1);

    let mut wrong = air_slot(AirRole::Master);
    let mut stranger = air_offer(AirRole::Master, 1, 0, 0);
    stranger.trade_partner = Some("stranger".into());
    wrong.observe(stranger);
    assert!(wrong.saw_wrong_partner);
    assert_eq!(wrong.partner_transfer_events, 0);
}

#[test]
fn nature_partnerless_new_offer_invalidates_previous_episode() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    master.observe(air_offer(AirRole::Master, 1, 0, 0));
    runner.observe(air_offer(AirRole::Runner, 1, TRADE_CAP, 0));
    master.observe(air_confirm(AirRole::Master, 2, 0));
    runner.observe(air_confirm(AirRole::Runner, 2, 0));
    let mut master_offer = air_offer(AirRole::Master, 3, 0, 0);
    let mut runner_offer = air_offer(AirRole::Runner, 3, TRADE_CAP, 0);
    master_offer.trade_partner = None;
    runner_offer.trade_partner = None;
    master.observe(master_offer);
    runner.observe(runner_offer);
    master.observe(air_offer(AirRole::Master, 4, 0, 0));
    runner.observe(air_offer(AirRole::Runner, 4, TRADE_CAP, 0));
    master.observe(air_confirm(AirRole::Master, 5, 0));
    runner.observe(air_confirm(AirRole::Runner, 5, 0));
    master.observe(air_close(AirRole::Master, 6, TRADE_CAP));
    runner.observe(air_close(AirRole::Runner, 6, 0));
    assert_eq!(master.partner_transfer_events, 0);
    assert_eq!(runner.partner_transfer_events, 0);
}

#[test]
fn nature_seed_and_bank_motion_is_not_a_transfer() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    master.observe(air_temple(AirRole::Master, 1, 0, TRADE_CAP, AIR_CRAFT_XP));
    nature_runner_bank_return(&mut runner, 2, TRADE_CAP);
    assert_eq!(master.partner_transfer_events, 0);
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(master.post_transfer_craft_events, 0);
    assert_eq!(runner.transferred_out, 0);
    assert_eq!(master.transferred_in, 0);
    let pair = AirPairWitness { master, runner };
    assert!(pair.qualify_supported().is_err());
    assert!(pair.qualify_full_cycle().is_err());
}

#[test]
fn mule_separated_consume_xp_air_after_trade_counts_once() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    assert_mule_transfer(&crafter, &mule, 1, MULE_TRADE_CAP);
    crafter.observe(mule_temple(MuleRole::Crafter, 110, 0, 0, 0));
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
    crafter.observe(mule_temple(MuleRole::Crafter, 111, 0, 0, MULE_CRAFT_XP));
    assert_eq!(crafter.craft_events, 0);
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        112,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    assert_eq!(crafter.craft_events, 1);
    assert_eq!(crafter.post_exchange_craft_events, 1);
}

#[test]
fn mule_seed_craft_excluded_from_postexchange() {
    let mut baseline = mule_baseline(MuleRole::Crafter);
    baseline.essence_unnoted = MULE_TRADE_CAP;
    let mut crafter = MuleSlotRecord::new(
        MuleRole::Crafter,
        "crafter".into(),
        "crafter".into(),
        "mule".into(),
        serde_json::Map::new(),
        baseline,
    );
    crafter.observe(mule_temple(MuleRole::Crafter, 1, 0, 0, 0));
    crafter.observe(mule_temple(MuleRole::Crafter, 2, 0, 0, MULE_CRAFT_XP));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        3,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    assert_eq!(crafter.craft_events, 1);
    assert_eq!(crafter.post_exchange_craft_events, 0);
}

#[test]
fn mule_stale_unrelated_later_delta_does_not_count() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    crafter.observe(mule_close(MuleRole::Crafter, 110, 0, 0));
    crafter.observe(mule_close(MuleRole::Crafter, 111, 0, MULE_TRADE_CAP));
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
    let mut later = mule_close(MuleRole::Crafter, 200, 0, MULE_TRADE_CAP);
    later.runecraft_xp = MULE_CRAFT_XP;
    crafter.observe(later);
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
}

#[test]
fn mule_bank_input_is_not_old_exchange_craft() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    crafter.observe(mule_temple(MuleRole::Crafter, 110, 0, 0, 0));
    crafter.observe(mule_temple(MuleRole::Crafter, 111, 0, 0, MULE_CRAFT_XP));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        112,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    assert_eq!(crafter.post_exchange_craft_events, 1);
    let mut bank = mule_close(MuleRole::Crafter, 150, MULE_TRADE_CAP, 0);
    bank.tile = Some(FALADOR_EAST);
    bank.bank_open = true;
    bank.bank_loaded = true;
    crafter.observe(bank);
    crafter.observe(mule_temple(MuleRole::Crafter, 160, 0, 0, MULE_CRAFT_XP));
    crafter.observe(mule_temple(MuleRole::Crafter, 161, 0, 0, MULE_CRAFT_XP * 2));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        162,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP * 2,
    ));
    assert_eq!(crafter.craft_events, 2);
    assert_eq!(crafter.post_exchange_craft_events, 1);
}

#[test]
fn mule_full_cycle_still_requires_second_exchange_and_craft() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    crafter.observe(mule_temple(MuleRole::Crafter, 110, 0, 0, 0));
    crafter.observe(mule_temple(MuleRole::Crafter, 111, 0, 0, MULE_CRAFT_XP));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        112,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    let mut deposit = mule_close(MuleRole::Mule, 130, 0, 0);
    deposit.tile = Some(FALADOR_EAST);
    deposit.bank_open = true;
    deposit.bank_loaded = true;
    mule.observe(deposit);
    let mut restock = mule_close(MuleRole::Mule, 132, MULE_TRADE_CAP, 0);
    restock.tile = Some(FALADOR_EAST);
    restock.bank_open = true;
    restock.bank_loaded = true;
    mule.observe(restock);
    mule.observe(mule_close(MuleRole::Mule, 134, MULE_TRADE_CAP, 0));
    let mut crafter_offer = mule_offer(MuleRole::Crafter, 140, 0, MULE_TRADE_CAP, 0);
    crafter_offer.runecraft_xp = MULE_CRAFT_XP;
    crafter.observe(crafter_offer);
    mule.observe(mule_offer(MuleRole::Mule, 140, MULE_TRADE_CAP, 0, 0));
    let mut crafter_confirm = mule_confirm(MuleRole::Crafter, 142, 0, MULE_TRADE_CAP);
    crafter_confirm.runecraft_xp = MULE_CRAFT_XP;
    crafter.observe(crafter_confirm);
    mule.observe(mule_confirm(MuleRole::Mule, 142, MULE_TRADE_CAP, 0));
    let mut crafter_close = mule_close(MuleRole::Crafter, 144, MULE_TRADE_CAP, 0);
    crafter_close.runecraft_xp = MULE_CRAFT_XP;
    crafter.observe(crafter_close);
    mule.observe(mule_close(MuleRole::Mule, 144, 0, MULE_TRADE_CAP));
    crafter.observe(mule_temple(MuleRole::Crafter, 150, 0, 0, MULE_CRAFT_XP));
    crafter.observe(mule_temple(MuleRole::Crafter, 151, 0, 0, MULE_CRAFT_XP * 2));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        152,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP * 2,
    ));
    assert_mule_transfer(&crafter, &mule, 2, MULE_TRADE_CAP * 2);
    assert!(mule.second_exchange_after_bank_return);
    assert_eq!(crafter.post_exchange_craft_events, 2);
    let pair = MulePairWitness { crafter, mule };
    assert_eq!(
        pair.qualify_full_cycle().unwrap(),
        MuleClaim::MuleBankReturnSecondCycle
    );
}
