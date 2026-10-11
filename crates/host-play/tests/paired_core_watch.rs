use std::sync::Arc;
use std::time::{Duration, Instant};

use api::snapshot::{Family, GameSnapshot};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
use client::config::ObjType;
use client::io::{Packet, ServerProt};

use host_play::live_gate::{
    pair_core_gate, pair_core_record, CoreGate, LiveTiming, PAIR_FIXTURE_SETUP_DEADLINE,
};
use host_play::paired_core::{
    pair_settings, AirClaim, AirExchangeStage, AirObservation, AirPairWitness, AirRole,
    AirSlotRecord, DuelObservation, FlaxExchangeStage, FlaxObservation, FlaxPairWitness, FlaxRole,
    FlaxSlotRecord, MuleClaim, MuleExchangeStage, MulePairWitness, MuleRole, MuleSlotRecord,
    PairCase, PairWatch, PairWatchStatus, StartBarrier, AIR_RUINS, DUEL_CHALLENGE_ANCHOR,
    FALADOR_EAST, FLAX_BANK, FLAX_FIELD, FLAX_ID, FLAX_MEET, FLAX_WHEEL, MULE_TRADE_CAP, TRADE_CAP,
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
        crafting_xp: match role {
            FlaxRole::Runner => 0,
            FlaxRole::Spinner => 1154,
        },
        trade_session_generation: 1,
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
    let watch = flax_replay_full_cycle(FlaxReplayTrace::fd2());
    assert_flax_full_cycle_receipt(&watch, "fd2");

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
fn duel_trainer_schema_defaults_prepare_with_native_counterpart_identity() {
    let schema = script::settings_schema_from_source(
        r#"export const SETTINGS = {
            mode: { type: 'string', default: 'Train', options: ['Train', 'Clue helper'] },
            partner: { type: 'string', default: '', showIf: { key: 'mode', anyOf: ['Clue helper'] } },
            targetAttack: { type: 'number', default: 99 },
            targetStrength: { type: 'number', default: 99 },
            targetDefence: { type: 'number', default: 1 }
        };"#,
    );
    let watch = duel_watch();
    host_play::live_start::pair_starts(
        &["alice".into(), "bob".into()],
        PairCase::Duel,
        String::new(),
        script::LoadShape::CompatClass,
        &schema,
        Vec::new(),
        &watch,
    )
    .expect("the training schema's hidden empty helper partner must not block preparation");
    watch.observe_duel("alice", duel_ready("alice"), false);
    watch.observe_duel("bob", duel_ready("bob"), false);
    watch.begin_shared_start("alice", "bob").unwrap();
    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["a"]["partner"], "bob");
    assert_eq!(evidence["witness"]["b"]["partner"], "alice");
    assert_eq!(evidence["witness"]["a"]["settings"]["partner"], "");
    assert_eq!(evidence["witness"]["a"]["settings"]["mode"], "Train");
    assert_eq!(watch.status(), PairWatchStatus::Running);
    assert!(watch.qualify().is_err(), "preparation is not combat proof");
}

#[test]
fn duel_trainer_rejects_helper_mode_and_any_populated_partner() {
    for bag in [
        serde_json::json!({"mode": "Clue helper", "partner": ""}),
        serde_json::json!({"mode": "Train", "partner": "bob"}),
        serde_json::json!({"mode": "Train", "partner": " "}),
        serde_json::json!({"mode": "Train", "partner": null}),
        serde_json::json!({"mode": "Train", "partner": 0}),
        serde_json::json!({"partner": "bob"}),
    ] {
        let watch = duel_watch();
        let bag = bag.as_object().unwrap().clone();
        assert!(
            watch
                .install_prepared_settings("alice", bag.clone(), "bob", bag)
                .is_err(),
            "a helper or explicit partner is not the native-counterpart trainer cell"
        );
    }
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
    let mut slot = FlaxSlotRecord::new(
        role,
        account.into(),
        account.into(),
        partner.into(),
        serde_json::Map::new(),
        flax_ready(account, role),
    );
    let mut after_start = flax_ready(account, role);
    after_start.tick = 1;
    after_start.tile = Some(FLAX_MEET);
    if role == FlaxRole::Runner {
        after_start.flax = FLAX_LOAD;
    }
    slot.observe(after_start);
    slot
}

#[allow(clippy::too_many_arguments)] // trade fixture packs role/tick/item counts
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
        trade_interface_generation: if offer {
            1
        } else if confirm {
            3
        } else {
            2
        },
        trade_modal_generation: if offer {
            1
        } else if confirm {
            3
        } else {
            2
        },
        trade_inventory_generation: 1,
        trade_my_offer_packet_generation: Some(1),
        trade_their_offer_packet_generation: Some(1),
        trade_my_offer_available: offer || confirm,
        trade_their_offer_available: offer || confirm,
        ..flax_ready(player, role)
    }
}

fn flax_offer_with_qty(
    role: FlaxRole,
    tick: u32,
    held_before: i32,
    quantity: i32,
    interface_generation: u64,
    inventory_generation: u64,
) -> FlaxObservation {
    let partner = match role {
        FlaxRole::Runner => "spinner",
        FlaxRole::Spinner => "runner",
    };
    let held_after_staging = match role {
        FlaxRole::Runner => held_before - quantity,
        FlaxRole::Spinner => held_before,
    };
    let mut observation = flax_trade(
        role,
        tick,
        held_after_staging,
        true,
        false,
        Some(partner),
        TRADE_OFFER_ACCEPT,
        if role == FlaxRole::Runner {
            quantity
        } else {
            0
        },
    );
    observation.trade_theirs_flax = if role == FlaxRole::Spinner {
        quantity
    } else {
        0
    };
    observation.trade_interface_generation = interface_generation;
    observation.trade_modal_generation = interface_generation;
    observation.trade_my_offer_packet_generation = Some(inventory_generation);
    observation.trade_their_offer_packet_generation = Some(inventory_generation);
    observation.trade_inventory_generation = inventory_generation;
    observation.trade_my_offer_available = true;
    observation.trade_their_offer_available = true;
    observation
}

fn flax_offer(role: FlaxRole, tick: u32, flax: i32, mine: i32) -> FlaxObservation {
    match role {
        FlaxRole::Runner => {
            let held_before = if mine > 0 { flax + mine } else { flax };
            let quantity = if mine > 0 { mine } else { flax };
            flax_offer_with_qty(role, tick, held_before, quantity, tick as u64, tick as u64)
        }
        FlaxRole::Spinner => flax_offer_with_qty(
            role,
            tick,
            flax,
            if mine > 0 { mine } else { FLAX_LOAD },
            tick as u64,
            tick as u64,
        ),
    }
}

fn flax_inactive(role: FlaxRole, tick: u32, flax: i32) -> FlaxObservation {
    let mut observation = flax_trade(role, tick, flax, false, false, None, -1, 0);
    observation.trade_interface_generation = tick as u64;
    observation.trade_inventory_generation = tick.saturating_sub(1) as u64;
    observation.trade_modal_generation = tick as u64;
    observation.trade_my_offer_packet_generation = Some(tick.saturating_sub(1) as u64);
    observation.trade_their_offer_packet_generation = Some(tick.saturating_sub(1) as u64);
    observation.trade_my_offer_available = true;
    observation.trade_their_offer_available = true;
    observation
}

fn flax_confirm_with_qty(
    role: FlaxRole,
    tick: u32,
    flax: i32,
    inventory_generation: u64,
) -> FlaxObservation {
    let partner = match role {
        FlaxRole::Runner => "spinner",
        FlaxRole::Spinner => "runner",
    };
    let mut observation = flax_trade(
        role,
        tick,
        flax,
        false,
        true,
        Some(partner),
        TRADE_CONFIRM_ACCEPT,
        0,
    );
    observation.trade_interface_generation = tick as u64;
    observation.trade_modal_generation = tick as u64 + 1;
    observation.trade_my_offer_packet_generation = Some(inventory_generation);
    observation.trade_their_offer_packet_generation = Some(inventory_generation);
    observation.trade_inventory_generation = inventory_generation;
    observation.trade_my_offer_available = true;
    observation.trade_their_offer_available = true;
    observation
}

fn flax_confirm(role: FlaxRole, tick: u32, flax: i32) -> FlaxObservation {
    let held = match role {
        FlaxRole::Runner => flax.saturating_sub(FLAX_LOAD),
        FlaxRole::Spinner => flax,
    };
    flax_confirm_with_qty(role, tick, held, tick.saturating_sub(1) as u64)
}

