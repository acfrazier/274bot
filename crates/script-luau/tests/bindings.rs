//! Luau bindings through product runs (plan slices L4a, L4b and L4c).
//! Each call checks its frame behavior and the typed seat row it produces.

pub mod support;

use std::sync::Arc;

use api::identity::RequestId;
use api::quest_progress::{EvidenceStamp, ProgressFlag};
use api::script_contract::{AbortReason, MAX_START_CHARGE};
use api::selected::{ClientRevision, FactKey, Gap, Knowledge, RunKey, Truth};
use api::snapshot::QuestListStatus;
use script::api_combat::{CombatEnd, CombatSummary, InterruptCause, ReportEnd, TargetSpec};
use script::api_gather::{GatherCounts, GatherEnd, GatherFailure};
use script::api_progress::{ProgressPage, QuestProgressRow};
use script::bank::request::BankSeatOperation;
use script::combat::{MeleeMode, Pick, PvpSource, RangedMode, Style};
use script::interact::{InspectAvoidWire, InteractReq};
use script::native::request::NativeKind;
use script::native::seat::{SeatResult, SeatTerminal};
use script::native::seat_rows::{SeatPageInput, SeatStartArgs};
use script::native::snapshot_source::{
    InspectHopInput, ItemRowInput, NativeFactsInput, PuzzleBoardInput, QuestStatusInput,
    RouteInspectFactsInput, RouteInspectTerminalInput, TileInput,
};
use script::native::{SettingsBag, WalkCancelReason};
use script::surface::{LentOutcome, SurfaceRun, SurfaceWake, TickOutcome};
use support::{
    snapshot, snapshot_with_items, spawn, spawn_with, spawn_with_game_data, stop, tick, tick_with,
};

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
fn item_row<'a>(item: &'a api::game_data::GameItem, slot: i32) -> ItemRowInput<'a> {
    ItemRowInput {
        name: item.name.as_deref(),
        count: 1,
        id: item.id,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot,
    }
}

fn queued_interactions(run: &dyn SurfaceRun) -> Vec<InteractReq> {
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one seat batch");
    batches[0]
        .batch
        .reqs
        .iter()
        .filter(|&row| !matches!(&row.request, InteractReq::LoopSettled))
        .map(|row| row.request.clone())
        .collect()
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
            onPaint = function()
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
            onPaint = function()
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

fn queued_bank_operation(run: &dyn SurfaceRun) -> BankSeatOperation {
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one bank seat batch");
    assert_eq!(batches[0].batch.starts.len(), 1, "one typed bank start");
    assert_eq!(
        batches[0]
            .batch
            .reqs
            .iter()
            .map(|row| row.request.clone())
            .collect::<Vec<_>>(),
        [InteractReq::WaitEnqueued],
        "bank starts queue no interaction rows"
    );
    let start = &batches[0].batch.starts[0];
    assert_eq!(start.kind, NativeKind::BankPlan);
    let SeatStartArgs::Bank(args) = &start.args else {
        panic!("bank start has typed arguments: {start:?}");
    };
    args.validate(start.kind).expect("valid bank arguments");
    args.operation()
}

#[test]
fn bank_bindings_start_the_typed_bank_operations() {
    for (member, expected) in [
        ("openNearest", BankSeatOperation::OpenNearestWorld),
        ("depositAll", BankSeatOperation::DepositAll),
        ("close", BankSeatOperation::Close),
    ] {
        let source = format!("return {{ tick=function(api) api.bank.{member}() end }}");
        let mut run = spawn(&source);
        assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
        assert_eq!(
            queued_bank_operation(&*run),
            expected,
            "api.bank.{member} starts the matching native plan"
        );
        assert!(run.drain_logs().is_empty());
    }
}

#[test]
fn interaction_bindings_queue_exact_typed_rows() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.interact.npc("Banker", "Bank", 1)
            api.interact.npc("Banker", "Bank")
            api.interact.loc(3093, 3243, 0, "Use-quickly", 2214)
            api.interact.obj(3093, 3243, 0, "Take", 995)
            api.interact.player("alice", "Trade")
            api.interact.held("Bronze axe", "Wield", 0, 1351)
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one interaction batch");
    assert_eq!(
        batches[0]
            .batch
            .reqs
            .iter()
            .map(|row| row.request.clone())
            .collect::<Vec<_>>(),
        [
            InteractReq::Npc {
                name: "Banker".into(),
                action: "Bank".into(),
                index: Some(1),
            },
            InteractReq::Npc {
                name: "Banker".into(),
                action: "Bank".into(),
                index: None,
            },
            InteractReq::Loc {
                x: 3093,
                z: 3243,
                level: 0,
                action: "Use-quickly".into(),
                id: Some(2214),
            },
            InteractReq::Obj {
                x: 3093,
                z: 3243,
                level: 0,
                name: None,
                action: "Take".into(),
                id: Some(995),
            },
            InteractReq::Player {
                name: "alice".into(),
                action: "Trade".into(),
            },
            InteractReq::Held {
                name: "Bronze axe".into(),
                action: "Wield".into(),
                slot: Some(0),
                target_item_id: Some(1351),
            },
            InteractReq::LoopSettled,
        ]
    );
    assert!(run.drain_logs().is_empty());
}

