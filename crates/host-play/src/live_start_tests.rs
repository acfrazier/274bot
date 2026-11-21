use super::*;
use crate::catalog_core::{CoreCase, Observation};
#[cfg(feature = "js")]
fn prepared_native(body: &str) -> (String, script_js::CheckedLoad) {
    let origin = script_js::native_fixture(body);
    let prepared = script_js::prepare_raw_origin_for_test("live-start.ts", &origin, &[])
        .expect("native live-start fixture prepares");
    (prepared.js, prepared.checked)
}

fn prepared_thiever() -> Observation {
    let mut prepared = Observation {
        ingame: true,
        scene_state: 2,
        player: Some("catalogtest".into()),
        tile: Some((2661, 3306, 0)),
        ..Observation::default()
    };
    prepared.levels.insert("thieving".into(), 50);
    prepared.levels.insert("hitpoints".into(), 50);
    prepared.effective_levels.insert("thieving".into(), 50);
    prepared.effective_levels.insert("hitpoints".into(), 50);
    prepared.items.insert("Lobster".into(), 10);
    prepared
}

fn empty_play() -> crate::Play {
    #[cfg(feature = "js")]
    script_js::install_cards().expect("install canonical Sherlock card for live start tests");
    crate::run_with_io(
        &crate::PlayOptions {
            host: "127.0.0.1".into(),
            transport: crate::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
}

#[cfg(feature = "js")]
#[test]
fn catalog_start_waits_without_arming_or_failing_current_session_witness() {
    let _iso = script::IsolatedEnv::enter("catalog-copy-witness");
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", Observation::default(), true);
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        None,
        vec![],
        vec![],
    )];
    let held = play.hold_script_starts("waiting for settings copy confirmation".into());
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Hold
    );
    assert!(!pending[0].started);
    assert_eq!(handle.run_state("catalogtest"), script::RunState::Idle);
    assert_eq!(watch.evidence()["phase"], "ready");
    assert!(watch.failure().is_none());
    drop(held);
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue
    );
    assert!(pending[0].started);
    assert_eq!(watch.evidence()["phase"], "running");
    let deadline = Instant::now() + Duration::from_secs(60);
    while handle.run_state("catalogtest") != script::RunState::Running {
        handle.poll_start("catalogtest");
        assert!(Instant::now() < deadline, "scenario Start did not settle");
        std::thread::yield_now();
    }
    handle.stop("catalogtest").unwrap();
}

#[cfg(feature = "js")]
#[test]
fn blank_loadout_without_saved_rows_starts_script() {
    let _iso = script::IsolatedEnv::enter("catalog-blank-loadout");
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let mut bag = Map::new();
    bag.insert("loadout".into(), Value::String(String::new()));
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        Some(bag),
        vec![],
        vec![],
    )];

    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue,
    );
    assert!(pending[0].started);
    assert_eq!(watch.evidence()["phase"], "running");
    let deadline = Instant::now() + Duration::from_secs(60);
    while handle.run_state("catalogtest") != script::RunState::Running {
        handle.poll_start("catalogtest");
        assert!(Instant::now() < deadline, "script Start did not settle");
        std::thread::yield_now();
    }
    handle.stop("catalogtest").unwrap();
}

#[cfg(feature = "js")]
#[test]
fn null_loadout_setting_uses_upstream_blank_default() {
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let mut bag = Map::new();
    bag.insert("loadout".into(), Value::Null);
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        Some(bag),
        vec![],
        vec![],
    )];

    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue,
    );
    assert!(pending[0].started);
    assert_eq!(watch.evidence()["phase"], "running");
    let deadline = Instant::now() + Duration::from_secs(60);
    while handle.run_state("catalogtest") != script::RunState::Running {
        handle.poll_start("catalogtest");
        assert!(Instant::now() < deadline, "script Start did not settle");
        std::thread::yield_now();
    }
    handle.stop("catalogtest").unwrap();
}

#[test]
fn no_slot_scenario_start_holds_and_retries_the_armed_core() {
    let play = empty_play();
    #[cfg(feature = "js")]
    let id = script::CompiledId("Sherlock");
    #[cfg(not(feature = "js"))]
    let id = script::CompiledId("Gatherer");
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let mut pending = vec![PendingCatalogStart::compiled("catalogtest", id, Map::new())];
    for _ in 0..2 {
        assert_eq!(
            fire_pending_catalog_start(&mut pending, true, false, arming),
            StartScriptPump::Hold,
        );
        assert!(!pending[0].started);
        assert!(
            pending[0]
                .waiting_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("no slot")),
            "{:?}",
            pending[0].waiting_reason
        );
        assert_eq!(watch.evidence()["phase"], "ready");
        assert!(watch.failure().is_none());
    }
}