fn flax_close(role: FlaxRole, tick: u32, flax: i32) -> FlaxObservation {
    let mut observation = flax_trade(role, tick, flax, false, false, None, -1, 0);
    observation.trade_interface_generation = tick as u64;
    observation.trade_modal_generation = tick as u64;
    observation.trade_inventory_generation = tick as u64;
    observation
}

struct FlaxTradeEngine {
    client: Client,
    snapshot: GameSnapshot,
}

impl FlaxTradeEngine {
    fn new(partner: &str) -> Self {
        const TRADEMAIN: i32 = 3323;
        const TRADECONFIRM: i32 = 3443;
        const TRADEMY: i32 = 3415;
        const TRADETHEIR: i32 = 3416;
        const TRADEPARTNER: i32 = 3417;

        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        client.gens.session = 1;
        let cache = Arc::get_mut(&mut client.cache).expect("test client has a unique cache");
        cache.objs.resize(FLAX_ID as usize + 1, ObjType::default());
        cache.objs[FLAX_ID as usize] = ObjType {
            id: FLAX_ID,
            name: "Flax".into(),
            ..Default::default()
        };

        client.set_iface(
            TRADEMAIN as usize,
            IfType {
                id: TRADEMAIN,
                layer_id: TRADEMAIN,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![TRADEMY, TRADETHEIR, TRADEPARTNER, 3420, 3422]),
                ..Default::default()
            },
        );
        client.set_iface(
            TRADECONFIRM as usize,
            IfType {
                id: TRADECONFIRM,
                layer_id: TRADECONFIRM,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![3546, 3548]),
                ..Default::default()
            },
        );
        for component in [TRADEMY, TRADETHEIR] {
            client.set_iface(
                component as usize,
                IfType {
                    id: component,
                    layer_id: TRADEMAIN,
                    r#type: ComponentType::TYPE_INV,
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                component as usize,
                IfTypeMut {
                    link_obj_type: Some(vec![0]),
                    link_obj_number: Some(vec![0]),
                    ..Default::default()
                },
            );
        }
        client.set_iface(
            TRADEPARTNER as usize,
            IfType {
                id: TRADEPARTNER,
                layer_id: TRADEMAIN,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            TRADEPARTNER as usize,
            IfTypeMut {
                text: format!("Trading With: {partner}"),
                ..Default::default()
            },
        );
        for (component, layer, button_type) in [
            (3420, TRADEMAIN, ButtonType::BUTTON_OK),
            (3422, TRADEMAIN, ButtonType::BUTTON_CLOSE),
            (3546, TRADECONFIRM, ButtonType::BUTTON_OK),
            (3548, TRADECONFIRM, ButtonType::BUTTON_CLOSE),
        ] {
            client.set_iface(
                component,
                IfType {
                    id: component as i32,
                    layer_id: layer,
                    r#type: ComponentType::TYPE_RECT,
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                component,
                IfTypeMut {
                    button_type,
                    ..Default::default()
                },
            );
        }
        Self {
            client,
            snapshot: GameSnapshot::new(),
        }
    }

    fn dispatch(&mut self, opcode: i32, payload: Vec<u8>) {
        self.client.handle_packet(opcode, &mut Packet::new(payload));
    }

    fn open_offer(&mut self, my_flax: i32, their_flax: i32) {
        self.dispatch(ServerProt::IF_OPENMAIN, vec![12, 251]);
        self.dispatch(
            ServerProt::UPDATE_INV_FULL,
            flax_inventory_packet(3415, my_flax),
        );
        self.dispatch(
            ServerProt::UPDATE_INV_FULL,
            flax_inventory_packet(3416, their_flax),
        );
    }

    fn update_my_offer(&mut self, quantity: i32) {
        self.dispatch(
            ServerProt::UPDATE_INV_FULL,
            flax_inventory_packet(3415, quantity),
        );
    }

    fn stop_offer_containers(&mut self) {
        for component in [3415, 3416] {
            self.dispatch(
                ServerProt::UPDATE_INV_STOP_TRANSMIT,
                vec![(component >> 8) as u8, component as u8],
            );
        }
    }

    fn open_confirm(&mut self) {
        self.dispatch(ServerProt::IF_OPENMAIN, vec![13, 115]);
    }

    fn remove_my_offer_component(&mut self) {
        Arc::make_mut(&mut self.client.ifaces)[3415] = None;
    }

    fn close(&mut self) {
        self.dispatch(ServerProt::IF_CLOSE, Vec::new());
    }

    fn observation(&mut self, player: &str, tick: u32, flax: i32) -> FlaxObservation {
        assert!(
            self.snapshot.rebuild_family(&self.client, Family::Trade),
            "trade packet changes rebuild through the API snapshot adapter"
        );
        let mut observation = FlaxObservation::from_snapshot(&self.snapshot);
        observation.ingame = true;
        observation.scene_state = 2;
        observation.inventory_tab_available = true;
        observation.player = Some(player.into());
        observation.tile = Some(FLAX_MEET);
        observation.tick = tick;
        observation.flax = flax;
        observation.crafting = if player == "runner" { 1 } else { 10 };
        observation.crafting_xp = if player == "runner" { 0 } else { 1154 };
        observation
    }
}

fn flax_inventory_packet(component: i32, quantity: i32) -> Vec<u8> {
    let mut packet = vec![(component >> 8) as u8, component as u8];
    if quantity > 0 {
        let item = FLAX_ID + 1;
        packet.extend_from_slice(&[1, (item >> 8) as u8, item as u8, quantity as u8]);
    } else {
        packet.push(0);
    }
    packet
}

fn flax_observe_real_trade_offers(
    watch: &PairWatch,
) -> (FlaxTradeEngine, FlaxTradeEngine, FlaxObservation) {
    let mut runner = FlaxTradeEngine::new("spinner");
    runner.open_offer(FLAX_LOAD, 0);
    let mut spinner = FlaxTradeEngine::new("runner");
    spinner.open_offer(0, FLAX_LOAD);
    let runner_offer = runner.observation("runner", 2, 0);
    let spinner_offer = spinner.observation("spinner", 2, 0);
    watch.observe_flax("runner", runner_offer.clone(), false);
    watch.observe_flax("spinner", spinner_offer, false);
    (runner, spinner, runner_offer)
}
#[derive(Clone, Copy)]
struct FlaxReplayExchange {
    runner_offer_tick: u32,
    runner_contents_tick: u32,
    runner_gaps: [Option<u32>; 2],
    runner_confirm_tick: u32,
    runner_close_tick: u32,
    spinner_offer_tick: u32,
    spinner_contents_tick: u32,
    spinner_gaps: [Option<u32>; 2],
    spinner_confirm_tick: u32,
    spinner_close_tick: u32,
    interface_floor: u64,
    inventory_floor: u64,
    contents_inventory_generation: u64,
    close_inventory_generation: u64,
}

// Reduced actor-local timelines transcribed from LEDGER-0202's
// COMPAT-WALK-TARGET observations-fd{1,2,3}.json and live-diag.md.
// The old captures did not record packet/modal generations: the generation
// values below are explicit synthetic continuity inputs, not recovered evidence.
// Adapter tests separately exercise their derivation from client packet state.
#[derive(Clone, Copy)]
struct FlaxReplayTrace {
    name: &'static str,
    first: FlaxReplayExchange,
    second: FlaxReplayExchange,
    first_string_tick: u32,
    first_string_xp: i32,
    full_strings_tick: u32,
    full_strings_xp: i32,
    full_xp_tick: u32,
    bank_open_tick: u32,
    bank_deposit_tick: u32,
    bank_close_tick: u32,
    return_tick: u32,
}

impl FlaxReplayTrace {
    fn fd2() -> Self {
        Self {
            name: "fd2",
            first: FlaxReplayExchange {
                runner_offer_tick: 104,
                runner_contents_tick: 106,
                runner_gaps: [Some(108), None],
                runner_confirm_tick: 108,
                runner_close_tick: 111,
                spinner_offer_tick: 107,
                spinner_contents_tick: 109,
                spinner_gaps: [Some(111), None],
                spinner_confirm_tick: 111,
                spinner_close_tick: 114,
                interface_floor: 0,
                inventory_floor: 0,
                contents_inventory_generation: 1,
                close_inventory_generation: 2,
            },
            second: FlaxReplayExchange {
                runner_offer_tick: 256,
                runner_contents_tick: 256,
                runner_gaps: [Some(260), None],
                runner_confirm_tick: 260,
                runner_close_tick: 263,
                spinner_offer_tick: 259,
                spinner_contents_tick: 259,
                spinner_gaps: [Some(263), None],
                spinner_confirm_tick: 263,
                spinner_close_tick: 266,
                interface_floor: 4,
                inventory_floor: 2,
                contents_inventory_generation: 3,
                close_inventory_generation: 4,
            },
            first_string_tick: 133,
            first_string_xp: 1154,
            full_strings_tick: 216,
            full_strings_xp: 1559,
            full_xp_tick: 217,
            bank_open_tick: 241,
            bank_deposit_tick: 242,
            bank_close_tick: 243,
            return_tick: 251,
        }
    }

    fn cw2() -> Self {
        Self {
            name: "cw2",
            first: FlaxReplayExchange {
                runner_offer_tick: 109,
                runner_contents_tick: 109,
                runner_gaps: [Some(112), Some(113)],
                runner_confirm_tick: 113,
                runner_close_tick: 116,
                spinner_offer_tick: 113,
                spinner_contents_tick: 113,
                spinner_gaps: [Some(117), None],
                spinner_confirm_tick: 117,
                spinner_close_tick: 120,
                interface_floor: 0,
                inventory_floor: 0,
                contents_inventory_generation: 1,
                close_inventory_generation: 2,
            },
            second: FlaxReplayExchange {
                runner_offer_tick: 262,
                runner_contents_tick: 262,
                runner_gaps: [Some(266), None],
                runner_confirm_tick: 266,
                runner_close_tick: 269,
                spinner_offer_tick: 266,
                spinner_contents_tick: 266,
                spinner_gaps: [Some(270), None],
                spinner_confirm_tick: 270,
                spinner_close_tick: 273,
                interface_floor: 4,
                inventory_floor: 2,
                contents_inventory_generation: 3,
                close_inventory_generation: 4,
            },
            // cw2 records aggregate conversion and bank receipts, but no
            // per-row ticks for those observations; these markers preserve
            // only their observed order between the two trade sequences.
            first_string_tick: 133,
            first_string_xp: 1154,
            full_strings_tick: 216,
            full_strings_xp: 1559,
            full_xp_tick: 217,
            bank_open_tick: 249,
            bank_deposit_tick: 250,
            bank_close_tick: 251,
            return_tick: 265,
        }
    }

    fn fd1() -> Self {
        Self {
            name: "fd1",
            first: FlaxReplayExchange {
                runner_offer_tick: 108,
                runner_contents_tick: 111,
                runner_gaps: [Some(111), Some(112)],
                runner_confirm_tick: 112,
                runner_close_tick: 115,
                spinner_offer_tick: 112,
                spinner_contents_tick: 115,
                spinner_gaps: [Some(115), None],
                spinner_confirm_tick: 116,
                spinner_close_tick: 119,
                interface_floor: 0,
                inventory_floor: 0,
                contents_inventory_generation: 1,
                close_inventory_generation: 2,
            },
            second: FlaxReplayExchange {
                runner_offer_tick: 262,
                runner_contents_tick: 262,
                runner_gaps: [Some(266), None],
                runner_confirm_tick: 266,
                runner_close_tick: 269,
                spinner_offer_tick: 266,
                spinner_contents_tick: 266,
                spinner_gaps: [Some(270), None],
                spinner_confirm_tick: 270,
                spinner_close_tick: 273,
                interface_floor: 4,
                inventory_floor: 2,
                contents_inventory_generation: 3,
                close_inventory_generation: 4,
            },
            first_string_tick: 140,
            first_string_xp: 1169,
            full_strings_tick: 223,
            full_strings_xp: 1559,
            full_xp_tick: 224,
            bank_open_tick: 248,
            bank_deposit_tick: 249,
            bank_close_tick: 250,
            return_tick: 258,
        }
    }

    fn fd3() -> Self {
        Self {
            name: "fd3",
            first: FlaxReplayExchange {
                runner_offer_tick: 108,
                runner_contents_tick: 111,
                runner_gaps: [Some(111), Some(112)],
                runner_confirm_tick: 112,
                runner_close_tick: 115,
                spinner_offer_tick: 112,
                spinner_contents_tick: 115,
                spinner_gaps: [Some(115), Some(116)],
                spinner_confirm_tick: 116,
                spinner_close_tick: 119,
                interface_floor: 0,
                inventory_floor: 0,
                contents_inventory_generation: 1,
                close_inventory_generation: 2,
            },
            second: FlaxReplayExchange {
                runner_offer_tick: 260,
                runner_contents_tick: 260,
                runner_gaps: [Some(264), None],
                runner_confirm_tick: 264,
                runner_close_tick: 267,
                spinner_offer_tick: 264,
                spinner_contents_tick: 264,
                spinner_gaps: [Some(268), None],
                spinner_confirm_tick: 268,
                spinner_close_tick: 271,
                interface_floor: 4,
                inventory_floor: 2,
                contents_inventory_generation: 3,
                close_inventory_generation: 4,
            },
            first_string_tick: 140,
            first_string_xp: 1169,
            full_strings_tick: 223,
            full_strings_xp: 1559,
            full_xp_tick: 224,
            bank_open_tick: 248,
            bank_deposit_tick: 249,
            bank_close_tick: 250,
            return_tick: 258,
        }
    }
}

fn flax_replay_started_watch() -> PairWatch {
    let watch = PairWatch::default();
    watch.configure(PairCase::Flax, "runner", "spinner");
    watch.observe_flax("runner", flax_ready("runner", FlaxRole::Runner), false);
    watch.observe_flax("spinner", flax_ready("spinner", FlaxRole::Spinner), false);
    watch.begin_shared_start("runner", "spinner").unwrap();
    watch
}

fn flax_pair_watch_with_picked_pack() -> PairWatch {
    let watch = flax_replay_started_watch();
    let mut runner = flax_ready("runner", FlaxRole::Runner);
    runner.tick = 1;
    runner.flax = FLAX_LOAD;
    runner.tile = Some(FLAX_MEET);
    watch.observe_flax("runner", runner, false);
    let mut spinner = flax_ready("spinner", FlaxRole::Spinner);
    spinner.tick = 1;
    spinner.tile = Some(FLAX_MEET);
    watch.observe_flax("spinner", spinner, false);
    watch
}

#[allow(clippy::too_many_arguments)] // replay row keeps tick, counts and provenance explicit
fn flax_replay_observation(
    role: FlaxRole,
    tick: u32,
    flax: i32,
    bow_string: i32,
    crafting_xp: i32,
    tile: (i32, i32, i32),
    interface_generation: u64,
    inventory_generation: u64,
) -> FlaxObservation {
    let player = match role {
        FlaxRole::Runner => "runner",
        FlaxRole::Spinner => "spinner",
    };
    let mut observation = flax_ready(player, role);
    observation.tick = tick;
    observation.flax = flax;
    observation.bow_string = bow_string;
    observation.crafting_xp = crafting_xp;
    observation.tile = Some(tile);
    observation.trade_interface_generation = interface_generation;
    observation.trade_modal_generation = interface_generation;
    observation.trade_inventory_generation = inventory_generation;
    observation
}

#[allow(clippy::too_many_arguments)] // actor-local replay includes both roles' item baselines
fn flax_replay_actor_stage(
    watch: &PairWatch,
    exchange: FlaxReplayExchange,
    role: FlaxRole,
    runner_quantity: i32,
    spinner_quantity: i32,
    spinner_flax_before: i32,
    spinner_strings: i32,
    spinner_xp: i32,
) {
    let (offer_tick, contents_tick) = match role {
        FlaxRole::Runner => (exchange.runner_offer_tick, exchange.runner_contents_tick),
        FlaxRole::Spinner => (exchange.spinner_offer_tick, exchange.spinner_contents_tick),
    };
    let (player, partner, held_before, strings, xp) = match role {
        FlaxRole::Runner => ("runner", "spinner", runner_quantity, 0, 0),
        FlaxRole::Spinner => (
            "spinner",
            "runner",
            spinner_flax_before,
            spinner_strings,
            spinner_xp,
        ),
    };
    let pre_offer = flax_replay_observation(
        role,
        offer_tick,
        held_before,
        strings,
        xp,
        FLAX_MEET,
        exchange.interface_floor,
        exchange.inventory_floor,
    );
    watch.observe_flax(player, pre_offer, false);

    let mut offer = flax_replay_observation(
        role,
        offer_tick,
        held_before,
        strings,
        xp,
        FLAX_MEET,
        exchange.interface_floor + 1,
        exchange.inventory_floor,
    );
    offer.trade_offer_open = true;
    offer.trade_partner = Some(partner.into());
    offer.trade_accept_id = TRADE_OFFER_ACCEPT;
    watch.observe_flax(player, offer, false);

    let staged_flax = match role {
        FlaxRole::Runner => held_before - runner_quantity,
        FlaxRole::Spinner => spinner_flax_before,
    };
    let mut contents = flax_replay_observation(
        role,
        contents_tick,
        staged_flax,
        strings,
        xp,
        FLAX_MEET,
        exchange.interface_floor + 2,
        exchange.contents_inventory_generation,
    );
    contents.trade_modal_generation = exchange.interface_floor + 1;
    contents.trade_my_offer_packet_generation = Some(exchange.contents_inventory_generation);
    contents.trade_their_offer_packet_generation = Some(exchange.contents_inventory_generation);
    contents.trade_offer_open = true;
    contents.trade_partner = Some(partner.into());
    contents.trade_accept_id = TRADE_OFFER_ACCEPT;
    contents.trade_mine_flax = if role == FlaxRole::Runner {
        runner_quantity
    } else {
        0
    };
    contents.trade_theirs_flax = if role == FlaxRole::Spinner {
        spinner_quantity
    } else {
        0
    };
    contents.trade_my_offer_available = true;
    contents.trade_their_offer_available = true;
    watch.observe_flax(player, contents, false);
}

#[allow(clippy::too_many_arguments)] // actor-local replay includes delivery/cancellation outcome
fn flax_replay_actor_finish(
    watch: &PairWatch,
    exchange: FlaxReplayExchange,
    role: FlaxRole,
    runner_quantity: i32,
    spinner_quantity: i32,
    spinner_flax_before: i32,
    spinner_strings: i32,
    spinner_xp: i32,
    confirm_and_deliver: bool,
) {
    let (player, partner, gaps, confirm_tick, close_tick) = match role {
        FlaxRole::Runner => (
            "runner",
            "spinner",
            exchange.runner_gaps,
            exchange.runner_confirm_tick,
            exchange.runner_close_tick,
        ),
        FlaxRole::Spinner => (
            "spinner",
            "runner",
            exchange.spinner_gaps,
            exchange.spinner_confirm_tick,
            exchange.spinner_close_tick,
        ),
    };
    let staged_flax = match role {
        FlaxRole::Runner => 0,
        FlaxRole::Spinner => spinner_flax_before,
    };
    let strings = if role == FlaxRole::Spinner {
        spinner_strings
    } else {
        0
    };
    let xp = if role == FlaxRole::Spinner {
        spinner_xp
    } else {
        0
    };
    let content_interface = exchange.interface_floor + 2;
    let mut saw_gap = false;
    let cancelled_flax = match role {
        FlaxRole::Runner => runner_quantity,
        FlaxRole::Spinner => spinner_flax_before,
    };
    let inventory_generation = if confirm_and_deliver {
        exchange.contents_inventory_generation
    } else {
        exchange.contents_inventory_generation + 1
    };
    for tick in gaps.into_iter().flatten() {
        saw_gap = true;
        let mut gap = flax_replay_observation(
            role,
            tick,
            if confirm_and_deliver {
                staged_flax
            } else {
                cancelled_flax
            },
            strings,
            xp,
            FLAX_MEET,
            content_interface,
            inventory_generation,
        );
        gap.trade_my_offer_packet_generation = Some(exchange.contents_inventory_generation);
        gap.trade_their_offer_packet_generation = Some(exchange.contents_inventory_generation);
        gap.trade_my_offer_available = true;
        gap.trade_their_offer_available = true;
        watch.observe_flax(player, gap, false);
    }

    let accepted_flax = if confirm_and_deliver {
        staged_flax
    } else {
        if !saw_gap {
            let mut refund = flax_replay_observation(
                role,
                confirm_tick.saturating_sub(1),
                cancelled_flax,
                strings,
                xp,
                FLAX_MEET,
                content_interface,
                inventory_generation,
            );
            refund.trade_my_offer_packet_generation = Some(exchange.contents_inventory_generation);
            refund.trade_their_offer_packet_generation =
                Some(exchange.contents_inventory_generation);
            refund.trade_my_offer_available = true;
            refund.trade_their_offer_available = true;
            watch.observe_flax(player, refund, false);
        }
        cancelled_flax
    };

    let mut confirm = flax_replay_observation(
        role,
        confirm_tick,
        accepted_flax,
        strings,
        xp,
        FLAX_MEET,
        exchange.interface_floor + 3,
        inventory_generation,
    );
    confirm.trade_confirm_open = true;
    confirm.trade_partner = Some(partner.into());
    confirm.trade_accept_id = TRADE_CONFIRM_ACCEPT;
    confirm.trade_my_offer_packet_generation = Some(exchange.contents_inventory_generation);
    confirm.trade_their_offer_packet_generation = Some(exchange.contents_inventory_generation);
    confirm.trade_my_offer_available = true;
    confirm.trade_their_offer_available = true;
    watch.observe_flax(player, confirm, false);

    let delivered_flax = if confirm_and_deliver {
        match role {
            FlaxRole::Runner => 0,
            FlaxRole::Spinner => spinner_flax_before + spinner_quantity,
        }
    } else {
        accepted_flax
    };
    let close_inventory_generation = if confirm_and_deliver {
        exchange.close_inventory_generation
    } else {
        inventory_generation + 1
    };
    let close = flax_replay_observation(
        role,
        close_tick,
        delivered_flax,
        strings,
        xp,
        FLAX_MEET,
        exchange.interface_floor + 4,
        close_inventory_generation,
    );
    watch.observe_flax(player, close.clone(), false);
    // Duplicate close frames are common; they cannot create a second event.
    watch.observe_flax(player, close, false);
}

fn flax_replay_exchange(
    watch: &PairWatch,
    exchange: FlaxReplayExchange,
    runner_quantity: i32,
    spinner_quantity: i32,
    spinner_flax_before: i32,
    spinner_strings: i32,
    spinner_xp: i32,
) {
    flax_replay_actor_stage(
        watch,
        exchange,
        FlaxRole::Runner,
        runner_quantity,
        spinner_quantity,
        spinner_flax_before,
        spinner_strings,
        spinner_xp,
    );
    flax_replay_actor_stage(
        watch,
        exchange,
        FlaxRole::Spinner,
        runner_quantity,
        spinner_quantity,
        spinner_flax_before,
        spinner_strings,
        spinner_xp,
    );
    flax_replay_actor_finish(
        watch,
        exchange,
        FlaxRole::Runner,
        runner_quantity,
        spinner_quantity,
        spinner_flax_before,
        spinner_strings,
        spinner_xp,
        true,
    );
    flax_replay_actor_finish(
        watch,
        exchange,
        FlaxRole::Spinner,
        runner_quantity,
        spinner_quantity,
        spinner_flax_before,
        spinner_strings,
        spinner_xp,
        true,
    );
}

fn flax_replay_first_exchange(watch: &PairWatch, trace: FlaxReplayTrace) {
    flax_replay_exchange(watch, trace.first, FLAX_LOAD, FLAX_LOAD, 0, 0, 1154);
}

fn flax_replay_finish_cycle(watch: &PairWatch, trace: FlaxReplayTrace) {
    watch.observe_flax(
        "spinner",
        flax_replay_observation(
            FlaxRole::Spinner,
            trace.first_string_tick,
            27,
            1,
            trace.first_string_xp,
            FLAX_WHEEL,
            4,
            2,
        ),
        false,
    );
    watch.observe_flax(
        "spinner",
        flax_replay_observation(
            FlaxRole::Spinner,
            trace.full_strings_tick,
            0,
            FLAX_LOAD,
            trace.full_strings_xp,
            FLAX_WHEEL,
            4,
            2,
        ),
        false,
    );
    watch.observe_flax(
        "spinner",
        flax_replay_observation(
            FlaxRole::Spinner,
            trace.full_xp_tick,
            0,
            FLAX_LOAD,
            1574,
            FLAX_WHEEL,
            4,
            2,
        ),
        false,
    );

    let mut bank_open = flax_replay_observation(
        FlaxRole::Spinner,
        trace.bank_open_tick,
        0,
        FLAX_LOAD,
        1574,
        FLAX_BANK,
        4,
        2,
    );
    bank_open.bank_open = true;
    bank_open.bank_loaded = true;
    bank_open.bank_session_generation = 1;
    watch.observe_flax("spinner", bank_open, false);

    let mut deposit = flax_replay_observation(
        FlaxRole::Spinner,
        trace.bank_deposit_tick,
        0,
        0,
        1574,
        FLAX_BANK,
        4,
        2,
    );
    deposit.bank_open = true;
    deposit.bank_loaded = true;
    deposit.bank_session_generation = 1;
    deposit.bank_string = FLAX_LOAD;
    watch.observe_flax("spinner", deposit, false);

    let mut bank_close = flax_replay_observation(
        FlaxRole::Spinner,
        trace.bank_close_tick,
        0,
        0,
        1574,
        FLAX_BANK,
        4,
        2,
    );
    bank_close.bank_session_generation = 1;
    bank_close.bank_string = FLAX_LOAD;
    watch.observe_flax("spinner", bank_close, false);

    let mut returned = flax_replay_observation(
        FlaxRole::Spinner,
        trace.return_tick,
        0,
        0,
        1574,
        FLAX_MEET,
        4,
        2,
    );
    returned.bank_session_generation = 1;
    returned.bank_string = FLAX_LOAD;
    watch.observe_flax("spinner", returned, false);

    flax_replay_exchange(watch, trace.second, FLAX_LOAD, FLAX_LOAD, 0, 0, 1574);
}

fn flax_replay_full_cycle(trace: FlaxReplayTrace) -> PairWatch {
    let watch = flax_replay_started_watch();
    flax_replay_first_exchange(&watch, trace);
    flax_replay_finish_cycle(&watch, trace);
    watch
}

fn assert_flax_full_cycle_receipt(watch: &PairWatch, name: &str) {
    assert_eq!(watch.status(), PairWatchStatus::Qualified, "{name}");
    let receipt = watch.qualify().expect("full flax receipt");
    assert_eq!(receipt["supported"], "first-flax-transfer", "{name}");
    assert_eq!(receipt["full"], "spin-bank-second-delivery", "{name}");
    assert_eq!(receipt["runner"]["transferred_out"], 56, "{name}");
    assert_eq!(receipt["spinner"]["transferred_in"], 56, "{name}");
    assert_eq!(receipt["runner"]["partner_transfer_events"], 2, "{name}");
    assert_eq!(receipt["spinner"]["partner_transfer_events"], 2, "{name}");
    assert_eq!(
        receipt["runner"]["exchange_outcomes"],
        serde_json::json!([28, 28]),
        "{name}"
    );
    assert_eq!(
        receipt["spinner"]["exchange_incomes"],
        serde_json::json!([28, 28]),
        "{name}"
    );
    assert_eq!(receipt["spinner"]["string_from_script"], 28, "{name}");
    assert_eq!(receipt["spinner"]["xp_from_script"], 420, "{name}");
    assert_eq!(
        receipt["spinner"]["physical_string_deposit_observed"], true,
        "{name}"
    );
    assert_eq!(receipt["spinner"]["deposited_strings"], true, "{name}");
    assert_eq!(receipt["spinner"]["returned_to_meet"], true, "{name}");
    assert_eq!(
        receipt["spinner"]["second_delivery_after_bank_return"], true,
        "{name}"
    );
}

fn flax_replay_cancelled_exchange(watch: &PairWatch, trace: FlaxReplayTrace) {
    for role in [FlaxRole::Runner, FlaxRole::Spinner] {
        flax_replay_actor_finish(
            watch,
            trace.first,
            role,
            FLAX_LOAD,
            FLAX_LOAD,
            0,
            0,
            1154,
            false,
        );
    }
}

fn flax_replay_staged_exchange(watch: &PairWatch, trace: FlaxReplayTrace) {
    for role in [FlaxRole::Runner, FlaxRole::Spinner] {
        flax_replay_actor_stage(watch, trace.first, role, FLAX_LOAD, FLAX_LOAD, 0, 0, 1154);
    }
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

#[allow(clippy::too_many_arguments)] // trade fixture packs role/tick/item counts
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
    runner.observe(flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0));
    spinner.observe(flax_offer(FlaxRole::Spinner, 1, 0, 0));
    runner.observe(flax_inactive(FlaxRole::Runner, 2, 0));
    spinner.observe(flax_inactive(FlaxRole::Spinner, 2, 0));
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(spinner.partner_transfer_events, 0);
    assert!(!runner.saw_confirm_with_partner);
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_close(FlaxRole::Runner, 3, 0));
    spinner.observe(flax_close(FlaxRole::Spinner, 3, FLAX_LOAD));
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
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
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
    // The first staged load is cancelled/refunded; a new 20-flax offer must
    // bind against the held pack and fresh generations, not inherit old data.
    runner.observe(flax_inactive(FlaxRole::Runner, 2, 20));
    spinner.observe(flax_inactive(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_offer_with_qty(FlaxRole::Runner, 3, 20, 20, 3, 3));
    spinner.observe(flax_offer_with_qty(FlaxRole::Spinner, 3, 0, 20, 3, 3));
    runner.observe(flax_confirm_with_qty(FlaxRole::Runner, 4, 0, 3));
    spinner.observe(flax_confirm_with_qty(FlaxRole::Spinner, 4, 0, 3));
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
    runner.observe(flax_confirm(FlaxRole::Runner, 3, FLAX_LOAD));
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
    runner.observe(flax_confirm(FlaxRole::Runner, 2, FLAX_LOAD));
    spinner.observe(flax_confirm(FlaxRole::Spinner, 2, 0));
    runner.observe(flax_replay_observation(
        FlaxRole::Runner,
        3,
        20,
        0,
        0,
        FLAX_MEET,
        2,
        2,
    ));
    runner.observe(flax_offer(FlaxRole::Runner, 3, 0, 20));
    // The replacement modal is newer than the previous confirm (generation 3).
    spinner.observe(flax_offer_with_qty(FlaxRole::Spinner, 3, 0, 20, 4, 3));
    assert_eq!(runner.partner_transfer_events, 0);
    assert_eq!(spinner.partner_transfer_events, 0);
    assert_eq!(runner.exchange_stage, FlaxExchangeStage::Confirm);
    runner.observe(flax_confirm_with_qty(FlaxRole::Runner, 4, 0, 3));
    spinner.observe(flax_confirm_with_qty(FlaxRole::Spinner, 4, 0, 3));
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
    let trace = FlaxReplayTrace::fd2();
    let watch = flax_replay_started_watch();
    flax_replay_first_exchange(&watch, trace);
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
fn flax_live_cw2_asymmetric_gap_replays_full_cycle() {
    let trace = FlaxReplayTrace::cw2();
    let watch = flax_replay_started_watch();
    flax_replay_first_exchange(&watch, trace);
    let first = watch.evidence();
    assert_eq!(
        first["witness"]["runner"]["transferred_out"], 28,
        "{}",
        trace.name
    );
    assert_eq!(
        first["witness"]["spinner"]["transferred_in"], 28,
        "{}",
        trace.name
    );
    assert_eq!(
        first["witness"]["runner"]["partner_transfer_events"], 1,
        "{}",
        trace.name
    );
    assert_eq!(
        first["witness"]["spinner"]["partner_transfer_events"], 1,
        "{}",
        trace.name
    );
    assert_eq!(watch.status(), PairWatchStatus::Running);

    flax_replay_finish_cycle(&watch, trace);
    assert_flax_full_cycle_receipt(&watch, trace.name);
}

#[test]
fn flax_live_fd1_fd3_gaps_link_first_delivery_to_bank_and_return() {
    for trace in [FlaxReplayTrace::fd1(), FlaxReplayTrace::fd3()] {
        let watch = flax_replay_started_watch();
        flax_replay_first_exchange(&watch, trace);
        let first = watch.evidence();
        assert_eq!(
            first["witness"]["runner"]["transferred_out"], 28,
            "{}",
            trace.name
        );
        assert_eq!(
            first["witness"]["spinner"]["transferred_in"], 28,
            "{}",
            trace.name
        );

        flax_replay_finish_cycle(&watch, trace);
        assert_flax_full_cycle_receipt(&watch, trace.name);
    }
}

#[test]
fn flax_live_prompt_replay_uses_original_deadline_and_cached_receipt() {
    let trace = FlaxReplayTrace::fd2();
    let setup_started = Instant::now();
    let core_deadline = setup_started + Duration::from_secs(180);
    let mut timing = LiveTiming::new(setup_started, Some(core_deadline));
    assert_eq!(
        timing.setup_deadline(PAIR_FIXTURE_SETUP_DEADLINE),
        setup_started + PAIR_FIXTURE_SETUP_DEADLINE
    );
    timing.mark_script_started(setup_started + Duration::from_secs(2));
    timing.mark_script_started(setup_started + Duration::from_secs(30));
    assert_eq!(timing.setup_elapsed(core_deadline), Duration::from_secs(2));
    assert_eq!(
        timing.script_elapsed(core_deadline),
        Some(Duration::from_secs(178))
    );

    let watch = flax_replay_started_watch();
    flax_replay_first_exchange(&watch, trace);
    assert!(matches!(
        pair_core_gate(
            Some(&watch),
            Some(core_deadline),
            setup_started + Duration::from_secs(179)
        ),
        CoreGate::Pending
    ));
    flax_replay_finish_cycle(&watch, trace);

    let gate = pair_core_gate(Some(&watch), Some(core_deadline), core_deadline);
    assert!(matches!(&gate, CoreGate::Qualified(None)));
    let text = pair_core_record(Some(&watch), &gate).expect("cached pair receipt");
    let receipt: serde_json::Value = serde_json::from_str(&text).expect("serialized pair receipt");
    assert_eq!(receipt["supported"], "first-flax-transfer");
    assert_eq!(receipt["full"], "spin-bank-second-delivery");
    assert_flax_full_cycle_receipt(&watch, trace.name);
}

#[test]
fn flax_live_late_setup_fails_at_unchanged_deadline() {
    let trace = FlaxReplayTrace::fd2();
    let setup_started = Instant::now();
    let core_deadline = setup_started + Duration::from_secs(180);
    let mut timing = LiveTiming::new(setup_started, Some(core_deadline));
    assert_eq!(
        timing.setup_deadline(PAIR_FIXTURE_SETUP_DEADLINE),
        setup_started + PAIR_FIXTURE_SETUP_DEADLINE
    );
    assert!(timing.setup_expired(
        PAIR_FIXTURE_SETUP_DEADLINE,
        setup_started + PAIR_FIXTURE_SETUP_DEADLINE
    ));
    let script_started = setup_started + Duration::from_secs(119);
    assert!(!timing.setup_expired(PAIR_FIXTURE_SETUP_DEADLINE, script_started));
    timing.mark_script_started(script_started);
    assert!(!timing.setup_expired(
        PAIR_FIXTURE_SETUP_DEADLINE,
        setup_started + PAIR_FIXTURE_SETUP_DEADLINE
    ));
    assert_eq!(
        timing.setup_elapsed(core_deadline),
        Duration::from_secs(119)
    );

    let watch = flax_replay_started_watch();
    flax_replay_first_exchange(&watch, trace);
    assert_eq!(watch.status(), PairWatchStatus::Running);
    assert!(matches!(
        pair_core_gate(
            Some(&watch),
            Some(core_deadline),
            setup_started + Duration::from_secs(179)
        ),
        CoreGate::Pending
    ));
    let gate = pair_core_gate(Some(&watch), Some(core_deadline), core_deadline);
    let CoreGate::Failed(error) = gate else {
        panic!("incomplete late cycle must fail at the original core deadline: {gate:?}");
    };
    assert!(error.contains("before the headed deadline"), "{error}");
    assert_eq!(core_deadline, setup_started + Duration::from_secs(180));
    assert_eq!(
        timing.script_elapsed(core_deadline),
        Some(Duration::from_secs(61))
    );
    assert_eq!(watch.status(), PairWatchStatus::Running);
    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 1);
    assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 1);
    assert_eq!(evidence["witness"]["spinner"]["deposited_strings"], false);
}