#[test]
fn ordinary_action_bindings_queue_native_requests() {
    let data = api::game_data::for_revision(ClientRevision::R377).expect("R377 game data");
    let tinderbox = data.item_by_alias("tinderbox").expect("tinderbox alias");
    let lamp_unlit = data
        .item_by_alias("oil_lamp_unlit")
        .expect("unlit lamp alias");
    let lamp_lit = data.item_by_alias("oil_lamp_lit").expect("lit lamp alias");
    let axe = data.item_by_alias("bronze_axe").expect("axe alias");
    let inventory = [
        item_row(tinderbox, 0),
        item_row(lamp_unlit, 1),
        item_row(lamp_lit, 2),
        item_row(axe, 3),
    ];
    let equipment = [item_row(lamp_unlit, 0)];
    let input = snapshot_with_items(1, &inventory, &equipment);
    let mut run = spawn_with_game_data(
        r#"
        return { tick = function(api)
            api.inventory.useOn("tinderbox", "oil_lamp_unlit")
            api.inventory.option("oil_lamp_unlit", 2)
            api.equipment.wear("bronze_axe")
            api.equipment.remove("oil_lamp_unlit")
            api.loc.interact({x = 3169, z = 3172, level = 0}, "Climb-down")
            api.dialog.continue()
            api.dialog.answer(2)
            api.interface.button(601)
        end }
        "#,
        None,
        Some(Arc::clone(&data)),
    )
    .expect("valid Luau run");

    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    assert_eq!(
        queued_interactions(&*run),
        [
            InteractReq::UseOn {
                name: "Tinderbox".into(),
                kind: "inv".into(),
                target_name: Some("Oil lamp".into()),
                x: 0,
                z: 0,
                level: 0,
                index: None,
                source_item_id: Some(tinderbox.id),
                source_item_slot: Some(0),
                target_item_id: Some(lamp_unlit.id),
                target_item_slot: Some(1),
            },
            InteractReq::HeldOp {
                item_id: lamp_unlit.id,
                slot: 1,
                operation: 2,
            },
            InteractReq::Wear {
                name: "Bronze axe".into(),
            },
            InteractReq::Unequip {
                name: "Oil lamp".into(),
            },
            InteractReq::Loc {
                x: 3169,
                z: 3172,
                level: 0,
                action: "Climb-down".into(),
                id: None,
            },
            InteractReq::ContinueDialog { component_id: None },
            InteractReq::Answer { option: 2 },
            InteractReq::IfButton { component_id: 601 },
        ]
    );
    assert!(run.drain_logs().is_empty());
}

#[test]
fn ordinary_action_bindings_refuse_invalid_arguments() {
    let data = api::game_data::for_revision(ClientRevision::R377).expect("R377 game data");
    let tinderbox = data.item_by_alias("tinderbox").expect("tinderbox alias");
    let lamp = data
        .item_by_alias("oil_lamp_unlit")
        .expect("unlit lamp alias");
    let inventory = [item_row(tinderbox, 0), item_row(lamp, 1)];
    let equipment = [item_row(lamp, 0)];
    let input = snapshot_with_items(1, &inventory, &equipment);
    let mut run = spawn_with_game_data(
        r#"
        return { tick = function(api)
            api.log(pcall(api.inventory.useOn, 1, "oil_lamp_unlit"))
            api.log(pcall(api.inventory.useOn, "tinderbox"))
            api.log(pcall(api.inventory.useOn, "missing_alias", "oil_lamp_unlit"))
            api.log(pcall(api.inventory.option, "oil_lamp_unlit", "2"))
            api.log(pcall(api.inventory.option, "oil_lamp_unlit"))
            api.log(pcall(api.inventory.option, "oil_lamp_unlit", 0))
            api.log(pcall(api.inventory.option, "oil_lamp_unlit", 6))
            api.log(pcall(api.equipment.wear, 3))
            api.log(pcall(api.equipment.wear))
            api.log(pcall(api.equipment.remove, 3))
            api.log(pcall(api.equipment.remove))
            api.log(pcall(api.loc.interact, "not a tile", "Open"))
            api.log(pcall(api.loc.interact, {x = 1, z = 2}, "Open"))
            api.log(pcall(api.loc.interact, {x = "1", z = 2, level = 0}, "Open"))
            api.log(pcall(api.loc.interact, {x = 1, z = 2, level = 0, extra = 1}, "Open"))
            api.log(pcall(api.loc.interact, {x = 1, z = 2, level = 0}))
            api.log(pcall(api.loc.interact, {x = 2147483648, z = 2, level = 0}, "Open"))
            api.log(pcall(api.dialog.continue, true))
            api.log(pcall(api.dialog.answer, "2"))
            api.log(pcall(api.dialog.answer, 0))
            api.log(pcall(api.dialog.answer, 2147483648))
            api.log(pcall(api.interface.button, "601"))
            api.log(pcall(api.interface.button, -1))
            api.log(pcall(api.interface.button))
            api.log(pcall(api.interface.button, 2147483648))
        end }
        "#,
        None,
        Some(Arc::clone(&data)),
    )
    .expect("valid Luau run");

    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "a string",
            "a string",
            "known item",
            "32-bit integer",
            "32-bit integer",
            "between 1 and 5",
            "between 1 and 5",
            "a string",
            "a string",
            "a string",
            "a string",
            "tile table",
            "level",
            "32-bit integer",
            "unknown tile key",
            "a string",
            "32-bit integer",
            "at most 0",
            "32-bit integer",
            "at least 1",
            "32-bit integer",
            "integer from 0",
            "integer from 0",
            "integer from 0",
            "integer from 0",
        ],
    );
    assert!(queued_interactions(&*run).is_empty());
}