#[cfg(feature = "js")]
#[test]
fn catalog_start_waits_for_pre_start_publication() {
    let _iso = script::IsolatedEnv::enter("catalog-start-publication");
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        None,
        vec![],
        vec![],
    )];

    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Hold
    );
    assert!(pending[0]
        .waiting_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("no published pre-Start observation")));
    assert_eq!(watch.evidence()["phase"], "ready");

    watch.observe("catalogtest", Observation::default(), false);
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Hold
    );
    assert!(pending[0]
        .waiting_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("not attached ingame scene2")));
    assert_eq!(watch.evidence()["phase"], "ready");

    watch.observe("catalogtest", prepared_thiever(), false);
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue
    );
    assert!(pending[0].started);
    let deadline = Instant::now() + Duration::from_secs(60);
    while handle.run_state("catalogtest") != script::RunState::Running {
        handle.poll_start("catalogtest");
        assert!(Instant::now() < deadline, "scenario Start did not settle");
        std::thread::yield_now();
    }
    handle.stop("catalogtest").unwrap();
}

#[cfg(feature = "js")]
#[test]
fn catalog_start_refuses_wrong_account_baseline() {
    let _iso = script::IsolatedEnv::enter("catalog-start-wrong-account");
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    let mut wrong = prepared_thiever();
    wrong.player = Some("other".into());
    watch.observe("catalogtest", wrong, false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        None,
        vec![],
        vec![],
    )];

    let pump = fire_pending_catalog_start(&mut pending, true, false, arming);
    let StartScriptPump::Failed(error) = pump else {
        panic!("expected a Failed pump, got {pump:?}");
    };
    assert!(error.contains("fresh account"), "{error}");
    assert!(!pending[0].started);
}

#[test]
fn no_slot_pair_start_restores_the_prepared_pair_for_retry() {
    let play = empty_play();
    let pair = PairWatch::default();
    pair.configure(crate::paired_core::PairCase::Flax, "runner", "spinner");
    pair.observe_flax(
        "runner",
        crate::paired_core::FlaxObservation {
            ingame: true,
            scene_state: 2,
            inventory_tab_available: true,
            player: Some("runner".into()),
            tile: Some(crate::paired_core::FLAX_FIELD),
            crafting: 1,
            ..Default::default()
        },
        false,
    );
    pair.observe_flax(
        "spinner",
        crate::paired_core::FlaxObservation {
            ingame: true,
            scene_state: 2,
            inventory_tab_available: true,
            player: Some("spinner".into()),
            tile: Some(crate::paired_core::FLAX_MEET),
            crafting: 10,
            ..Default::default()
        },
        false,
    );
    assert_eq!(pair.barrier(), crate::paired_core::StartBarrier::StartBoth);

    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        pair: Some(pair.clone()),
        ..Default::default()
    };
    let id = if cfg!(feature = "js") {
        "Sherlock"
    } else {
        "Gatherer"
    };
    let mut pending = vec![
        PendingCatalogStart::compiled("runner", script::CompiledId(id), Map::new()),
        PendingCatalogStart::compiled("spinner", script::CompiledId(id), Map::new()),
    ];
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Hold
    );
    assert!(!pending[0].started);
    assert!(
        pending[0]
            .waiting_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("no slot")),
        "{:?}",
        pending[0].waiting_reason
    );
    assert_eq!(pair.evidence()["phase"], "ready");
    assert_eq!(pair.barrier(), crate::paired_core::StartBarrier::StartBoth);
    assert!(pair.failure().is_none());
}

#[test]
fn pair_start_without_both_cards_fails_instead_of_holding() {
    let play = empty_play();
    let pair = PairWatch::default();
    pair.configure(crate::paired_core::PairCase::Flax, "runner", "spinner");
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        pair: Some(pair.clone()),
        ..Default::default()
    };
    let id = if cfg!(feature = "js") {
        "Sherlock"
    } else {
        "Gatherer"
    };
    let mut pending = vec![PendingCatalogStart::compiled(
        "runner",
        script::CompiledId(id),
        Map::new(),
    )];

    let pump = fire_pending_catalog_start(&mut pending, true, false, arming);
    let StartScriptPump::Failed(error) = pump else {
        panic!("expected a Failed pump, got {pump:?}");
    };
    assert!(
        error.contains("actual scripts on both visible slots"),
        "{error}"
    );
    // The card-count refusal happens before the pair starts its witness.
    assert_eq!(pair.evidence()["phase"], "ready");
    assert!(pair.failure().is_none(), "{:?}", pair.failure());
    assert!(!pending[0].started);
}