#[test]
fn flax_pair_watch_rejects_staging_cancelled_refund_and_one_sided_confirmation() {
    let trace = FlaxReplayTrace::fd2();
    let staged = flax_replay_started_watch();
    flax_replay_staged_exchange(&staged, trace);
    let staged_evidence = staged.evidence();
    assert_eq!(staged_evidence["witness"]["runner"]["transferred_out"], 0);
    assert_eq!(staged_evidence["witness"]["spinner"]["transferred_in"], 0);
    assert_eq!(
        staged_evidence["witness"]["runner"]["partner_transfer_events"],
        0
    );
    assert_eq!(
        staged_evidence["witness"]["spinner"]["partner_transfer_events"],
        0
    );

    let cancelled = flax_replay_started_watch();
    flax_replay_staged_exchange(&cancelled, trace);
    flax_replay_cancelled_exchange(&cancelled, trace);
    let cancelled_evidence = cancelled.evidence();
    assert_eq!(
        cancelled_evidence["witness"]["runner"]["partner_transfer_events"],
        0
    );
    assert_eq!(
        cancelled_evidence["witness"]["spinner"]["partner_transfer_events"],
        0
    );
    assert!(cancelled.qualify().is_err());

    let one_sided = flax_replay_started_watch();
    flax_replay_staged_exchange(&one_sided, trace);
    flax_replay_actor_finish(
        &one_sided,
        trace.first,
        FlaxRole::Runner,
        FLAX_LOAD,
        FLAX_LOAD,
        0,
        0,
        1154,
        true,
    );
    flax_replay_actor_finish(
        &one_sided,
        trace.first,
        FlaxRole::Spinner,
        FLAX_LOAD,
        FLAX_LOAD,
        0,
        0,
        1154,
        false,
    );
    let one_sided_evidence = one_sided.evidence();
    assert_eq!(
        one_sided_evidence["witness"]["runner"]["transferred_out"],
        28
    );
    assert_eq!(
        one_sided_evidence["witness"]["spinner"]["transferred_in"],
        0
    );
    assert_eq!(
        one_sided_evidence["witness"]["runner"]["partner_transfer_events"],
        1
    );
    assert_eq!(
        one_sided_evidence["witness"]["spinner"]["partner_transfer_events"],
        0
    );
    assert!(one_sided.qualify().is_err());
}

