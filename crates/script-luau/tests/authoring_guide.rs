//! The Luau authoring guide, run as tests.
//!
//! Every `luau` code block of `docs/api/writing-luau-scripts.md` and
//! `docs/api/luau-api-v2.md` follows a `<!-- ci-example: NAME -->` or a
//! `<!-- ci-refusal: CODE -->` marker. This file extracts those blocks, reads
//! each manifest with the zero-VM reader, checks the setting schema and runs
//! each example on the real Luau runtime with synthetic game input. It needs no
//! game data. A `luau` block with no marker fails the test, so no snippet in
//! the guide goes unchecked.

pub mod support;

use std::collections::BTreeMap;

use api::identity::RequestId;
use script::api_gather::{GatherCounts, GatherEnd};
use script::bank::request::BankSeatOperation;
use script::canvas::CanvasOp;
use script::interact::InteractReq;
use script::manifest::DeclaredHostCapabilities;
use script::native::request::NativeKind;
use script::native::seat::{SeatResult, SeatTerminal};
use script::native::seat_rows::{SeatPageInput, SeatStartArgs};
use script::native::snapshot_source::{NativeFactsInput, QuestStatusInput, TileInput};
use script::native::SettingsBag;
use script::surface::{LentOutcome, SurfaceRun};
use script_luau::read_manifest;
use support::{snapshot, spawn_with, stop, tick, tick_with};

const GUIDE: &str = include_str!("../../../docs/api/writing-luau-scripts.md");
const REFERENCE: &str = include_str!("../../../docs/api/luau-api-v2.md");

/// The examples that the guide must keep. Adding an example means adding its
/// name here and a driver below.
const EXAMPLES: [&str; 5] = [
    "quickstart-luau",
    "walk-status-luau",
    "paint-overlay-luau",
    "bank-interact-luau",
    "gather-quest-luau",
];
const REFUSALS: [&str; 3] = [
    "luau-manifest-nonliteral",
    "luau-capability-unsupported",
    "manifest-required-field",
];

struct Blocks {
    examples: BTreeMap<String, String>,
    refusals: BTreeMap<String, String>,
}

/// The marked `luau` blocks of one page. A `luau` fence with no marker on the
/// line before it, or a marker with no fence after it, panics.
fn blocks(page: &str) -> Blocks {
    let mut found = Blocks {
        examples: BTreeMap::new(),
        refusals: BTreeMap::new(),
    };
    let mut marker: Option<(bool, String)> = None;
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("<!-- ci-") {
            assert!(marker.is_none(), "two markers without a block: {trimmed}");
            let (kind, name) = rest
                .strip_suffix(" -->")
                .and_then(|rest| rest.split_once(": "))
                .unwrap_or_else(|| panic!("malformed marker: {trimmed}"));
            marker = match kind {
                "example" => Some((true, name.to_owned())),
                "refusal" => Some((false, name.to_owned())),
                other => panic!("unknown marker kind {other}: {trimmed}"),
            };
        } else if trimmed == "```luau" {
            let (is_example, name) = marker
                .take()
                .expect("a luau block follows a ci-example or ci-refusal marker");
            let mut source = String::new();
            for body in lines.by_ref() {
                if body.trim() == "```" {
                    break;
                }
                source.push_str(body);
                source.push('\n');
            }
            assert!(!source.is_empty(), "the block {name} is empty");
            let map = if is_example {
                &mut found.examples
            } else {
                &mut found.refusals
            };
            assert!(map.insert(name.clone(), source).is_none(), "{name} twice");
        } else if marker.is_some() && !trimmed.is_empty() {
            panic!("a marker is followed by `{trimmed}`, not a luau block");
        }
    }
    assert!(
        marker.is_none(),
        "a marker at the end of the page has no block"
    );
    found
}

fn guide_blocks() -> Blocks {
    assert!(
        blocks(REFERENCE).examples.is_empty() && blocks(REFERENCE).refusals.is_empty(),
        "the reference page holds no runnable blocks"
    );
    blocks(GUIDE)
}

fn example(name: &str) -> String {
    guide_blocks()
        .examples
        .remove(name)
        .unwrap_or_else(|| panic!("the guide has no example {name}"))
}

fn settings(values: &[(&str, serde_json::Value)]) -> SettingsBag {
    let mut bag = SettingsBag::new();
    for (key, value) in values {
        bag.insert((*key).to_owned(), value.clone());
    }
    bag
}

