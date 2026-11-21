//! Each L4a binding through a real product run: the value it returns and
//! the typed error of each wrong argument (plan slice L4a).

pub mod support;

use script::interact::InteractReq;
use script::native::snapshot_source::{NativeFactsInput, TileInput};
use script::native::{SettingsBag, WalkCancelReason};
use script::surface::{LentOutcome, SurfaceRun, SurfaceWake, TickOutcome};
use support::{snapshot, spawn, spawn_with, stop, tick, tick_with};

fn queued_walk(run: &dyn SurfaceRun) -> InteractReq {
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one seat batch");
    let walks = batches[0]
        .batch
        .reqs
        .iter()
        .filter(|row| {
            matches!(
                row.request,
                InteractReq::Walk { .. } | InteractReq::WalkNear { .. }
            )
        })
        .map(|row| row.request.clone())
        .collect::<Vec<_>>();
    assert_eq!(walks.len(), 1, "one walk row: {batches:?}");
    walks[0].clone()
}

fn walk_id(request: &InteractReq) -> u64 {
    match request {
        InteractReq::Walk { request_id, .. } | InteractReq::WalkNear { request_id, .. } => {
            *request_id
        }
        other => panic!("not a walk row: {other:?}"),
    }
}

fn no_walk_rows(run: &dyn SurfaceRun) -> bool {
    run.drain_seat_batches().iter().all(|batch| {
        batch.batch.reqs.iter().all(|row| {
            !matches!(
                row.request,
                InteractReq::Walk { .. } | InteractReq::WalkNear { .. }
            )
        })
    })
}

/// One `pcall` per case; the log line is `false<TAB>message`.
fn assert_refused(logs: &[String], expected: &[&str]) {
    assert_eq!(logs.len(), expected.len(), "logs: {logs:?}");
    for (line, text) in logs.iter().zip(expected) {
        assert!(
            line.starts_with("false\t") && line.contains(text),
            "expected a refusal with {text:?}, got {line:?}"
        );
    }
}

#[test]
fn status_reads_return_the_lent_frame() {
    let mut run = spawn(
        r#"
        return {
            tick = function(api)
                local x, z, level = api.status.tile()
                api.log(api.status.tick(), tostring(api.status.inGame()), x, z, level,
                    api.status.runEnergy(), tostring(api.status.runEnabled()),
                    tostring(api.status.inCombat()), tostring(api.status.animating()),
                    api.status.name())
            end,
            onPaint = function(api)
                api.log("paint", api.status.tick())
            end,
        }
        "#,
    );
    let mut input = snapshot(7);
    input.here = Some(TileInput {
        x: 3200,
        z: 3201,
        level: 0,
    });
    input.run_energy = 55;
    input.run_enabled = true;
    input.animating = true;
    input.in_combat = true;
    input.my_name = Some("alice");
    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    assert_eq!(
        run.drain_logs(),
        [
            "7\ttrue\t3200\t3201\t0\t55\ttrue\ttrue\ttrue\talice",
            "paint\t7",
        ]
    );

    // An unknown tile is no value and a missing name is nil. The next frame
    // is read, not a copy of the first.
    let mut later = snapshot(8);
    later.ingame = false;
    assert_eq!(tick(&mut *run, &later), LentOutcome::Accepted);
    assert_eq!(
        run.drain_logs(),
        [
            "8\tfalse\tnil\tnil\tnil\t0\tfalse\tfalse\tfalse\tnil",
            "paint\t8",
        ]
    );
}

#[test]
fn status_tile_returns_no_value_without_a_tile() {
    let mut run = spawn(
        r##"
        return { tick = function(api) api.log(select("#", api.status.tile())) end }
        "##,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), ["0"]);
}

#[test]
fn status_reads_refuse_outside_a_lending_entry_and_extra_arguments() {
    let run = spawn(
        r#"
        api.log(pcall(api.status.tick))
        return {
            tick = function(api)
                api.log(pcall(api.status.tick, 1))
                api.log(pcall(api.status.tile, 1, 2))
            end,
            onStop = function()
                api.log(pcall(api.status.inGame))
            end,
        }
        "#,
    );
    let mut run = run;
    assert_refused(
        &run.drain_logs(),
        &["api.status.tick: game input is lent only to tick and onPaint"],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.status.tick: expects at most 0 arguments",
            "api.status.tile: expects at most 0 arguments",
        ],
    );
    let done = stop(run);
    assert_eq!(done.hook_failure, None);
    assert_refused(
        &done.logs,
        &["api.status.inGame: game input is lent only to tick and onPaint"],
    );
}

fn settings() -> SettingsBag {
    serde_json::from_value(serde_json::json!({
        "name": "alice",
        "count": 3,
        "ratio": 0.5,
        "flag": true,
    }))
    .expect("settings bag")
}