#[test]
fn flax_unavailable_or_stale_offer_contents_cannot_prove_delivery() {
    for unavailable in [true, false] {
        let mut runner = flax_slot(FlaxRole::Runner);
        let mut spinner = flax_slot(FlaxRole::Spinner);
        let mut runner_offer = if unavailable {
            flax_offer(FlaxRole::Runner, 1, FLAX_LOAD, 0)
        } else {
            flax_offer_with_qty(FlaxRole::Runner, 1, FLAX_LOAD, FLAX_LOAD, 1, 0)
        };
        let mut spinner_offer = if unavailable {
            flax_offer(FlaxRole::Spinner, 1, 0, 0)
        } else {
            flax_offer_with_qty(FlaxRole::Spinner, 1, 0, FLAX_LOAD, 1, 0)
        };
        if unavailable {
            runner_offer.trade_my_offer_available = false;
            runner_offer.trade_their_offer_available = false;
            spinner_offer.trade_my_offer_available = false;
            spinner_offer.trade_their_offer_available = false;
        }
        runner.observe(runner_offer);
        spinner.observe(spinner_offer);
        runner.observe(flax_confirm_with_qty(FlaxRole::Runner, 2, 0, 1));
        spinner.observe(flax_confirm_with_qty(FlaxRole::Spinner, 2, 0, 1));
        runner.observe(flax_close(FlaxRole::Runner, 3, 0));
        spinner.observe(flax_close(FlaxRole::Spinner, 3, FLAX_LOAD));

        assert_flax_no_transfer(&runner, &spinner, false);
        let pair = FlaxPairWitness { runner, spinner };
        assert!(pair.qualify_supported().is_err());
    }
}