#[test]
fn item_actions_use_aliases_and_reject_ambiguous_display_names() {
    let data = api::game_data::for_revision(ClientRevision::R377).expect("R377 game data");
    let tinderbox = data.item_by_alias("tinderbox").expect("tinderbox alias");
    let lamp_unlit = data
        .item_by_alias("oil_lamp_unlit")
        .expect("unlit lamp alias");
    let lamp_lit = data.item_by_alias("oil_lamp_lit").expect("lit lamp alias");
    let inventory = [
        item_row(tinderbox, 0),
        item_row(lamp_unlit, 1),
        item_row(lamp_lit, 2),
    ];
    let equipment = [item_row(lamp_unlit, 0), item_row(lamp_lit, 1)];
    let input = snapshot_with_items(1, &inventory, &equipment);
    let mut run = spawn_with_game_data(
        r#"
        return { tick = function(api)
            api.log(pcall(api.inventory.useOn, "tinderbox", "Oil lamp"))
            api.log(pcall(api.inventory.option, "Oil lamp", 2))
            api.log(pcall(api.equipment.wear, "oil_lamp_unlit"))
            api.log(pcall(api.equipment.remove, "oil_lamp_unlit"))
        end }
        "#,
        None,
        Some(Arc::clone(&data)),
    )
    .expect("valid Luau run");

    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    assert_refused(&run.drain_logs(), &["ambiguous"; 4]);
    assert!(queued_interactions(&*run).is_empty());
}

#[test]
fn item_option_without_game_data_accepts_only_a_posted_display_name() {
    let inventory = [ItemRowInput {
        name: Some("Tinderbox"),
        count: 1,
        id: 590,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: -1,
        slot: 0,
    }];
    let input = snapshot_with_items(1, &inventory, &[]);
    let mut run = spawn_with_game_data(
        r#"
        return { tick = function(api)
            api.inventory.option("Tinderbox", 1)
            api.log(pcall(api.inventory.option, "tinderbox", 1))
        end }
        "#,
        None,
        None,
    )
    .expect("valid Luau run");

    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    assert_refused(&run.drain_logs(), &["known display name"]);
    assert_eq!(
        queued_interactions(&*run),
        [InteractReq::HeldOp {
            item_id: 590,
            slot: 0,
            operation: 1,
        }]
    );
}

#[test]
fn interaction_text_is_bounded_before_host_copy() {
    let source = format!(
        r#"
        return {{ tick = function(api)
            local name = string.rep("x", {MAX_START_CHARGE})
            api.interact.player(name, "Trade")
            api.log(pcall(api.interact.player, string.rep("y", {MAX_START_CHARGE}+1), "Trade"))
        end }}
        "#
    );
    let mut run = spawn(&source);
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let error = format!("api.interact.player: name exceeds {MAX_START_CHARGE} bytes");
    assert_refused(&run.drain_logs(), &[error.as_str()]);
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one interaction batch");
    assert_eq!(batches[0].batch.reqs.len(), 2);
    let InteractReq::Player { name, action } = &batches[0].batch.reqs[0].request else {
        panic!("the argument at the limit is accepted");
    };
    assert_eq!(name.len(), MAX_START_CHARGE);
    assert_eq!(action, "Trade");
}

#[test]
fn bank_and_interaction_bindings_refuse_bad_arguments_and_wrong_phases() {
    let mut run = spawn(
        r#"
        api.log(pcall(api.bank.close))
        api.log(pcall(api.interact.player, "alice", "Trade"))
        return {
            tick = function(api)
                api.log(pcall(api.bank.depositAll, 1))
                api.log(pcall(api.interact.npc, 12, "Bank"))
                api.log(pcall(api.interact.npc, "Banker", "Bank", -1))
                api.log(pcall(api.interact.loc, 3093, 3243, 0, "Use-quickly", -1))
                api.log(pcall(api.interact.obj, 3093, 3243, 0, "Take", -1))
                api.log(pcall(api.interact.held, "Bronze axe", "Wield", -1, 1351))
                api.log(pcall(api.interact.held, "Bronze axe", "Wield", 0))
                api.log(pcall(api.interact.player, "alice"))
                api.log(pcall(api.interact.player, "alice", "Trade", "extra"))
            end,
            onPaint = function()
                api.log(pcall(api.bank.close))
                api.log(pcall(api.interact.player, "alice", "Trade"))
            end,
        }
        "#,
    );
    assert_refused(
        &run.drain_logs(),
        &[
            "api.bank.close: is only available during tick",
            "api.interact.player: is only available during tick",
        ],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.bank.depositAll: expects at most 0 arguments",
            "api.interact.npc: name must be a string",
            "api.interact.npc: index must be an integer from 0",
            "api.interact.loc: id must be an integer from 0",
            "api.interact.obj: id must be an integer from 0",
            "api.interact.held: slot must be an integer from 0",
            "api.interact.held: target_item_id must be an integer from 0",
            "api.interact.player: action must be a string",
            "api.interact.player: expects at most 2 arguments",
            "api.bank.close: is only available during tick",
            "api.interact.player: is only available during tick",
        ],
    );
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "the tick has one seat batch");
    assert!(
        batches[0].batch.starts.is_empty(),
        "refused calls queue no starts"
    );
    assert_eq!(
        batches[0]
            .batch
            .reqs
            .iter()
            .map(|row| row.request.clone())
            .collect::<Vec<_>>(),
        [InteractReq::LoopSettled],
        "refused calls queue no interaction rows"
    );
    assert!(!run.stopped(), "a refused call does not stop the run");
}

fn queued_start(run: &dyn SurfaceRun, expected_kind: NativeKind) -> (RequestId, SeatStartArgs) {
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one seat batch");
    assert_eq!(batches[0].batch.starts.len(), 1, "one typed start");
    let reqs = batches[0]
        .batch
        .reqs
        .iter()
        .map(|row| row.request.clone())
        .collect::<Vec<_>>();
    assert!(
        reqs == [InteractReq::WaitEnqueued]
            || reqs == [InteractReq::WaitSettled, InteractReq::WaitEnqueued],
        "awaited starts queue only wait facts, got {reqs:?}"
    );
    let start = &batches[0].batch.starts[0];
    assert_eq!(start.kind, expected_kind);
    (start.id, start.args.clone())
}