#[test]
fn settings_read_the_bag_with_js_v2_fallbacks() {
    let source = r#"
        local function read()
            api.log(api.settings.str("name"), api.settings.str("missing"),
                api.settings.str("missing", "dflt"), api.settings.str("count", "text"))
            api.log(api.settings.num("count"), api.settings.num("ratio"),
                api.settings.num("missing"), api.settings.num("missing", 9),
                api.settings.num("name", 4))
            api.log(tostring(api.settings.bool("flag")), tostring(api.settings.bool("missing")),
                tostring(api.settings.bool("missing", true)),
                tostring(api.settings.bool("name", true)))
        end
        read()
        return { tick = function() read() end, onStop = function() read() end }
    "#;
    let expected = [
        "alice\t\tdflt\ttext",
        "3\t0.5\t0\t9\t4",
        "true\tfalse\ttrue\ttrue",
    ];
    let mut run = spawn_with(source, Some(settings())).expect("valid Luau run");
    assert_eq!(run.drain_logs(), expected, "load entry");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), expected, "tick entry");

    // A posted bag replaces the bag of the Start.
    let mut posted = settings();
    posted.insert("name".to_owned(), serde_json::json!("bob"));
    run.post_settings_bag(&posted);
    assert_eq!(tick(&mut *run, &snapshot(2)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs()[0], "bob\t\tdflt\ttext");

    let done = stop(run);
    assert_eq!(done.hook_failure, None);
    assert_eq!(done.logs[0], "bob\t\tdflt\ttext", "onStop reads settings");
}

#[test]
fn settings_refuse_wrong_kinds_and_numeric_strings() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.log(pcall(api.settings.str, 3))
            api.log(pcall(api.settings.str, "name", 1))
            api.log(pcall(api.settings.num, "count", "3"))
            api.log(pcall(api.settings.bool, "flag", 1))
            api.log(pcall(api.settings.num, {}))
            api.log(pcall(api.settings.str, "a", "b", "c"))
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.settings.str: key must be a string",
            "api.settings.str: fallback must be a string or nil",
            "api.settings.num: fallback must be a number or nil",
            "api.settings.bool: fallback must be a boolean or nil",
            "api.settings.num: key must be a string",
            "api.settings.str: expects at most 2 arguments",
        ],
    );
}

const WALK_RESULT: &str = r#"
    local function show(result)
        api.log("walk", tostring(result.ok), tostring(result.failed), result.x, result.z,
            result.level, result.radius, tostring(result.blocked), tostring(result.no_path),
            tostring(result.allow_teleports), result.cancel_reason, result.request_id, result.seq)
    end
"#;

#[test]
fn walk_to_queues_one_walk_row_and_resumes_with_its_outcome() {
    let mut run = spawn(&format!(
        r#"{WALK_RESULT}
        local done = false
        return {{ tick = function(api)
            if not done then
                done = true
                show(api.walk.to(3200, 3201, 0, {{ allow_teleports = true }}))
            end
        end }}
        "#
    ));
    let mut input = snapshot(1);
    input.user_move_intent_seq = 4;
    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    assert!(run.live_walking_operation(), "the walk is awaited");
    let request = queued_walk(&*run);
    let id = walk_id(&request);
    assert!(id != 0, "the seat client stamps the walk id");
    assert!(matches!(
        request,
        InteractReq::Walk {
            x: 3200,
            z: 3201,
            level: 0,
            allow_teleports: true,
            allow_wilderness: false,
            allow_bank_fetch: false,
            allow_prereq_dialogue: None,
            allow_quest_start: None,
            ..
        }
    ));

    // No outcome yet: the invocation stays suspended and logs nothing.
    let mut waiting = snapshot(2);
    waiting.user_move_intent_seq = 4;
    assert_eq!(tick(&mut *run, &waiting), LentOutcome::Accepted);
    assert!(run.drain_logs().is_empty());

    let mut arrived = snapshot(3);
    arrived.user_move_intent_seq = 4;
    let outcome = NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_request_id: id,
        walk_outcome_x: 3200,
        walk_outcome_z: 3201,
        walk_outcome_level: 0,
        walk_outcome_allow_teleports: true,
        local_player_moving: Some(false),
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &arrived, outcome),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        [format!(
            "walk\ttrue\tfalse\t3200\t3201\t0\t0\tfalse\tfalse\ttrue\tnone\t{id}\t1"
        )]
    );
    assert!(!run.live_walking_operation());
    assert_eq!(
        run.drain_tick_outcomes(),
        [TickOutcome::Success {
            tick: 3,
            generation: 1
        }]
    );
}