#[test]
fn flax_pair_watch_rejects_old_offer_packets_after_unrelated_inventory_update() {
    let watch = PairWatch::default();
    watch.configure(PairCase::Flax, "runner", "spinner");
    let mut runner_baseline = flax_ready("runner", FlaxRole::Runner);
    runner_baseline.trade_inventory_generation = 10;
    runner_baseline.trade_my_offer_packet_generation = Some(4);
    runner_baseline.trade_their_offer_packet_generation = Some(9);
    runner_baseline.trade_my_offer_available = true;
    runner_baseline.trade_their_offer_available = true;
    let mut spinner_baseline = flax_ready("spinner", FlaxRole::Spinner);
    spinner_baseline.trade_inventory_generation = 10;
    spinner_baseline.trade_my_offer_packet_generation = Some(6);
    spinner_baseline.trade_their_offer_packet_generation = Some(8);
    spinner_baseline.trade_my_offer_available = true;
    spinner_baseline.trade_their_offer_available = true;
    watch.observe_flax("runner", runner_baseline.clone(), false);
    watch.observe_flax("spinner", spinner_baseline.clone(), false);
    watch.begin_shared_start("runner", "spinner").unwrap();

    let mut runner_after_start = runner_baseline.clone();
    runner_after_start.tick = 1;
    runner_after_start.flax = FLAX_LOAD;
    let mut spinner_after_start = spinner_baseline.clone();
    spinner_after_start.tick = 1;
    watch.observe_flax("runner", runner_after_start.clone(), false);
    watch.observe_flax("spinner", spinner_after_start, false);

    let mut runner_unrelated_update = runner_after_start;
    runner_unrelated_update.tick = 2;
    runner_unrelated_update.trade_inventory_generation = 11;
    let mut spinner_unrelated_update = spinner_baseline;
    spinner_unrelated_update.tick = 2;
    spinner_unrelated_update.trade_inventory_generation = 11;
    watch.observe_flax("runner", runner_unrelated_update, false);
    watch.observe_flax("spinner", spinner_unrelated_update, false);

    let mut runner_offer = flax_offer_with_qty(FlaxRole::Runner, 3, FLAX_LOAD, FLAX_LOAD, 1, 11);
    runner_offer.trade_my_offer_packet_generation = Some(4);
    runner_offer.trade_their_offer_packet_generation = Some(9);
    let mut spinner_offer = flax_offer_with_qty(FlaxRole::Spinner, 3, 0, FLAX_LOAD, 1, 11);
    spinner_offer.trade_my_offer_packet_generation = Some(6);
    spinner_offer.trade_their_offer_packet_generation = Some(8);
    watch.observe_flax("runner", runner_offer, false);
    watch.observe_flax("spinner", spinner_offer, false);

    watch.observe_flax(
        "runner",
        flax_confirm_with_qty(FlaxRole::Runner, 4, 0, 11),
        false,
    );
    watch.observe_flax(
        "spinner",
        flax_confirm_with_qty(FlaxRole::Spinner, 4, 0, 11),
        false,
    );
    watch.observe_flax("runner", flax_close(FlaxRole::Runner, 5, 0), false);
    watch.observe_flax(
        "spinner",
        flax_close(FlaxRole::Spinner, 5, FLAX_LOAD),
        false,
    );

    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["runner"]["transferred_out"], 0);
    assert_eq!(evidence["witness"]["spinner"]["transferred_in"], 0);
    assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 0);
    assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 0);
    assert!(watch.qualify().is_err());
}

