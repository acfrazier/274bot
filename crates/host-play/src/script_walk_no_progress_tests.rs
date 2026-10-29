use super::*;
use std::sync::Mutex;

struct Capture(Mutex<Vec<(api::hostlog::Source, api::hostlog::Level, String)>>);

impl api::hostlog::Sink for Capture {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record.message.starts_with("walk no-progress ") {
            self.0.lock().unwrap().push((
                record.source,
                record.level,
                record.message.to_owned(),
            ));
        }
    }
}

static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn pending_bot(request_id: u64, destination: WorldTile) -> NavBot {
    let mut bot = NavBot {
        walk_request_id: request_id,
        route_request_id: request_id,
        requested_route: Some((
            destination,
            0,
            false,
            false,
            false,
            nav_planner::zones::ZoneExempt::NONE,
        )),
        ..NavBot::default()
    };
    bot.set_route_work_preparing_for_test();
    bot
}

fn observe(bot: &mut NavBot, tick: u32, at: WorldTile) {
    record_walk_no_progress("walk-wait-test", bot, Some((at.x, at.z, at.level)), tick, None);
}

fn lines() -> Vec<String> {
    CAPTURE
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(_, _, message)| message.clone())
        .collect()
}

#[test]
fn no_progress_counts_batched_observed_ticks_and_clock_wrap() {
    let mut wait = super::super::script_nav::WalkNoProgress::default();
    let at = tile(10, 20);
    assert_eq!(wait.observe(1, at, 100), None);
    assert_eq!(wait.observe(1, at, 109), None);
    assert_eq!(wait.observe(1, at, 109), None);
    assert_eq!(wait.observe(1, at, 110), Some(10));
    assert_eq!(wait.observe(1, at, 500), None);
    wait.reset();
    assert_eq!(wait.observe(1, at, u32::MAX - 4), None);
    assert_eq!(wait.observe(1, at, 5), Some(10));
}

#[test]
fn no_progress_info_logs_once_for_each_observed_episode() {
    const CHILD: &str = "WALK_NO_PROGRESS_LOG_TEST_CHILD";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "script_runtime::script_walk::no_progress_tests::no_progress_info_logs_once_for_each_observed_episode",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env_remove("BOT_DEBUG")
            .output()
            .expect("run isolated walk wait log test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed; 0 failed;"),
            "walk wait log child failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    assert!(api::hostlog::install_sink(&CAPTURE));
    let start = tile(10, 20);
    let first_target = tile(20, 20);
    let mut bot = pending_bot(7, first_target);

    observe(&mut bot, 100, start);
    for _ in 0..4 {
        observe(&mut bot, 100, start);
    }
    for tick in 101..110 {
        observe(&mut bot, tick, start);
    }
    assert!(lines().is_empty(), "the threshold has not been crossed");
    observe(&mut bot, 110, start);
    observe(&mut bot, 110, start);
    observe(&mut bot, 111, start);
    observe(&mut bot, 112, start);
    let first = lines();
    assert_eq!(first.len(), 1, "one line covers the first wait episode");
    assert!(first[0].contains("request=7"));
    assert!(first[0].contains("no_progress_ticks=10"));
    assert!(first[0].contains("reason=route"));
    assert!(first[0].contains("target=Some(WorldTile { x: 20, z: 20, level: 0 })"));

    let moved = tile(11, 20);
    observe(&mut bot, 112, moved);
    for tick in 113..122 {
        observe(&mut bot, tick, moved);
    }
    assert_eq!(lines().len(), 1, "movement starts a fresh count");
    observe(&mut bot, 122, moved);
    assert_eq!(lines().len(), 2, "movement permits one new episode line");
    assert!(lines()[1].contains("tile=WorldTile { x: 11, z: 20, level: 0 }"));

    let second_target = tile(30, 20);
    bot.walk_request_id = 8;
    bot.route_request_id = 8;
    bot.requested_route = Some((
        second_target,
        0,
        false,
        false,
        false,
        nav_planner::zones::ZoneExempt::NONE,
    ));
    bot.set_route_work_preparing_for_test();
    observe(&mut bot, 200, moved);
    for tick in 201..210 {
        observe(&mut bot, tick, moved);
    }
    assert_eq!(lines().len(), 2, "a new request starts a fresh count");
    observe(&mut bot, 210, moved);
    assert_eq!(lines().len(), 3);
    assert!(lines()[2].contains("request=8"));
    assert!(lines()[2].contains("target=Some(WorldTile { x: 30, z: 20, level: 0 })"));

    bot.pending_route = None;
    bot.route = Some(Arc::new(nav_planner::router::Route {
        legs: Vec::new(),
        dest: second_target,
        ticks: 0.0,
    }));
    observe(&mut bot, 300, moved);
    for tick in 301..310 {
        observe(&mut bot, tick, moved);
    }
    observe(&mut bot, 310, moved);
    assert_eq!(lines().len(), 3, "route publication does not repeat an episode");
    apply_nav_follow_outcome(
        &mut bot,
        Some(nav_executor::TravelOutcome::Arrived { at: moved }),
        false,
    );
    bot.route = None;
    bot.set_route_work_preparing_for_test();
    observe(&mut bot, 400, moved);
    for tick in 401..410 {
        observe(&mut bot, tick, moved);
    }
    assert_eq!(lines().len(), 3, "the ended route starts a clean count");
    observe(&mut bot, 410, moved);
    assert_eq!(lines().len(), 4, "route end resets the reported episode");

    abort_walk_on_bot(&mut bot);
    bot.walk_request_id = 8;
    bot.route_request_id = 8;
    bot.requested_route = Some((
        second_target,
        0,
        false,
        false,
        false,
        nav_planner::zones::ZoneExempt::NONE,
    ));
    bot.set_route_work_preparing_for_test();
    observe(&mut bot, 500, moved);
    for tick in 501..510 {
        observe(&mut bot, tick, moved);
    }
    assert_eq!(lines().len(), 4, "abort resets the episode state");
    observe(&mut bot, 510, moved);
    assert_eq!(lines().len(), 5);

    let route_wait = walk_wait_details(&bot, None, None);
    assert_eq!(route_wait.reason, "route");
    assert_eq!(route_wait.target, Some(second_target));

    bot.route = Some(Arc::new(nav_planner::router::Route {
        legs: Vec::new(),
        dest: second_target,
        ticks: 0.0,
    }));
    bot.pending_route = None;
    bot.route_request_id = bot.walk_request_id;
    let door_at = tile(12, 20);
    let door_wait = walk_wait_details(
        &bot,
        None,
        Some(("door", Some(door_at), Some(1530))),
    );
    assert_eq!(door_wait.reason, "door");
    assert_eq!(door_wait.target, Some(door_at));
    assert_eq!(door_wait.target_id, Some(1530));
    log_walk_no_progress("walk-wait-test", 8, moved, 10, door_wait);

    let captured = CAPTURE.0.lock().unwrap();
    assert_eq!(captured.len(), 6);
    assert!(captured.iter().all(|(source, level, _)| {
        *source == api::hostlog::Source::Nav && *level == api::hostlog::Level::Info
    }));
    assert!(captured[0].2.contains("reason=route"));
    assert!(captured[5].2.contains("reason=door"));
    assert!(captured[5].2.contains("target_id=Some(1530)"));
    assert!(captured[5].2.contains("target=Some(WorldTile { x: 12, z: 20, level: 0 })"));
}
