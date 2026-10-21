use super::*;
use crate::catalog_core::{CoreCase, Observation};

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
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        "export function tick() {}".into(),
        script::LoadShape::NativeTick,
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

#[test]
fn no_slot_scenario_start_holds_and_retries_the_armed_core() {
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
    let mut pending = vec![PendingCatalogStart::compiled(
        "catalogtest",
        script::CompiledId("Sherlock"),
        Map::new(),
    )];
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

#[test]
fn unknown_loadout_refusal_holds_and_retries() {
    // Unknown and ambiguous selections are ordinary refusals; only an empty
    // saved-loadout store is a missing prerequisite.
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
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        "export function tick() {}".into(),
        script::LoadShape::NativeTick,
        Some(bag),
        vec![],
        vec![api::loadout::Loadout::new("First")],
    )];
    for _ in 0..2 {
        assert_eq!(
            fire_pending_catalog_start(&mut pending, true, false, arming),
            StartScriptPump::Hold,
        );
        assert!(!pending[0].started);
        assert!(pending[0].waiting_reason.is_some());
        assert_eq!(watch.evidence()["phase"], "ready");
        assert!(watch.failure().is_none());
    }
}

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
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        "export function tick() {}".into(),
        script::LoadShape::NativeTick,
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
    let mut pending = vec![PendingCatalogStart::load(
        "catalogtest",
        "throw new Error('zz-setup-proof');\nexport function tick() {}".into(),
        script::LoadShape::NativeTick,
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