#[test]
fn flax_pair_watch_accepts_real_stop_transmit_handoff_before_confirm() {
    let watch = flax_pair_watch_with_picked_pack();
    let (mut runner, mut spinner, runner_offer) = flax_observe_real_trade_offers(&watch);

    assert_eq!(runner_offer.trade_my_offer_packet_generation, Some(1));
    assert_eq!(runner_offer.trade_their_offer_packet_generation, Some(1));
    assert!(runner_offer.trade_my_offer_available);
    assert!(runner_offer.trade_their_offer_available);
    assert_eq!(runner_offer.trade_mine_flax, FLAX_LOAD);
    assert_eq!(runner_offer.trade_theirs_flax, 0);

    runner.stop_offer_containers();
    spinner.stop_offer_containers();
    runner.open_confirm();
    spinner.open_confirm();
    let runner_confirm = runner.observation("runner", 3, 0);
    let spinner_confirm = spinner.observation("spinner", 3, 0);
    assert_eq!(runner_confirm.trade_modal_generation, 2);
    assert_eq!(runner_confirm.trade_my_offer_packet_generation, Some(2));
    assert_eq!(runner_confirm.trade_their_offer_packet_generation, Some(2));
    assert!(!runner_confirm.trade_my_offer_available);
    assert!(!runner_confirm.trade_their_offer_available);
    assert!(runner_confirm.trade_my_offer_stopped);
    assert!(runner_confirm.trade_their_offer_stopped);
    watch.observe_flax("runner", runner_confirm, false);
    watch.observe_flax("spinner", spinner_confirm, false);

    runner.close();
    spinner.close();
    watch.observe_flax("runner", runner.observation("runner", 4, 0), false);
    watch.observe_flax(
        "spinner",
        spinner.observation("spinner", 4, FLAX_LOAD),
        false,
    );
    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["runner"]["transferred_out"], FLAX_LOAD);
    assert_eq!(evidence["witness"]["spinner"]["transferred_in"], FLAX_LOAD);
    assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 1);
    assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 1);
}

