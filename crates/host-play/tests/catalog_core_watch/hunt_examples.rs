//! Manifest-bearing enter/leave fixtures run in a real isolate against a toy
//! world that walks one tile a tick toward the last walk it was sent. Requests,
//! host position, paint receipts, and named stops form local watch evidence.
//! Removing the awaited run must fail the test-local watch.

use host_play::catalog_core::{LineOfSightTile, ScriptAct};
use script::interact::InteractReq;
use {
    script::native::snapshot_source::ReachViewInput,
    script::native::snapshot_source::SnapshotInput, script::native::snapshot_source::TileInput,
    script_js::wire::encode_snapshot,
};
use {script_js::LoadIsolate, script_js::LoadShape};

use super::ht;

const LEAVE_EXAMPLE: &str = r#"
const STOP_OK = 'leave lair qualification complete';
const RECEIPT_PREFIX = 'leave-lair-receipt:';
let running = false;

export async function tick(api) {
    if (running || !api.snapshot.ingame || !api.snapshot.here) return;
    const here = api.snapshot.here;
    const box = {
        minX: here.x - 2,
        maxX: here.x + 2,
        minZ: here.z - 2,
        maxZ: here.z + 2,
        level: here.level,
    };
    const walkOut = { x: here.x + 8, z: here.z, level: here.level };
    const site = { key: 'leave-lair', boxes: [box], walkOut };
    const hooks = { leaveByWalk: () => true };

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.leaveRun(site, hooks);
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after) {
        throw new Error('leaveRun did not settle true at a host position');
    }
    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: { x: walkOut.x, z: walkOut.z, level: walkOut.level },
        key: site.key,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint.begin().title('leave lair fixture').row(line).end();
    api.stop(STOP_OK);
}
"#;

const ENTER_EXAMPLE: &str = r#"
const STOP_OK = 'enter lair qualification complete';
const RECEIPT_PREFIX = 'enter-lair-receipt:';
let token = null;
let running = false;

export async function tick(api) {
    if (running || !api.snapshot.ingame || !api.snapshot.here) return;
    const here = api.snapshot.here;
    const approach = { x: here.x + 8, z: here.z, level: here.level };
    const box = {
        minX: approach.x - 1,
        maxX: approach.x + 1,
        minZ: approach.z - 1,
        maxZ: approach.z + 1,
        level: approach.level,
    };
    const site = {
        key: 'enter-lair',
        boxes: [box],
        approach: [approach],
        talkGate: null,
        feeGate: null,
        gate: null,
        keyItem: null,
    };
    const hooks = {
        parked: () => false,
        shieldReady: () => true,
        hpFraction: () => 1,
        panicHp: () => 0.2,
    };

    if (token == null) {
        const began = api.enterBegin(site);
        if (!began.ok) throw new Error(began.error);
        token = began.value.token;
    }
    const validated = api.enterValidate({ token }, hooks);
    if (!validated.ok) throw new Error(validated.error);
    if (!validated.value) return;

    running = true;
    const from = { x: here.x, z: here.z, level: here.level };
    const outcome = await api.enterRun({ token }, hooks);
    const after = api.snapshot.here;
    if (outcome.kind !== 'done' || outcome.value !== true || !after) {
        throw new Error('enterRun did not settle true at a host position');
    }
    const receipt = {
        outcome,
        from,
        here: { x: after.x, z: after.z, level: after.level },
        dest: { x: approach.x, z: approach.z, level: approach.level },
        box,
        key: site.key,
    };
    const line = `${RECEIPT_PREFIX}${JSON.stringify(receipt)}`;
    api.log(line);
    api.paint.begin().title('enter lair fixture').row(line).end();
    api.stop(STOP_OK);
}
"#;

fn snapshot(tick: u64, here: LineOfSightTile) -> SnapshotInput<'static> {
    SnapshotInput {
        tick,
        here: Some(TileInput {
            x: here.x,
            z: here.z,
            level: here.level,
        }),
        ingame: true,
        walk_outcome_cancel_reason: script::native::WalkCancelReason::None,
        user_move_intent_seq: 0,
        inv: &[],
        inv_size: 28,
        stats: &[],
        booths: &[],
        nearest_booth: None,
        banks: &[],
        bank: &[],
        bank_side: &[],
        bank_open: false,
        bank_loaded: false,
        bank_generation: 0,
        count_dialog_open: false,
        withdraw_x_result_seq: 0,
        withdraw_x_result: false,
        bank_op_result_seq: 0,
        bank_op_result: false,
        hold: false,
        ours: false,
        npcs: &[],
        locs: &[],
        players: &[],
        ground: &[],
        equipment: &[],
        chat_open: false,
        chat_continue: false,
        chat_text: None,
        chat_options: &[],
        side_tab: -1,
        varps: &[],
        combat_styles: &[],
        run_energy: 0,
        run_enabled: false,
        retaliate_enabled: false,
        my_name: None,
        in_combat: false,
        animating: false,
        main_modal_id: -1,
        chat_modal_id: -1,
        make_products: &[],
        side_tab_ifaces: &[],
        spell_buttons: &[],
        chat_lines: &[],
        bank_note_on: -1,
        bank_note_off: -1,
        scene_state: 2,
        weight: 0,
        combat_level: 0,
        camera_yaw: 0,
        camera_pitch: 0,
        teleports_enabled: false,
        self_slot: 0,
        trade_offer_open: false,
        trade_confirm_open: false,
        trade_partner: None,
        trade_mine: &[],
        trade_theirs: &[],
        trade_side: &[],
        trade_accept_id: -1,
        trade_decline_id: -1,
        shop_open: false,
        shop_stock: &[],
        reach: ReachViewInput::UNAVAILABLE,
        attacked_by_player: false,
        self_target_kind: 0,
        self_target_index: -1,
        widgets: &[],
    }
}