#[cfg(feature = "js")]
#[test]
fn unknown_loadout_refusal_fails_start() {
    // Unknown and ambiguous selections are refusals, not missing prerequisites.
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let mut bag = Map::new();
    bag.insert("loadout".into(), Value::String("missing".into()));
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        Some(bag),
        vec![],
        vec![api::loadout::Loadout::new("First")],
    )];
    let pump = fire_pending_catalog_start(&mut pending, true, false, arming);
    let StartScriptPump::Failed(error) = pump else {
        panic!("expected a Failed pump, got {pump:?}");
    };
    assert!(error.contains("loadout"), "{error}");
    assert!(watch.failure().is_some(), "{:?}", watch.failure());
    assert!(!pending[0].started);
}

#[cfg(feature = "js")]
#[test]
fn empty_loadout_store_fails_the_start_fast() {
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let mut bag = Map::new();
    bag.insert("loadout".into(), Value::String("missing".into()));
    let (js, checked) = prepared_native("export function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        Some(bag),
        vec![],
        vec![],
    )];
    let pump = fire_pending_catalog_start(&mut pending, true, false, arming);
    let StartScriptPump::Failed(error) = pump else {
        panic!("expected a Failed pump, got {pump:?}");
    };
    assert!(error.contains("no loadouts exist"), "{error}");
    assert!(
        watch
            .failure()
            .is_some_and(|failure| failure.contains("no loadouts exist")),
        "{:?}",
        watch.failure()
    );
    assert!(!pending[0].started);
}

#[cfg(feature = "js")]
#[test]
fn js_setup_failure_settles_as_a_failed_pump() {
    // A JS card admitted at Start whose setup then throws settles Failed:
    // the pump reports it so the scenario fails its StartScript step with
    // the setup diagnostic instead of watching a script that never ran.
    let _iso = script::IsolatedEnv::enter("catalog-setup-failure");
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let (js, checked) =
        prepared_native("throw new Error('zz-setup-proof');\nexport function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        None,
        vec![],
        vec![],
    )];
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue,
        "the Start is admitted before setup settles"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match fire_pending_catalog_start(&mut pending, true, false, arming) {
            StartScriptPump::Failed(error) => {
                assert!(error.contains("zz-setup-proof"), "{error}");
                break;
            }
            StartScriptPump::Continue => {
                assert!(Instant::now() < deadline, "setup failure did not settle");
                std::thread::yield_now();
            }
            other => panic!("expected a Failed settle, got {other:?}"),
        }
    }
    assert!(pending[0].settled);
}

#[cfg(feature = "js")]
#[test]
fn js_setup_failure_consumed_by_frontend_still_fails_catalog_start() {
    let _iso = script::IsolatedEnv::enter("catalog-consumed-setup-failure");
    let play = empty_play();
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);
    let handle = play.script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        catalog: Some(watch.clone()),
        ..Default::default()
    };
    let (js, checked) =
        prepared_native("throw new Error('zz-consumed-setup-proof');\nexport function tick() {}");
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        js,
        checked,
        None,
        vec![],
        vec![],
    )];
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue
    );
    // The frontend polls the same Start handle. Its earlier poll can consume
    // setup failure before the live pump sees the one-shot settlement.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match handle.poll_start("catalogtest") {
            script::StartPoll::Pending => {
                assert!(Instant::now() < deadline, "setup failure did not settle");
                std::thread::yield_now();
            }
            script::StartPoll::Settled(script::StartOutcome::Failed(error)) => {
                assert!(error.contains("zz-consumed-setup-proof"), "{error}");
                break;
            }
            other => panic!("expected the frontend to consume setup failure, got {other:?}"),
        }
    }
    assert_eq!(handle.run_state("catalogtest"), script::RunState::Idle);
    assert!(matches!(
        handle.poll_start("catalogtest"),
        script::StartPoll::NotOwed
    ));
    let pump = fire_pending_catalog_start(&mut pending, true, false, arming);
    let StartScriptPump::Failed(error) = pump else {
        panic!("expected a Failed pump after consumed setup failure, got {pump:?}");
    };
    assert!(error.contains("zz-consumed-setup-proof"), "{error}");
    assert!(pending[0].settled);
    assert!(
        watch
            .failure()
            .is_some_and(|failure| failure.contains("zz-consumed-setup-proof")),
        "{:?}",
        watch.failure()
    );
}