fn seat_result(id: RequestId, seq: u64, outcome: SeatTerminal) -> SeatResult {
    SeatResult {
        id,
        seq,
        outcome,
        prior_parent: None,
        session: None,
        authority_revoked: false,
    }
}

#[test]
fn combat_fight_queues_typed_start_and_resumes_with_combat_outcomes() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            local first = api.combat.fight({
                target = { npc = { "cow", "Chicken" } },
                pick = "lowestHealth",
                notTargetingOthers = false,
                area = { min_x = 3240, min_z = 3254, max_x = 3266, max_z = 3299, level = 0 },
                radius = 10,
                lostRadius = 18,
                style = "melee",
                meleeMode = "aggressive",
                prayer = false,
                food = true,
                potions = false,
                retaliate = true,
                budgetTicks = 250,
            })
            api.log("first", first.kind, first.value["end"], first.value.report["end"], first.value.report.npcType, first.value.report.swings)
            local second = api.combat.fight({
                target = { attackers = "npcs" },
                style = "magic",
                spells = { "wind_strike", "water_strike" },
                fallbackSpells = true,
                area = {
                    { min_x = 3200, min_z = 3200, max_x = 3210, max_z = 3210, level = 0 },
                    { min_x = 3220, min_z = 3220, max_x = 3230, max_z = 3230, level = 0 },
                },
            })
            api.log("second", second.kind, second.reason)
            local third = api.combat.fight({
                target = { npcId = { 81, 41 } },
                style = "ranged",
                rangedMode = "longRange",
            })
            api.log("third", third.kind, third.value["end"], third.value.cause)
            local fourth = api.combat.fight({
                target = { player = "bob" },
                pvp = true,
            })
            api.log("fourth", fourth.kind, fourth.reason)
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let (id1, args1) = queued_start(&*run, NativeKind::Combat);
    let SeatStartArgs::Combat(req1) = args1 else {
        panic!("expected Combat args: {args1:?}");
    };
    assert_eq!(
        req1.target,
        TargetSpec::Names(vec![Arc::from("cow"), Arc::from("Chicken")].into_boxed_slice())
    );
    assert_eq!(req1.pick, Pick::LowestHealth);
    assert!(!req1.not_targeting_others);
    assert_eq!(&*req1.area, &[[3240, 3254, 3266, 3299, 0]]);
    assert_eq!(req1.radius, 10);
    assert_eq!(req1.lost_radius, 18);
    assert_eq!(req1.style, Style::Melee);
    assert_eq!(req1.melee_mode, Some(MeleeMode::Aggressive));
    assert!(!req1.prayer);
    assert!(req1.food);
    assert!(!req1.potions);
    assert!(req1.retaliate);
    assert_eq!(req1.budget_ticks, 250);

    let end1 = CombatEnd::Fought {
        token: id1.get(),
        report: CombatSummary {
            end: ReportEnd::Killed,
            reason: None,
            npc_type: 81,
            ticks: 14,
            swings: 4,
            casts: 0,
            damage_taken: 1,
            food: 0,
            prayer_doses: 0,
            boost_doses: 0,
            antifire_doses: 0,
            protect_switches: 0,
            pvp_source: Some(PvpSource::Request),
        },
    };
    let results1 = [seat_result(id1, 1, SeatTerminal::Done)];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(2),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results1, &[], 0).expect("seat page")),
                api_combat_outcome: Some(&end1),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["first\tdone\tfought\tkilled\t81\t4"]);

    let (id2, args2) = queued_start(&*run, NativeKind::Combat);
    let SeatStartArgs::Combat(req2) = args2 else {
        panic!("expected Combat args: {args2:?}");
    };
    assert_eq!(
        req2.target,
        TargetSpec::Attackers {
            npcs: true,
            players: false
        }
    );
    assert_eq!(req2.style, Style::Mage);
    assert_eq!(
        req2.spells,
        Some(vec![Arc::from("wind_strike"), Arc::from("water_strike")].into_boxed_slice())
    );
    assert!(req2.fallback_spells);
    assert_eq!(req2.area.len(), 2);

    // Manual movement revokes the combat seat with AbortReason::UserInput.
    let results2 = [seat_result(
        id2,
        2,
        SeatTerminal::Aborted(AbortReason::UserInput),
    )];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(3),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results2, &[], 0).expect("seat page")),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["second\taborted\tuser-input"]);

    let (id3, args3) = queued_start(&*run, NativeKind::Combat);
    let SeatStartArgs::Combat(req3) = args3 else {
        panic!("expected Combat args: {args3:?}");
    };
    assert_eq!(
        req3.target,
        TargetSpec::Ids(vec![81, 41].into_boxed_slice())
    );
    assert_eq!(req3.style, Style::Ranged);
    assert_eq!(req3.ranged_mode, RangedMode::LongRange);

    let end3 = CombatEnd::Interrupted {
        token: id3.get(),
        cause: InterruptCause::Pause,
    };
    let results3 = [seat_result(id3, 3, SeatTerminal::Done)];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(4),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results3, &[], 0).expect("seat page")),
                api_combat_outcome: Some(&end3),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["third\tdone\tinterrupted\tpause"]);

    let (_id4, args4) = queued_start(&*run, NativeKind::Combat);
    let SeatStartArgs::Combat(req4) = args4 else {
        panic!("expected Combat args: {args4:?}");
    };
    assert_eq!(req4.target, TargetSpec::Player(Arc::from("bob")));
    assert!(req4.pvp);

    // Session reset aborts the awaited fight with the CombatOutcome abort shape.
    run.reset_session_work(None);
    assert_eq!(tick(&mut *run, &snapshot(5)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), ["fourth\taborted\treset"]);
    assert_eq!(
        run.drain_tick_outcomes(),
        [TickOutcome::Success {
            tick: 5,
            generation: 2
        }]
    );
}