#[test]
fn flax_pair_watch_rejects_content_mutation_followed_by_stop_transmit() {
    let watch = flax_pair_watch_with_picked_pack();
    let (mut runner, mut spinner, _) = flax_observe_real_trade_offers(&watch);

    runner.update_my_offer(FLAX_LOAD - 1);
    assert_eq!(
        runner.observation("runner", 3, 0).trade_mine_flax,
        FLAX_LOAD - 1
    );
    runner.stop_offer_containers();
    spinner.stop_offer_containers();
    runner.open_confirm();
    spinner.open_confirm();
    let runner_confirm = runner.observation("runner", 3, 0);
    assert_eq!(runner_confirm.trade_my_offer_packet_generation, Some(3));
    assert!(runner_confirm.trade_my_offer_stopped);
    watch.observe_flax("runner", runner_confirm, false);
    watch.observe_flax("spinner", spinner.observation("spinner", 3, 0), false);

    runner.close();
    spinner.close();
    watch.observe_flax("runner", runner.observation("runner", 4, 0), false);
    watch.observe_flax("spinner", spinner.observation("spinner", 4, 0), false);
    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 0);
    assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 0);
    assert!(watch.qualify().is_err());
}

#[test]
fn flax_pair_watch_rejects_missing_offer_source_at_confirm() {
    let watch = flax_pair_watch_with_picked_pack();
    let (mut runner, mut spinner, _) = flax_observe_real_trade_offers(&watch);

    runner.remove_my_offer_component();
    runner.open_confirm();
    spinner.open_confirm();
    let runner_confirm = runner.observation("runner", 3, 0);
    assert_eq!(runner_confirm.trade_my_offer_packet_generation, Some(1));
    assert!(!runner_confirm.trade_my_offer_available);
    assert!(!runner_confirm.trade_my_offer_stopped);
    watch.observe_flax("runner", runner_confirm, false);
    watch.observe_flax("spinner", spinner.observation("spinner", 3, 0), false);

    runner.close();
    spinner.close();
    watch.observe_flax("runner", runner.observation("runner", 4, 0), false);
    watch.observe_flax("spinner", spinner.observation("spinner", 4, 0), false);
    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 0);
    assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 0);
    assert!(watch.qualify().is_err());
}

#[test]
fn flax_changed_identity_or_trade_session_drops_pending_episode() {
    for changed_session in [false, true] {
        let trace = FlaxReplayTrace::fd2();
        let watch = flax_replay_started_watch();
        flax_replay_staged_exchange(&watch, trace);
        let mut broken = flax_replay_observation(FlaxRole::Runner, 107, 0, 0, 0, FLAX_MEET, 2, 1);
        if changed_session {
            broken.trade_session_generation = 2;
        } else {
            broken.player = Some("replacement".into());
        }
        watch.observe_flax("runner", broken, false);
        flax_replay_actor_finish(
            &watch,
            trace.first,
            FlaxRole::Runner,
            FLAX_LOAD,
            FLAX_LOAD,
            0,
            0,
            1154,
            true,
        );
        flax_replay_actor_finish(
            &watch,
            trace.first,
            FlaxRole::Spinner,
            FLAX_LOAD,
            FLAX_LOAD,
            0,
            0,
            1154,
            true,
        );
        let evidence = watch.evidence();
        assert_eq!(evidence["witness"]["runner"]["partner_transfer_events"], 0);
        assert_eq!(evidence["witness"]["spinner"]["partner_transfer_events"], 1);
        let error = watch.qualify().unwrap_err();
        if changed_session {
            assert!(error.contains("stale session"), "{error}");
        } else {
            assert!(error.contains("mixed identities"), "{error}");
        }
    }
}