fn seat_result(id: RequestId, seq: u64) -> SeatResult {
    SeatResult {
        id,
        seq,
        outcome: SeatTerminal::Done,
        prior_parent: None,
        session: None,
        authority_revoked: false,
    }
}

#[test]
fn the_guide_keeps_its_runnable_examples_and_marks_every_luau_block() {
    let found = guide_blocks();
    assert_eq!(
        found
            .examples
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        {
            let mut names = EXAMPLES.to_vec();
            names.sort_unstable();
            names
        },
        "the examples of the guide"
    );
    assert_eq!(
        found
            .refusals
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        {
            let mut codes = REFUSALS.to_vec();
            codes.sort_unstable();
            codes
        },
        "the refusal samples of the guide"
    );
}

#[test]
fn every_example_has_a_manifest_that_the_zero_vm_reader_accepts() {
    for (name, source) in guide_blocks().examples {
        let read = read_manifest(&source)
            .unwrap_or_else(|error| panic!("the manifest of {name} is refused: {error}"));
        let manifest = read.manifest();
        assert_eq!(manifest.api_version(), 2, "{name}");
        assert!(!manifest.name().is_empty(), "{name}");
        assert_eq!(
            manifest.capabilities(),
            DeclaredHostCapabilities::EMPTY,
            "{name}"
        );
        let erased = read.erased_source(&source);
        assert_eq!(
            erased.len(),
            source.len(),
            "{name}: erasing keeps every byte"
        );
        assert_eq!(erased.lines().count(), source.lines().count(), "{name}");
        assert!(
            !erased.contains("manifest {"),
            "{name}: the statement is erased"
        );
    }
}

#[test]
fn every_refusal_sample_is_refused_with_its_code_at_a_position() {
    for (code, source) in guide_blocks().refusals {
        let error = match read_manifest(&source) {
            Ok(read) => panic!("{code}: the reader accepted {:?}", read.manifest()),
            Err(error) => error,
        };
        assert_eq!(error.code(), code);
        assert!(error.line() >= 1 && error.column() >= 1, "{code}: {error}");
        assert!(error.to_string().contains(&code), "{code}: {error}");
    }
}

#[test]
fn quickstart_validates_its_settings_runs_a_tick_and_stops_with_its_reason() {
    let source = example("quickstart-luau");
    let read = read_manifest(&source).expect("manifest");
    let schema = read.manifest().settings_schema();
    assert_eq!(schema.len(), 1);
    let count = &schema[0];
    assert_eq!(count.id, "count");
    assert_eq!(count.ty, "number");
    assert_eq!(count.default.as_deref(), Some("3"));
    assert_eq!(count.min.as_deref(), Some("1"));
    assert_eq!(count.max.as_deref(), Some("10"));
    assert_eq!(count.label.as_deref(), Some("Count"));

    let mut run = spawn_with(&source, Some(settings(&[("count", 7.into())]))).expect("spawn");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(run.drain_logs(), ["Count:\t7"]);
    let receipt = run.script_stop_receipt().expect("the example stops itself");
    assert_eq!(receipt.tick, 1);
    assert_eq!(receipt.reason, "Quick start complete. Count=7");

    let done = stop(run);
    assert_eq!(done.hook_failure, None);
    assert_eq!(done.logs, ["Stopped:\tQuick start complete. Count=7"]);
}

#[test]
fn quickstart_uses_the_fallback_when_no_value_is_saved() {
    let mut run = spawn_with(&example("quickstart-luau"), None).expect("spawn");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(
        run.script_stop_receipt().expect("stops").reason,
        "Quick start complete. Count=3"
    );
}

/// The walk rows that the run queued since the last call: the request id and
/// the tile, level and radius of each.
fn walk_rows(run: &dyn SurfaceRun) -> Vec<(u64, i32, i32, i32, i32)> {
    run.drain_seat_batches()
        .iter()
        .flat_map(|batch| batch.batch.reqs.iter())
        .filter_map(|row| match row.request {
            InteractReq::WalkNear {
                x,
                z,
                level,
                radius,
                request_id,
                ..
            } => Some((request_id, x, z, level, radius)),
            _ => None,
        })
        .collect()
}

fn queued_walk(run: &dyn SurfaceRun) -> (u64, i32, i32, i32, i32) {
    let walks = walk_rows(run);
    assert_eq!(walks.len(), 1, "one walk row: {walks:?}");
    walks[0]
}