#[test]
fn combat_fight_refuses_bad_arguments_numeric_strings_and_wrong_phases() {
    let mut run = spawn(
        r#"
        api.log(pcall(api.combat.fight, { target = { npc = "cow" } }))
        return {
            tick = function(api)
                api.log(pcall(api.combat.fight))
                api.log(pcall(api.combat.fight, "cow"))
                api.log(pcall(api.combat.fight, {}))
                api.log(pcall(api.combat.fight, { target = { npc = "cow" }, radius = "12" }))
                api.log(pcall(api.combat.fight, { target = { npcId = "81" } }))
                api.log(pcall(api.combat.fight, { target = { attackers = "npcs" }, pick = "nearest" }))
                api.log(pcall(api.combat.fight, { target = { npc = "cow" }, style = "melee", spells = { "wind_strike" } }))
                api.log(pcall(api.combat.fight, { target = { npc = "cow" }, budgetTicks = 0 }))
            end,
            onPaint = function()
                api.log(pcall(api.combat.fight, { target = { npc = "cow" } }))
            end,
        }
        "#,
    );
    assert_refused(
        &run.drain_logs(),
        &["api.combat.fight: is only available during tick"],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.combat.fight: request must be a table",
            "api.combat.fight: request must be a table",
            "api.combat.fight: invalid-setting:target:required",
            "api.combat.fight: radius must be a 32-bit integer",
            "api.combat.fight: target.npcId must be a 32-bit integer",
            "api.combat.fight: invalid-setting:pick:requires-npc-target",
            "api.combat.fight: invalid-setting:spells:requires-magic",
            "api.combat.fight: invalid-setting:budgetTicks:out-of-range",
            "api.combat.fight: is only available during tick",
        ],
    );
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].batch.starts.is_empty());
}

#[test]
fn gather_run_queues_typed_settings_and_resumes_with_gather_outcomes() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            local first = api.gather.run({
                skill = "Woodcutting",
                woodcuttingResources = { "normal", "oak" },
                location = "Custom",
                customTile = { x = 3190, z = 3245, level = 0 },
                radius = 12,
                disposition = "Power",
                buyTools = false,
                allowTeleports = false,
            })
            api.log("first", first.kind, first.value["end"], first.value.failure.code, first.value.counts.yielded)
            local second = api.gather.run()
            api.log("second", second.kind, second.value["end"], second.value.counts.deposited)
            local third = api.gather.run({ skill = "Mining", miningResources = { "copper" } })
            api.log("third", third.kind, third.reason)
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let (id1, args1) = queued_start(&*run, NativeKind::Gather);
    let SeatStartArgs::Gather(bag1) = args1 else {
        panic!("expected Gather args: {args1:?}");
    };
    assert_eq!(
        bag1.get("skill").and_then(|v| v.as_str()),
        Some("Woodcutting")
    );
    assert_eq!(bag1.get("radius").and_then(|v| v.as_i64()), Some(12));
    assert_eq!(bag1.get("buyTools").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        bag1.get("customTile"),
        Some(&serde_json::json!({ "x": 3190, "z": 3245, "level": 0 }))
    );

    let end1 = GatherEnd::Blocked {
        token: id1.get(),
        failure: GatherFailure {
            code: Arc::from("no-tool"),
            message: Arc::from("missing axe"),
        },
        counts: GatherCounts {
            yielded: 3,
            dropped: 0,
            deposited: 0,
            trips: 0,
            xp: 75,
        },
    };
    let results1 = [seat_result(id1, 1, SeatTerminal::Failed)];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(2),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results1, &[], 0).expect("seat page")),
                api_gather_outcome: Some(&end1),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["first\tdone\tblocked\tno-tool\t3"]);

    let (id2, args2) = queued_start(&*run, NativeKind::Gather);
    let SeatStartArgs::Gather(bag2) = args2 else {
        panic!("expected Gather args: {args2:?}");
    };
    assert!(bag2.is_empty(), "nil settings queues empty default bag");

    let end2 = GatherEnd::Stopped {
        token: id2.get(),
        counts: GatherCounts {
            yielded: 28,
            dropped: 0,
            deposited: 28,
            trips: 1,
            xp: 700,
        },
    };
    let results2 = [seat_result(id2, 2, SeatTerminal::Done)];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(3),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results2, &[], 0).expect("seat page")),
                api_gather_outcome: Some(&end2),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["second\tdone\tstopped\t28"]);

    let (id3, _args3) = queued_start(&*run, NativeKind::Gather);
    let results3 = [seat_result(
        id3,
        3,
        SeatTerminal::Refused(api::script_contract::Refusal::Busy),
    )];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(4),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results3, &[], 0).expect("seat page")),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["third\trefused\tbusy"]);
}