#[test]
fn flax_offsetting_exchange_mismatches_fail_per_delivery_conservation() {
    let trace = FlaxReplayTrace::fd1();
    let watch = flax_replay_started_watch();
    flax_replay_exchange(&watch, trace.first, 28, 27, 0, 0, 1154);
    watch.observe_flax(
        "spinner",
        flax_replay_observation(FlaxRole::Spinner, 200, 0, 27, 1559, FLAX_WHEEL, 4, 2),
        false,
    );
    flax_replay_exchange(&watch, trace.second, 27, 28, 0, 27, 1559);

    let evidence = watch.evidence();
    assert_eq!(evidence["witness"]["runner"]["transferred_out"], 55);
    assert_eq!(evidence["witness"]["spinner"]["transferred_in"], 55);
    assert_eq!(
        evidence["witness"]["runner"]["exchange_outcomes"],
        serde_json::json!([28, 27])
    );
    assert_eq!(
        evidence["witness"]["spinner"]["exchange_incomes"],
        serde_json::json!([27, 28])
    );
    let error = watch.qualify().unwrap_err();
    assert!(error.contains("exchange 1"), "{error}");
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
            runner.observe(flax_inactive(FlaxRole::Runner, 7, FLAX_LOAD));
            spinner.observe(flax_inactive(FlaxRole::Spinner, 7, 0));
            runner.observe(flax_offer(FlaxRole::Runner, 8, FLAX_LOAD, 0));
            spinner.observe(flax_offer(FlaxRole::Spinner, 8, 0, 0));
            runner.observe(flax_confirm(FlaxRole::Runner, 9, FLAX_LOAD));
            spinner.observe(flax_confirm(FlaxRole::Spinner, 9, 0));
            runner.observe(flax_close(FlaxRole::Runner, 10, 0));
            spinner.observe(flax_close(FlaxRole::Spinner, 10, FLAX_LOAD));
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

#[allow(clippy::too_many_arguments)] // trade fixture packs role/tick/item counts
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

fn air_loading(
    role: AirRole,
    tick: u32,
    essence: i32,
    air: i32,
    xp: i32,
    in_temple: bool,
) -> AirObservation {
    let mut observation = if in_temple {
        air_temple(role, tick, essence, air, xp)
    } else {
        air_keep(air_close(role, tick, essence), air, xp)
    };
    observation.scene_state = 1;
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

fn mule_loading(
    role: MuleRole,
    tick: u32,
    essence: i32,
    air: i32,
    xp: i32,
    in_temple: bool,
) -> AirObservation {
    let mut observation = if in_temple {
        mule_temple(role, tick, essence, air, xp)
    } else {
        let mut ruins = mule_close(role, tick, essence, air);
        ruins.runecraft_xp = xp;
        ruins
    };
    observation.scene_state = 1;
    observation
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

#[test]
fn nature_pending_craft_expires_on_leaving_temple() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    master.observe(air_temple(AirRole::Master, 20, 0, 0, 0));
    // A later product update outside the temple cannot complete that episode.
    master.observe(air_keep(
        air_close(AirRole::Master, 21, 0),
        TRADE_CAP,
        AIR_CRAFT_XP,
    ));
    assert_eq!(master.craft_events, 0);
    assert_eq!(master.post_transfer_craft_events, 0);
    // Returning with the same old fields cannot resurrect the consumed input.
    master.observe(air_temple(AirRole::Master, 22, 0, TRADE_CAP, AIR_CRAFT_XP));
    assert_eq!(master.craft_events, 0);
    assert_eq!(master.post_transfer_craft_events, 0);
}

#[test]
fn mule_pending_craft_expires_on_leaving_temple() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    crafter.observe(mule_temple(MuleRole::Crafter, 110, 0, 0, 0));
    let mut outside = mule_close(MuleRole::Crafter, 111, 0, MULE_TRADE_CAP);
    outside.runecraft_xp = MULE_CRAFT_XP;
    crafter.observe(outside);
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        112,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
}

#[test]
fn nature_transfer_scene1_stable_temple_craft_counts_receipt() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    assert_eq!(master.craft_input_from_exchange, TRADE_CAP);
    // Native altar enter publishes LoadingScene while the last ruins tile is
    // still current, then a temple frame whose inventory can read empty.
    master.observe(air_loading(AirRole::Master, 17, TRADE_CAP, 0, 0, false));
    master.observe(air_loading(AirRole::Master, 18, 0, 0, 0, true));
    assert_eq!(master.craft_input_from_exchange, TRADE_CAP);
    assert_eq!(master.craft_events, 0);
    assert_eq!(master.post_transfer_craft_events, 0);
    master.observe(air_temple(AirRole::Master, 19, TRADE_CAP, 0, 0));
    master.observe(air_temple(AirRole::Master, 20, 0, 0, 0));
    master.observe(air_temple(AirRole::Master, 21, 0, 0, AIR_CRAFT_XP));
    master.observe(air_temple(AirRole::Master, 22, 0, TRADE_CAP, AIR_CRAFT_XP));
    assert_eq!(master.craft_events, 1);
    assert_eq!(master.post_transfer_craft_events, 1);
}

#[test]
fn mule_transfer_scene1_stable_temple_craft_counts_receipt() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    assert_eq!(crafter.craft_input_from_exchange, MULE_TRADE_CAP);
    crafter.observe(mule_loading(
        MuleRole::Crafter,
        101,
        MULE_TRADE_CAP,
        0,
        0,
        false,
    ));
    crafter.observe(mule_loading(MuleRole::Crafter, 102, 0, 0, 0, true));
    assert_eq!(crafter.craft_input_from_exchange, MULE_TRADE_CAP);
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
    crafter.observe(mule_temple(MuleRole::Crafter, 110, MULE_TRADE_CAP, 0, 0));
    crafter.observe(mule_temple(MuleRole::Crafter, 111, 0, 0, 0));
    crafter.observe(mule_temple(MuleRole::Crafter, 112, 0, 0, MULE_CRAFT_XP));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        113,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    assert_eq!(crafter.craft_events, 1);
    assert_eq!(crafter.post_exchange_craft_events, 1);
}

#[test]
fn nature_pending_craft_expires_on_scene_loading() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    master.observe(air_temple(AirRole::Master, 20, 0, 0, 0));
    master.observe(air_loading(AirRole::Master, 21, 0, 0, 0, true));
    master.observe(air_temple(AirRole::Master, 22, 0, TRADE_CAP, AIR_CRAFT_XP));
    assert_eq!(master.craft_events, 0);
    assert_eq!(master.post_transfer_craft_events, 0);
}

#[test]
fn mule_pending_craft_expires_on_scene_loading() {
    let mut crafter = mule_slot(MuleRole::Crafter);
    let mut mule = mule_slot(MuleRole::Mule);
    mule_gap_close(&mut crafter, &mut mule);
    crafter.observe(mule_temple(MuleRole::Crafter, 110, 0, 0, 0));
    crafter.observe(mule_loading(MuleRole::Crafter, 111, 0, 0, 0, true));
    crafter.observe(mule_temple(
        MuleRole::Crafter,
        112,
        0,
        MULE_TRADE_CAP,
        MULE_CRAFT_XP,
    ));
    assert_eq!(crafter.craft_events, 0);
    assert_eq!(crafter.post_exchange_craft_events, 0);
}

#[test]
fn nature_loading_without_consume_is_not_craft() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    master.observe(air_loading(AirRole::Master, 17, TRADE_CAP, 0, 0, false));
    master.observe(air_temple(AirRole::Master, 19, TRADE_CAP, 0, 0));
    assert_eq!(master.craft_events, 0);
    assert_eq!(master.post_transfer_craft_events, 0);
}

#[test]
fn nature_disconnect_after_receipt_drops_credit() {
    let mut master = air_slot(AirRole::Master);
    let mut runner = air_slot(AirRole::Runner);
    nature_first_close(&mut master, &mut runner);
    let mut gone = air_close(AirRole::Master, 17, TRADE_CAP);
    gone.ingame = false;
    gone.scene_state = 0;
    master.observe(gone);
    master.observe(air_temple(AirRole::Master, 20, 0, 0, 0));
    master.observe(air_temple(AirRole::Master, 21, 0, 0, AIR_CRAFT_XP));
    master.observe(air_temple(AirRole::Master, 22, 0, TRADE_CAP, AIR_CRAFT_XP));
    assert_eq!(master.craft_events, 1);
    assert_eq!(master.post_transfer_craft_events, 0);
}
