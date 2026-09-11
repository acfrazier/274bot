use std::sync::Arc;

use host_play::catalog_core::{CoreCase, CoreWatch, CoreWatchStatus, Observation};

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