#[test]
fn gather_run_refuses_bad_arguments_numeric_strings_and_wrong_phases() {
    let mut run = spawn(
        r#"
        api.log(pcall(api.gather.run))
        return {
            tick = function(api)
                api.log(pcall(api.gather.run, "Woodcutting"))
                api.log(pcall(api.gather.run, { radius = "12" }))
                api.log(pcall(api.gather.run, { buyTools = "yes" }))
                api.log(pcall(api.gather.run, { skill = "Runecrafting" }))
                api.log(pcall(api.gather.run, { radius = 1 }))
                api.log(pcall(api.gather.run, { location = "Custom" }))
                api.log(pcall(api.gather.run, {}, "extra"))
            end,
            onPaint = function()
                api.log(pcall(api.gather.run))
            end,
        }
        "#,
    );
    assert_refused(
        &run.drain_logs(),
        &["api.gather.run: is only available during tick"],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.gather.run: settings must be a table or nil",
            "api.gather.run: radius must be a 32-bit integer",
            "api.gather.run: buyTools must be a boolean or nil",
            "api.gather.run: invalid-setting:skill:unsupported-skill",
            "api.gather.run: invalid-setting:radius:invalid-radius",
            "api.gather.run: invalid-setting:customTile:required",
            "api.gather.run: expects at most 1 argument",
            "api.gather.run: is only available during tick",
        ],
    );
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].batch.starts.is_empty());
}

#[test]
fn quest_status_reads_lent_rows_and_matches_shared_error_contract() {
    let mut run = spawn(
        r#"
        api.log(pcall(api.questStatus, "Cook's Assistant"))
        return { tick = function(api)
            local a = api.questStatus("Cook's Assistant")
            api.log("a", a.ok, a.status, a.as_of_sequence, a.error)
            local b = api.questStatus("  cook's assistant  ")
            api.log("b", b.ok, b.status, b.as_of_sequence, b.error)
            local c = api.questStatus("   ")
            api.log("c", c.ok, c.error)
            api.log(pcall(api.questStatus, 42))
            api.log(pcall(api.questStatus, "a", "b"))
        end }
        "#,
    );
    assert_refused(
        &run.drain_logs(),
        &["api.questStatus: game input is lent only to tick and onPaint"],
    );

    // L4c Test 3: `quest_statuses: None` in the lent frame returns `quest-tab-unbound`.
    assert_eq!(tick(&mut *run, &snapshot(10)), LentOutcome::Accepted);
    let logs = run.drain_logs();
    assert_eq!(logs[0], "a\tfalse\tnil\tnil\tquest-tab-unbound");
    assert_eq!(logs[1], "b\tfalse\tnil\tnil\tquest-tab-unbound");
    assert_eq!(logs[2], "c\tfalse\tinvalid-args");
    assert_refused(
        &logs[3..],
        &[
            "api.questStatus: name must be a string",
            "api.questStatus: expects at most 1 argument",
        ],
    );

    // L4c Test 4: an empty `Some(&[])` returns `not-on-tab`.
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(11),
            NativeFactsInput {
                quest_statuses: Some(&[]),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    let logs = run.drain_logs();
    assert_eq!(logs[0], "a\tfalse\tnil\tnil\tnot-on-tab");

    // First legal row wins with ASCII folding and `as_of_sequence == snap_tick`.
    let rows = [
        QuestStatusInput {
            name: "Cook's Assistant",
            status: "bogus",
            component_id: Some(7333),
        },
        QuestStatusInput {
            name: "  cook's assistant  ",
            status: "complete",
            component_id: Some(7333),
        },
        QuestStatusInput {
            name: "Cook's Assistant",
            status: "notStarted",
            component_id: Some(7333),
        },
    ];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(25),
            NativeFactsInput {
                quest_statuses: Some(&rows),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    let logs = run.drain_logs();
    assert_eq!(logs[0], "a\ttrue\tcomplete\t25\tnil");
    assert_eq!(logs[1], "b\ttrue\tcomplete\t25\tnil");
}

#[test]
fn quest_paths_matches_released_paths_index() {
    let mut run = spawn(
        r#"
        api.log(pcall(api.questPaths))
        return { tick = function(api)
            local rows = api.questPaths()
            for i = 1, #rows do
                local row = rows[i]
                api.log(row.id, row.display, row.journal, row.draft, table.concat(row.stages, ","))
            end
            api.log(pcall(api.questPaths, 1))
        end }
        "#,
    );
    assert_refused(
        &run.drain_logs(),
        &["api.questPaths: game input is lent only to tick and onPaint"],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let logs = run.drain_logs();
    let expected = script::quester::card::released_paths();
    assert_eq!(logs.len(), expected.len() + 1);
    for (actual, row) in logs.iter().zip(expected.iter()) {
        let stages = row
            .stages
            .iter()
            .map(|stage| stage.as_ref())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            actual,
            &format!(
                "{}\t{}\t{}\t{}\t{}",
                row.id, row.display, row.journal, row.draft, stages
            )
        );
    }
    assert_refused(
        &logs[expected.len()..],
        &["api.questPaths: expects at most 0 arguments"],
    );
}

#[test]
fn quest_progress_queues_owned_read_and_resumes_with_progress_page() {
    let mut run = spawn(
        r#"
        api.log(pcall(api.questProgress, "cook"))
        return {
            tick = function(api)
                api.log(pcall(api.questProgress))
                api.log(pcall(api.questProgress, "   "))
                api.log(pcall(api.questProgress, 7))
                local done = api.questProgress("cook")
                local row = done.value.row
                api.log(
                    "done",
                    done.kind,
                    done.value["end"],
                    row.quest,
                    row.display,
                    row.colour,
                    row.stage.state,
                    row.stage.value,
                    row.rule.state,
                    row.rule.gap,
                    row.complete,
                    row.journal_read,
                    row.binding,
                    row.role,
                    row.evidence.tick,
                    row.flags[1].flag,
                    row.flags[1].truth,
                    row.flags[1].count
                )
                local refused = api.questProgress("unknown_path")
                api.log("refused", refused.kind, refused.value["end"], refused.value.reason)
                local aborted = api.questProgress("cook")
                api.log("aborted", aborted.kind, aborted.reason)
            end,
            onPaint = function()
                api.log(pcall(api.questProgress, "cook"))
            end,
        }
        "#,
    );
    assert_refused(
        &run.drain_logs(),
        &["api.questProgress: is only available during tick"],
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.questProgress: quest must be a string",
            "api.questProgress: quest must be a non-empty string",
            "api.questProgress: quest must be a string",
            "api.questProgress: is only available during tick",
        ],
    );
    let (id1, args1) = queued_start(&*run, NativeKind::QuestProgress);
    assert_eq!(
        args1,
        SeatStartArgs::QuestProgress {
            name: "cook".to_owned(),
            journal_only: false,
        }
    );

    let page1 = ProgressPage::Done {
        token: id1.get(),
        row: Arc::new(QuestProgressRow {
            quest: Arc::from("cook"),
            display: Arc::from("Cook's Assistant"),
            colour: QuestListStatus::Complete,
            stage: Knowledge::Known(Arc::from("cook:2")),
            rule: Knowledge::Unknown(Gap {
                code: Arc::from("tab-complete"),
                sources: Arc::from([]),
            }),
            complete: Truth::True,
            flags: Arc::from([ProgressFlag {
                flag: FactKey(Arc::from("delivered")),
                truth: Truth::True,
                count: Some(3),
            }]),
            evidence: EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 77,
                sequence: 9,
            },
            journal_read: false,
            binding: Arc::from("cook"),
            role: Some(Arc::from("solo")),
        }),
    };
    let results1 = [seat_result(id1, 1, SeatTerminal::Done)];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(2),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results1, &[], 0).expect("seat page")),
                api_progress: Some(&page1),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    let logs = run.drain_logs();
    assert_eq!(
        logs[0],
        "done\tdone\tdone\tcook\tCook's Assistant\tcomplete\tknown\tcook:2\tunknown\ttab-complete\ttrue\tfalse\tcook\tsolo\t77\tdelivered\ttrue\t3"
    );

    let (id2, args2) = queued_start(&*run, NativeKind::QuestProgress);
    assert_eq!(
        args2,
        SeatStartArgs::QuestProgress {
            name: "unknown_path".to_owned(),
            journal_only: false,
        }
    );
    let page2 = ProgressPage::Refused {
        token: id2.get(),
        reason: Arc::from("unknown-path"),
    };
    let results2 = [seat_result(id2, 2, SeatTerminal::Failed)];
    assert_eq!(
        tick_with(
            &mut *run,
            &snapshot(3),
            NativeFactsInput {
                seat_page: Some(SeatPageInput::new(&[], &results2, &[], 0).expect("seat page")),
                api_progress: Some(&page2),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    let logs = run.drain_logs();
    assert_eq!(logs[0], "refused\tdone\trefused\tunknown-path");

    let (_id3, _args3) = queued_start(&*run, NativeKind::QuestProgress);
    run.reset_session_work(None);
    assert_eq!(tick(&mut *run, &snapshot(4)), LentOutcome::Accepted);
    let logs = run.drain_logs();
    assert_eq!(logs[0], "aborted\taborted\treset");
}

#[test]
fn combat_and_gather_refuse_unknown_fields_and_non_sequence_lists() {
    let mut run = spawn(
        r#"
        next = function() return nil end
        return {
            tick = function(api)
                api.log(pcall(api.combat.fight, { target = { npc = "cow" }, budget_ticks = 5 }))
                api.log(pcall(api.combat.fight, { target = { npc = "cow", bogus = true } }))
                api.log(pcall(api.combat.fight, { target = { npc = "cow" }, area = { minX = 3240, min_z = 3254, max_x = 3266, max_z = 3299, level = 0 } }))
                api.log(pcall(api.combat.fight, { target = { npc = "cow" }, area = { min_x = 3240, min_z = 3254, max_x = 3266, max_z = 3299, level = 0, extra = 1 } }))
                api.log(pcall(api.combat.fight, { target = { npc = { "cow", extra = 1 } } }))
                api.log(pcall(api.gather.run, { buy_tools = false }))
                api.log(pcall(api.gather.run, { location = "Custom", customTile = { x = 3190, y = 3245, level = 0 } }))
                api.log(pcall(api.gather.run, { woodcuttingResources = { "normal", bogus = true } }))
            end,
        }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.combat.fight: invalid-settings: unknown request key `budget_ticks`",
            "api.combat.fight: invalid-settings: unknown target key `bogus`",
            "api.combat.fight: invalid-settings: unknown area key `minX`",
            "api.combat.fight: invalid-settings: unknown area key `extra`",
            "api.combat.fight: target.npc must be a contiguous list table",
            "api.gather.run: invalid-settings: unknown settings key `buy_tools`",
            "api.gather.run: invalid-settings: unknown customTile key `y`",
            "api.gather.run: woodcuttingResources must be a contiguous list table",
        ],
    );
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].batch.starts.is_empty());
}