/// How many starts and action requests the run queued since the last call.
/// The wait and loop facts that the seat client queues on its own do not count.
fn queued_rows(run: &dyn SurfaceRun) -> usize {
    run.drain_seat_batches()
        .iter()
        .map(|batch| {
            batch.batch.starts.len()
                + batch
                    .batch
                    .reqs
                    .iter()
                    .filter(|row| {
                        !matches!(
                            row.request,
                            InteractReq::WaitEnqueued
                                | InteractReq::WaitSettled
                                | InteractReq::LoopSettled
                        )
                    })
                    .count()
        })
        .sum()
}

#[test]
fn walk_example_walks_near_the_tile_then_stops_on_arrival() {
    let source = example("walk-status-luau");
    let read = read_manifest(&source).expect("manifest");
    let schema = read.manifest().settings_schema();
    assert_eq!(
        schema.iter().map(|def| def.id.as_str()).collect::<Vec<_>>(),
        ["x", "z", "radius"]
    );
    assert!(schema.iter().all(|def| def.ty == "number"));
    assert_eq!(schema[2].max.as_deref(), Some("20"));

    let mut run = spawn_with(&source, None).expect("spawn");
    let at = |tick, x, z| {
        let mut input = snapshot(tick);
        input.here = Some(TileInput { x, z, level: 0 });
        input
    };

    // The player is far away: the tick starts a walk and waits for its outcome.
    assert_eq!(tick(&mut *run, &at(1, 3200, 3200)), LentOutcome::Accepted);
    assert!(run.live_walking_operation(), "the walk is awaited");
    let (id, x, z, level, radius) = queued_walk(&*run);
    assert_eq!((x, z, level, radius), (3222, 3218, 0, 2));
    assert_ne!(id, 0, "the seat client stamps the walk id");

    // No outcome yet: nothing is logged and the tick stays suspended.
    assert_eq!(tick(&mut *run, &at(2, 3205, 3205)), LentOutcome::Accepted);
    assert!(run.drain_logs().is_empty());
    assert!(run.script_stop_receipt().is_none());

    // The outcome arrives: the tick resumes and logs the tile of the request.
    let outcome = NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_request_id: id,
        walk_outcome_x: 3222,
        walk_outcome_z: 3218,
        walk_outcome_level: 0,
        walk_outcome_radius: 2,
        local_player_moving: Some(false),
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &at(3, 3221, 3218), outcome),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["Walk finished at\t3222\t3218"]);
    assert!(run.script_stop_receipt().is_none());

    // The next tick starts again from the top and sees that the player arrived.
    assert_eq!(tick(&mut *run, &at(4, 3221, 3218)), LentOutcome::Accepted);
    assert_eq!(run.script_stop_receipt().expect("stops").reason, "Arrived");
    assert!(walk_rows(&*run).is_empty(), "no second walk");
}

#[test]
fn walk_example_logs_a_failed_walk_and_waits_without_a_tile() {
    let mut run = spawn_with(&example("walk-status-luau"), None).expect("spawn");
    // A tick with no player tile does nothing.
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert_eq!(queued_rows(&*run), 0, "no request without a tile");
    assert!(run.drain_logs().is_empty());

    let mut input = snapshot(2);
    input.here = Some(TileInput {
        x: 3200,
        z: 3200,
        level: 0,
    });
    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    let (id, ..) = queued_walk(&*run);
    let failed = NativeFactsInput {
        walk_outcome_seq: 1,
        walk_outcome_request_id: id,
        walk_outcome_failed: true,
        walk_outcome_no_path: true,
        walk_outcome_x: 3222,
        walk_outcome_z: 3218,
        walk_outcome_level: 0,
        walk_outcome_radius: 2,
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &snapshot(3), failed),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        ["Walk failed. No path:\ttrue\tBlocked:\tfalse"]
    );
}

