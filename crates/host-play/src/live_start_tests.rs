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
    let deadline = Instant::now() + Duration::from_secs(10);
    while handle.run_state("catalogtest") != script::RunState::Running {
        handle.poll_start("catalogtest");
        assert!(Instant::now() < deadline, "scenario Start did not settle");
        std::thread::yield_now();
    }
    handle.stop("catalogtest").unwrap();
}

#[test]
fn refused_scenario_start_fails_the_armed_core() {
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
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::CompiledFailed("start failed: no slot: catalogtest".into()),
    );
    assert_eq!(watch.failure().as_deref(), Some("no slot: catalogtest"));
}