/// A world walk as host-play records it once queued.
fn recorded_walk(req: &InteractReq) -> Option<ScriptAct> {
    let (x, z, level, radius, exact, teleports, wilderness, bank_fetch, request_id) = match *req {
        InteractReq::Walk {
            x,
            z,
            level,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            allow_prereq_dialogue: _,
            allow_quest_start: _,
            request_id,
            avoid: _,
            cross: _,
        } => (
            x,
            z,
            level,
            0,
            true,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
        ),
        InteractReq::WalkNear {
            x,
            z,
            level,
            radius,
            compat_tile_intent: _,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            allow_prereq_dialogue: _,
            allow_quest_start: _,
            request_id,
            avoid: _,
            cross: _,
        } => (
            x,
            z,
            level,
            radius,
            false,
            allow_teleports,
            allow_wilderness,
            allow_bank_fetch,
            request_id,
        ),
        _ => return None,
    };
    Some(ScriptAct::Walk {
        dest: LineOfSightTile { x, z, level },
        radius,
        exact,
        allow_teleports: teleports,
        allow_wilderness: wilderness,
        allow_bank_fetch: bank_fetch,
        request_id,
    })
}

/// Run a manifest-bearing fixture in a real isolate and retain the host's
/// requests, observed position, paint receipt, and named stop.
#[derive(Clone, Copy)]
enum LairFixture {
    Enter,
    Leave,
}

impl LairFixture {
    fn receipt_prefix(self) -> &'static str {
        match self {
            Self::Enter => "enter-lair-receipt:",
            Self::Leave => "leave-lair-receipt:",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Enter => "enter-lair",
            Self::Leave => "leave-lair",
        }
    }

    fn stop_reason(self) -> &'static str {
        match self {
            Self::Enter => "enter lair qualification complete",
            Self::Leave => "leave lair qualification complete",
        }
    }
}

fn example_through_the_gate(
    kind: LairFixture,
    body: &str,
    from: LineOfSightTile,
) -> Result<(), &'static str> {
    let origin = script_js::native_fixture(body);
    let iso = LoadIsolate::spawn(origin, LoadShape::NativeTick, vec![]).unwrap();
    let mut here = from;
    let mut walk = None;
    let mut requests = Vec::new();
    let mut request_count = 0;
    let mut receipt = None;
    let mut stop = None;
    for tick in 1..=40 {
        iso.post_snapshot(encode_snapshot(&snapshot(tick, here)));
        iso.on_game_tick(tick);
        let _ = iso.probe("1");

        let drained = iso.drain_interacts();
        request_count += drained.len();
        for req in &drained {
            if let Some(act) = recorded_walk(req) {
                if let ScriptAct::Walk { dest, radius, .. } = &act {
                    walk = Some((*dest, *radius));
                }
                requests.push(act);
            }
        }
        if let Some(paint) = iso.paint() {
            for line in &paint.lines {
                if let Some(encoded) = line.strip_prefix(kind.receipt_prefix()) {
                    receipt = serde_json::from_str(encoded).ok();
                }
            }
        }
        if let Some(receipt) = iso.script_stop_receipt() {
            stop = Some((receipt.tick, receipt.reason));
        }
        if iso.stopped() {
            break;
        }
        if let Some((dest, radius)) = walk {
            if (here.x - dest.x).abs().max((here.z - dest.z).abs()) > radius {
                here.x += (dest.x - here.x).signum();
                here.z += (dest.z - here.z).signum();
            }
        }
    }
    iso.join();
    lair_fixture_passes(
        kind,
        from,
        here,
        request_count,
        &requests,
        receipt.as_ref(),
        stop.as_ref(),
    )
}

fn value_i32(value: &serde_json::Value, key: &str) -> Option<i32> {
    i32::try_from(value.get(key)?.as_i64()?).ok()
}

fn receipt_tile(receipt: &serde_json::Value, key: &str) -> Option<LineOfSightTile> {
    let value = receipt.get(key)?;
    Some(LineOfSightTile {
        x: value_i32(value, "x")?,
        z: value_i32(value, "z")?,
        level: value_i32(value, "level")?,
    })
}

fn same_tile(a: LineOfSightTile, b: LineOfSightTile) -> bool {
    (a.x, a.z, a.level) == (b.x, b.z, b.level)
}