#[test]
fn paint_example_records_the_overlay_from_the_tick_state_and_its_settings() {
    let source = example("paint-overlay-luau");
    let read = read_manifest(&source).expect("manifest");
    let schema = read.manifest().settings_schema();
    assert_eq!(schema[0].ty, "boolean");
    assert_eq!(schema[1].group.as_deref(), Some("Colours"));

    let mut run = spawn_with(&source, None).expect("spawn");
    let mut input = snapshot(1);
    input.here = Some(TileInput {
        x: 3222,
        z: 3218,
        level: 0,
    });
    input.run_energy = 85;
    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    let frame = run.paint().expect("onPaint recorded a frame");
    assert!(frame.lines.is_empty(), "no paint error: {:?}", frame.lines);
    let texts: Vec<_> = frame
        .canvas
        .iter()
        .filter_map(|op| match op {
            CanvasOp::FillText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Tile 3222, 3218 (level 0)", "Run energy 85%"]);
    assert!(
        matches!(
            frame.canvas[0],
            CanvasOp::FillRect {
                x: 4,
                y: 4,
                w: 220,
                h: 44,
                ..
            }
        ),
        "the panel is the first operation: {:?}",
        frame.canvas[0]
    );

    // A saved `showEnergy = false` leaves out the second line.
    let mut run =
        spawn_with(&source, Some(settings(&[("showEnergy", false.into())]))).expect("spawn");
    assert_eq!(tick(&mut *run, &input), LentOutcome::Accepted);
    let frame = run.paint().expect("frame");
    assert_eq!(
        frame
            .canvas
            .iter()
            .filter(|op| matches!(op, CanvasOp::FillText { .. }))
            .count(),
        1
    );
}

/// Drives a run through awaited seat starts: each tick delivers the results of
/// the starts that the previous tick queued. `on_start` sees the arguments of
/// each start in order. Returns the other requests in order and the tick that
/// the script stopped on.
fn drive_to_stop(
    run: &mut dyn SurfaceRun,
    mut on_start: impl FnMut(&SeatStartArgs),
) -> (Vec<InteractReq>, u64) {
    let mut requests = Vec::new();
    let mut results: Vec<SeatResult> = Vec::new();
    let mut sequence = 0;
    for tick_number in 1..=40 {
        let page = SeatPageInput::new(&[], &results, &[], 0).expect("seat page");
        let facts = NativeFactsInput {
            seat_page: Some(page),
            ..Default::default()
        };
        assert_eq!(
            tick_with(run, &snapshot(tick_number), facts),
            LentOutcome::Accepted
        );
        results.clear();
        for batch in run.drain_seat_batches() {
            for start in &batch.batch.starts {
                sequence += 1;
                results.push(seat_result(start.id, sequence));
                on_start(&start.args);
            }
            requests.extend(batch.batch.reqs.iter().map(|row| row.request.clone()));
        }
        if run.script_stop_receipt().is_some() {
            return (requests, tick_number);
        }
    }
    panic!("the script did not stop within 40 ticks");
}

#[test]
fn bank_example_awaits_each_bank_step_and_queues_the_item_requests_in_order() {
    let source = example("bank-interact-luau");
    let read = read_manifest(&source).expect("manifest");
    assert_eq!(
        read.manifest().settings_schema()[0].default.as_deref(),
        Some("Bones")
    );

    let mut run =
        spawn_with(&source, Some(settings(&[("item", "Oak logs".into())]))).expect("spawn");
    let mut operations = Vec::new();
    let (requests, stopped_on) = drive_to_stop(&mut *run, |args| {
        let SeatStartArgs::Bank(bank) = args else {
            panic!("expected a bank start: {args:?}");
        };
        operations.push(bank.operation());
    });
    assert_eq!(
        operations,
        [
            BankSeatOperation::OpenNearestWorld,
            BankSeatOperation::DepositAll,
            BankSeatOperation::Close,
        ],
        "the bank steps run in the order of the script"
    );
    let named: Vec<_> = requests
        .iter()
        .filter_map(|request| match request {
            InteractReq::Withdraw { name, action } => {
                Some(("withdraw", name.as_str(), action.as_str()))
            }
            InteractReq::Held { name, action, .. } => {
                Some(("held", name.as_str(), action.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        named,
        [
            ("withdraw", "Oak logs", "Withdraw-All"),
            ("held", "Oak logs", "Bury"),
        ]
    );
    assert!(
        stopped_on >= 6,
        "each await and wait takes a tick: {stopped_on}"
    );
    assert_eq!(
        run.script_stop_receipt().expect("stops").reason,
        "Banked and buried Oak logs"
    );
}

#[test]
fn bank_example_stops_with_the_status_when_the_bank_does_not_open() {
    let mut run = spawn_with(&example("bank-interact-luau"), None).expect("spawn");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    let batches = run.drain_seat_batches();
    let id = batches[0].batch.starts[0].id;
    let results = [SeatResult {
        outcome: SeatTerminal::Failed,
        ..seat_result(id, 1)
    }];
    let facts = NativeFactsInput {
        seat_page: Some(SeatPageInput::new(&[], &results, &[], 0).expect("seat page")),
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &snapshot(2), facts),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.script_stop_receipt().expect("stops").reason,
        "Bank did not open: failed"
    );
    assert_eq!(queued_rows(&*run), 0, "no further bank step");
}

#[test]
fn gather_example_reads_the_quest_runs_the_session_and_reports_its_counts() {
    let source = example("gather-quest-luau");
    let read = read_manifest(&source).expect("manifest");
    let schema = read.manifest().settings_schema();
    assert_eq!(
        schema.iter().map(|def| def.ty.as_str()).collect::<Vec<_>>(),
        ["number", "boolean", "string"]
    );
    assert_eq!(schema[0].min.as_deref(), Some("2"));
    assert_eq!(schema[2].default.as_deref(), Some("Cook's Assistant"));

    let mut run = spawn_with(
        &source,
        Some(settings(&[("radius", 20.into()), ("bank", false.into())])),
    )
    .expect("spawn");
    let rows = [QuestStatusInput {
        name: "Cook's Assistant",
        status: "inProgress",
        component_id: Some(7333),
    }];
    let facts = NativeFactsInput {
        quest_statuses: Some(&rows),
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &snapshot(1), facts),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["Quest\tinProgress"]);

    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1, "one seat batch");
    let start = &batches[0].batch.starts[0];
    assert_eq!(start.kind, NativeKind::Gather);
    let SeatStartArgs::Gather(bag) = &start.args else {
        panic!("expected a gather start: {:?}", start.args);
    };
    assert_eq!(bag.get("skill"), Some(&serde_json::json!("Woodcutting")));
    assert_eq!(bag.get("radius"), Some(&serde_json::json!(20)));
    assert_eq!(bag.get("disposition"), Some(&serde_json::json!("Power")));
    assert_eq!(bag.get("buyTools"), Some(&serde_json::json!(false)));

    let end = GatherEnd::Stopped {
        token: start.id.get(),
        counts: GatherCounts {
            yielded: 28,
            dropped: 0,
            deposited: 28,
            trips: 1,
            xp: 700,
        },
    };
    let results = [seat_result(start.id, 1)];
    let facts = NativeFactsInput {
        seat_page: Some(SeatPageInput::new(&[], &results, &[], 0).expect("seat page")),
        api_gather_outcome: Some(&end),
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &snapshot(2), facts),
        LentOutcome::Accepted
    );
    assert_eq!(
        run.drain_logs(),
        ["Gatherer\tstopped\tyielded\t28\txp\t700"]
    );
    assert_eq!(
        run.script_stop_receipt().expect("stops").reason,
        "Session ended"
    );
}

#[test]
fn gather_example_reports_an_unknown_quest_row_and_a_busy_refusal() {
    let mut run = spawn_with(&example("gather-quest-luau"), None).expect("spawn");
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    // The snapshot has no quest tab, so the read reports why.
    assert_eq!(run.drain_logs(), ["Quest unknown:\tquest-tab-unbound"]);
    let batches = run.drain_seat_batches();
    let start = &batches[0].batch.starts[0];
    let SeatStartArgs::Gather(bag) = &start.args else {
        panic!("expected a gather start");
    };
    assert_eq!(
        bag.get("radius"),
        Some(&serde_json::json!(12)),
        "the fallback"
    );
    assert_eq!(bag.get("disposition"), Some(&serde_json::json!("Bank")));

    let results = [SeatResult {
        outcome: SeatTerminal::Refused(api::script_contract::Refusal::Busy),
        ..seat_result(start.id, 1)
    }];
    let facts = NativeFactsInput {
        seat_page: Some(SeatPageInput::new(&[], &results, &[], 0).expect("seat page")),
        ..Default::default()
    };
    assert_eq!(
        tick_with(&mut *run, &snapshot(2), facts),
        LentOutcome::Accepted
    );
    assert_eq!(run.drain_logs(), ["Gatherer did not run:\trefused\tbusy"]);
    assert_eq!(
        run.script_stop_receipt().expect("stops").reason,
        "Session ended"
    );
}
