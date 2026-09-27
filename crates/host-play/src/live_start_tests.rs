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

#[test]
fn catalog_core_start_marker_precedes_actual_isolate_start() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);

    start_catalog_with_core(&watch, "catalogtest", || {
        assert!(
            watch
                .qualify()
                .unwrap_err()
                .contains("no post-Start observations"),
            "the core baseline must be frozen before isolate Start"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn catalog_core_initial_login_then_preparation_then_start_uses_current_session() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", Observation::default(), true);
    watch.observe("catalogtest", prepared_thiever(), false);

    let mut started = false;
    start_catalog_with_core(&watch, "catalogtest", || {
        started = true;
        Ok(())
    })
    .unwrap();
    assert!(started);
    assert!(watch
        .qualify()
        .unwrap_err()
        .contains("no post-Start observations"));
}

#[test]
fn refused_isolate_start_fails_the_armed_core() {
    let watch = CoreWatch::default();
    watch.configure(CoreCase::Thiever, "catalogtest");
    watch.observe("catalogtest", prepared_thiever(), false);

    let error = start_catalog_with_core(&watch, "catalogtest", || Err("no slot".into()));
    assert_eq!(error, Err("no slot".to_string()));
    assert_eq!(watch.failure().as_deref(), Some("no slot"));
}