fn in_box(area: &serde_json::Value, tile: LineOfSightTile) -> bool {
    value_i32(area, "level") == Some(tile.level)
        && value_i32(area, "minX").is_some_and(|min| tile.x >= min)
        && value_i32(area, "maxX").is_some_and(|max| tile.x <= max)
        && value_i32(area, "minZ").is_some_and(|min| tile.z >= min)
        && value_i32(area, "maxZ").is_some_and(|max| tile.z <= max)
}

fn chebyshev(a: LineOfSightTile, b: LineOfSightTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn lair_fixture_passes(
    kind: LairFixture,
    from: LineOfSightTile,
    here: LineOfSightTile,
    request_count: usize,
    requests: &[ScriptAct],
    receipt: Option<&serde_json::Value>,
    stop: Option<&(u64, String)>,
) -> Result<(), &'static str> {
    if request_count != 1 || requests.len() != 1 {
        return Err("expected exactly one recorded request");
    }
    let ScriptAct::Walk {
        dest,
        radius,
        exact,
        allow_teleports,
        allow_wilderness,
        allow_bank_fetch,
        ..
    } = &requests[0]
    else {
        return Err("expected one world walk");
    };
    if *allow_teleports || *allow_wilderness || *allow_bank_fetch {
        return Err("the walk must stay local");
    }

    let Some(receipt) = receipt else {
        return Err("missing painted receipt");
    };
    let Some((stop_tick, stop_reason)) = stop else {
        return Err("missing script stop receipt");
    };
    if *stop_tick == 0 || stop_reason != kind.stop_reason() {
        return Err("unexpected named stop");
    }
    if receipt.get("key").and_then(serde_json::Value::as_str) != Some(kind.key()) {
        return Err("wrong site key in receipt");
    }
    let Some(outcome) = receipt.get("outcome") else {
        return Err("missing outcome in receipt");
    };
    if outcome.get("kind").and_then(serde_json::Value::as_str) != Some("done")
        || outcome.get("value") != Some(&serde_json::Value::Bool(true))
        || receipt_tile(receipt, "from").is_none_or(|tile| !same_tile(tile, from))
        || receipt_tile(receipt, "here").is_none_or(|tile| !same_tile(tile, here))
        || receipt_tile(receipt, "dest").is_none_or(|tile| !same_tile(tile, *dest))
    {
        return Err("receipt outcome or observed tiles do not match the host run");
    }

    let gap = chebyshev(from, *dest);
    match kind {
        LairFixture::Enter => {
            let Some(area) = receipt.get("box") else {
                return Err("enter receipt has no box");
            };
            if !(*exact
                && *radius == 0
                && (8..=16).contains(&gap)
                && value_i32(area, "minX") == Some(dest.x - 1)
                && value_i32(area, "maxX") == Some(dest.x + 1)
                && value_i32(area, "minZ") == Some(dest.z - 1)
                && value_i32(area, "maxZ") == Some(dest.z + 1)
                && in_box(area, *dest)
                && in_box(area, here)
                && !in_box(area, from))
            {
                return Err("enter walk or destination box does not qualify");
            }
        }
        LairFixture::Leave => {
            let in_start_box = |tile: LineOfSightTile| {
                tile.level == from.level
                    && (tile.x - from.x).abs() <= 2
                    && (tile.z - from.z).abs() <= 2
            };
            if !(!*exact
                && *radius == 3
                && (8..=16).contains(&gap)
                && !in_start_box(*dest)
                && !in_start_box(here))
            {
                return Err("leave walk or destination does not qualify");
            }
        }
    }
    Ok(())
}

/// The fixture with its awaited run replaced by an already-settled value.
fn without_the_run(source: &str, run_call: &str) -> String {
    assert!(source.contains(run_call), "the fixture awaits {run_call}");
    source.replace(run_call, "({ kind: 'done', value: true })")
}

#[test]
fn the_leave_fixture_passes_and_fails_without_its_run() {
    let from = ht(3222, 3218);
    assert_eq!(
        example_through_the_gate(LairFixture::Leave, LEAVE_EXAMPLE, from),
        Ok(()),
        "the manifest-bearing leave fixture qualifies"
    );
    let no_run = without_the_run(LEAVE_EXAMPLE, "await api.leaveRun(site, hooks)");
    assert!(
        example_through_the_gate(LairFixture::Leave, &no_run, from).is_err(),
        "deleting its leaveRun fails the local watch"
    );
}

#[test]
fn the_enter_fixture_passes_and_fails_without_its_run() {
    let from = ht(3222, 3218);
    assert_eq!(
        example_through_the_gate(LairFixture::Enter, ENTER_EXAMPLE, from),
        Ok(()),
        "the manifest-bearing enter fixture qualifies"
    );
    let no_run = without_the_run(ENTER_EXAMPLE, "await api.enterRun({ token }, hooks)");
    assert!(
        example_through_the_gate(LairFixture::Enter, &no_run, from).is_err(),
        "deleting its enterRun fails the local watch"
    );
}