#[test]
fn walk_near_reports_a_failed_outcome_and_a_manual_cancel() {
    let mut run = spawn(&format!(
        r#"{WALK_RESULT}
        return {{ tick = function(api)
            show(api.walk.near(3210, 3220, 1, 3))
        end }}
        "#
    ));
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let request = queued_walk(&*run);
    let id = walk_id(&request);
    assert!(matches!(
        request,
        InteractReq::WalkNear {
            x: 3210,
            z: 3220,
            level: 1,
            radius: 3,
            compat_tile_intent: false,
            allow_teleports: false,
            ..
        }
    ));
    let failed = NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_request_id: id,
        walk_outcome_failed: true,
        walk_outcome_no_path: true,
        walk_outcome_x: 3210,
        walk_outcome_z: 3220,
        walk_outcome_level: 1,
        walk_outcome_radius: 3,
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &snapshot(2), failed),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        [format!(
            "walk\tfalse\ttrue\t3210\t3220\t1\t3\tfalse\ttrue\tfalse\tnone\t{id}\t1"
        )]
    );

    // The tick ran to its end, so the next tick starts a new walk. A manual
    // movement after the request cancels it with the typed reason.
    assert_eq!(tick(&mut *run, &snapshot(3)), LentOutcome::Accepted);
    let id = walk_id(&queued_walk(&*run));
    let mut moved = snapshot(4);
    moved.user_move_intent_seq = 1;
    moved.walk_outcome_cancel_reason = WalkCancelReason::UserInput;
    assert_eq!(tick(&mut *run, &moved), LentOutcome::Accepted);
    assert_eq!(
        run.drain_logs(),
        [format!(
            "walk\tfalse\ttrue\t3210\t3220\t1\t3\tfalse\tfalse\tfalse\tuser-input\t{id}\t0"
        )]
    );
}

#[test]
fn walk_refuses_bad_arguments_before_it_queues_a_row() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.log(pcall(api.walk.to, "3200", 3201, 0))
            api.log(pcall(api.walk.to, 3200.5, 3201, 0))
            api.log(pcall(api.walk.to, 3200, 2^40, 0))
            api.log(pcall(api.walk.to, 3200, 3201))
            api.log(pcall(api.walk.to, 3200, 3201, 0, true))
            api.log(pcall(api.walk.to, 3200, 3201, 0, { allow_teleports = 1 }))
            api.log(pcall(api.walk.to, 3200, 3201, 0, {}, 5))
            api.log(pcall(api.walk.near, 3200, 3201, 0, -1))
            api.log(pcall(api.walk.near, 3200, 3201, 0, "2"))
            api.log(pcall(api.walk.near, 3200, 3201, 0, 2, { allow_bank_fetch = "yes" }))
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.walk.to: x must be a 32-bit integer",
            "api.walk.to: x must be a 32-bit integer",
            "api.walk.to: z must be a 32-bit integer",
            "api.walk.to: level must be a 32-bit integer",
            "api.walk.to: options must be a table or nil",
            "api.walk.to: allow_teleports must be a boolean or nil",
            "api.walk.to: expects at most 4 arguments",
            "api.walk.near: radius must be an integer from 0",
            "api.walk.near: radius must be an integer from 0",
            "api.walk.near: allow_bank_fetch must be a boolean or nil",
        ],
    );
    assert!(no_walk_rows(&*run), "no refused walk is queued");
    assert!(!run.live_walking_operation());
}

#[test]
fn walk_and_wait_refuse_outside_tick() {
    let run = spawn(
        r#"
        api.log(pcall(api.walk.to, 1, 2, 0))
        return {
            tick = function() end,
            onPaint = function(api)
                api.log(pcall(api.walk.to, 1, 2, 0))
                api.log(pcall(api.wait))
            end,
            onStop = function()
                api.log(pcall(api.walk.near, 1, 2, 0, 1))
            end,
        }
        "#,
    );
    let mut run = run;
    assert_refused(
        &run.drain_logs(),
        &["api.walk.to: is only available during tick"],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.walk.to: is only available during tick",
            "api.wait: is only available during tick",
        ],
    );
    assert!(no_walk_rows(&*run));
    let done = stop(run);
    assert_refused(
        &done.logs,
        &["api.walk.near: is only available during tick"],
    );
}

#[test]
fn log_and_stop_refuse_wrong_arguments() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.log(pcall(api.stop, 5))
            api.log(pcall(api.stop, "a", "b"))
            api.log(pcall(api.log, string.rep("x", 5000)))
            api.log(pcall(api.wait, 1))
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.stop: reason must be a string or nil",
            "api.stop: expects at most 1 argument",
            "api.log: log line exceeds 4096 bytes",
            "api.wait: expects at most 0 arguments",
        ],
    );
    assert!(!run.stopped(), "a refused stop does not stop the run");
}

#[test]
fn the_vm_exposes_exactly_the_binding_table() {
    let mut run = spawn(
        r#"
        local names = {}
        local function walk(prefix, value)
            for key, leaf in pairs(value) do
                if type(leaf) == "table" then
                    walk(prefix .. key .. ".", leaf)
                else
                    table.insert(names, prefix .. key .. ":" .. type(leaf))
                end
            end
        end
        walk("", api)
        table.sort(names)
        return { tick = function(api) api.log(table.concat(names, ",")) end }
        "#,
    );
    let mut expected = script_luau::api_members()
        .map(|member| format!("{}:function", member.path))
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(
        run.dispatch_lent(
            SurfaceWake::Tick { held: false },
            1,
            1,
            &snapshot(1),
            NativeFactsInput::default()
        ),
        LentOutcome::Accepted
    );
    let logged = run.drain_logs();
    let mut names = logged[0].split(',').map(str::to_owned).collect::<Vec<_>>();
    // The `__test` tree exists only in builds with the test hooks.
    names.retain(|name| !name.starts_with("__test."));
    assert_eq!(names, expected, "rule T8: no hidden binding");
}