#[test]
fn native_op_bindings_queue_exact_typed_rows_and_reads() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.interact.heldByName("Puzzle box", "Open")
            api.bank.openBooth(3091, 3243, 0, 2213)
            api.bank.openBooth(3091, 3243, 0, 2213, { name = "Bank booth", action = "Use-quickly" })
            api.bank.openStand(3091, 3243, 0, "booth", { name = "Bank booth", stand_op = 2 })
            api.bank.closeNow()
            api.bank.setNoteMode(true)
            api.bank.deposit("Bronze axe")
            api.bank.withdraw("Bronze axe", "Withdraw 1")
            api.bank.withdrawX({
                name = "Bronze axe",
                count = 3,
                bank_item_id = 1351,
                lands_as_id = 1351,
                action = "Withdraw X",
                bank_generation = api.bank.generation(),
            })
            api.walk.nearestBank()
            api.route.inspect(
                { x = 3222, z = 3222 },
                { x = 3093, z = 3243, level = 0 },
                {
                    allow_teleports = false,
                    allow_wilderness = true,
                    allow_bank_fetch = false,
                    avoid = { { minX = 2780, maxX = 3040, minZ = 3130, maxZ = 3330 } },
                    max_expansions = 5000,
                    request_id = 0,
                }
            )
            local board = api.puzzle.board()
            api.puzzle.move(board.items[1].id, board.items[1].slot, board.component_id, board.generation)
            local route = api.route.result()
            api.log(route.seq, tostring(route.ok), route.ticks, route.hops[1].locName, board.generation)
        end }
        "#,
    );
    let mut input = snapshot(1);
    input.bank_generation = 7;
    let hops = [InspectHopInput {
        kind: "npc",
        loc_id: 380,
        loc_name: "Captain Barnaby",
        action: "Pay-fare",
        option: 1,
        from_x: 2683,
        from_z: 3272,
        from_level: 0,
        to_x: 2772,
        to_z: 3234,
        to_level: 0,
        ticks: 12,
    }];
    let pieces = [ItemRowInput {
        name: None,
        count: 1,
        id: 2749,
        ops: &[],
        noted: false,
        cert: -1,
        component_id: 6979,
        slot: 23,
    }];
    assert_eq!(
        tick_with(
            &mut *run,
            &input,
            NativeFactsInput {
                route_inspect: RouteInspectFactsInput {
                    latest: RouteInspectTerminalInput {
                        seq: 4,
                        generation: 2,
                        request_id: 0,
                        ok: true,
                        reason: Some(""),
                        bank_planned: false,
                        ticks: 42.5,
                        terminal: None,
                        hops: &hops,
                    },
                    ..Default::default()
                },
                puzzle_board: Some(PuzzleBoardInput {
                    component_id: 6979,
                    size: 25,
                    items: &pieces,
                    generation: 9,
                }),
                ..Default::default()
            }
        ),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["4\ttrue\t42.5\tCaptain Barnaby\t9"]);
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(
        batches[0]
            .batch
            .reqs
            .iter()
            .map(|row| row.request.clone())
            .collect::<Vec<_>>(),
        [
            InteractReq::InspectAck {
                seq: 4,
                generation: 2,
            },
            InteractReq::Held {
                name: "Puzzle box".into(),
                action: "Open".into(),
                slot: None,
                target_item_id: None,
            },
            InteractReq::OpenBooth {
                x: 3091,
                z: 3243,
                level: 0,
                id: 2213,
                name: None,
                action: None,
            },
            InteractReq::OpenBooth {
                x: 3091,
                z: 3243,
                level: 0,
                id: 2213,
                name: Some("Bank booth".into()),
                action: Some("Use-quickly".into()),
            },
            InteractReq::OpenStand {
                x: 3091,
                z: 3243,
                level: 0,
                kind: "booth".into(),
                name: Some("Bank booth".into()),
                stand_op: Some(2),
                choose: None,
            },
            InteractReq::Close,
            InteractReq::SetNoteMode { on: true },
            InteractReq::Deposit {
                name: "Bronze axe".into(),
                id: None,
            },
            InteractReq::Withdraw {
                name: "Bronze axe".into(),
                action: "Withdraw 1".into(),
            },
            InteractReq::WithdrawX {
                name: "Bronze axe".into(),
                count: 3,
                bank_item_id: 1351,
                lands_as_id: 1351,
                action: "Withdraw X".into(),
                bank_generation: 7,
            },
            InteractReq::WalkNearestBank,
            InteractReq::InspectRoute {
                x: 3093,
                z: 3243,
                level: 0,
                from_x: 3222,
                from_z: 3222,
                from_level: 0,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: false,
                avoid: vec![InspectAvoidWire::Rect {
                    min_x: 2780,
                    max_x: 3040,
                    min_z: 3130,
                    max_z: 3330,
                    level: None,
                }],
                cross: Vec::new(),
                request_id: 0,
                max_expansions: Some(5000),
            },
            InteractReq::PuzzleMove {
                id: 2749,
                slot: 23,
                component: 6979,
                generation: 9,
            },
            InteractReq::LoopSettled,
        ]
    );
}

#[test]
fn native_op_bindings_refuse_unknown_keys_and_bad_types() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.log(pcall(api.bank.setNoteMode, "yes"))
            api.log(pcall(api.bank.openBooth, 3091, 3243, 0, 2213, { bogus = true }))
            api.log(pcall(api.bank.openStand, 3091, 3243, 0, "booth", { bogus = 1 }))
            api.log(pcall(api.bank.withdrawX, { name = "Logs", bogus = 1 }))
            api.log(pcall(api.route.inspect, { x = 1, y = 2 }, { x = 3, z = 4 }))
            api.log(pcall(api.route.inspect, { x = 1, z = 2 }, { x = 3, z = 4 }, { extra = true }))
            api.log(pcall(api.route.inspect, { x = 1, z = 2 }, { x = 3, z = 4 }, { request_id = 7 }))
            api.log(pcall(api.puzzle.move, 2749, -1, 6979, 1))
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_refused(
        &run.drain_logs(),
        &[
            "api.bank.setNoteMode: on must be a boolean",
            "api.bank.openBooth: invalid-settings: unknown options key `bogus`",
            "api.bank.openStand: invalid-settings: unknown options key `bogus`",
            "api.bank.withdrawX: invalid-settings: unknown request key `bogus`",
            "api.route.inspect: invalid-settings: unknown from key `y`",
            "api.route.inspect: invalid-settings: unknown options key `extra`",
            "api.route.inspect: request_id must be 0 or nil",
            "api.puzzle.move: slot must be an integer from 0",
        ],
    );
}
