//! v2 hunt File cells. Each run below is what host-play records for one
//! Start: the slot's dispatched requests (the act ledger), the host tile and
//! pack each frame, the example's paint receipt and its named stop. A
//! "no-op" run is the example with its `await *Run(...)` line deleted: it
//! paints the same receipt shape, but the host saw no request and no move.

use host_play::catalog_core::{
    parse_hunt_receipt_line, CoreCase, CoreWatch, HuntCell, LineOfSightTile, Observation,
    ScriptAct, ScriptActLedger, ACQUIRE_KEY_DEST, BANK_V2_DEST, CELL_V2_DOOR, DUSTY_KEY_ID,
    HUNT_ACT_ROWS, JAILER, JAIL_DOOR_LOC, JAIL_KEY_ID, VELRAK,
};
use serde_json::{json, Value};

#[path = "hunt_examples.rs"]
mod examples;

fn ht(x: i32, z: i32) -> LineOfSightTile {
    LineOfSightTile { x, z, level: 0 }
}

/// A world walk as host-play queued it. `open`: the wilderness and bank
/// fetch are allowed (the Hold / WalkToSpot families).
fn walk_act(dest: LineOfSightTile, exact: bool, radius: i32, open: bool) -> ScriptAct {
    ScriptAct::Walk {
        dest,
        radius,
        exact,
        allow_teleports: false,
        allow_wilderness: open,
        allow_bank_fetch: open,
        request_id: 7,
    }
}

fn done(value: Value) -> Value {
    json!({ "kind": "done", "value": value })
}

/// The receipt an example paints after its run.
fn hunt_receipt(
    outcome: Value,
    from: LineOfSightTile,
    here: LineOfSightTile,
    dest: LineOfSightTile,
) -> Value {
    json!({ "outcome": outcome, "from": from, "here": here, "dest": dest })
}

struct HuntRun {
    cell: HuntCell,
    watch: CoreWatch,
    obs: Observation,
    ledger: ScriptActLedger,
}

impl HuntRun {
    /// Configure `case`, publish a Start baseline on `from` holding `items`
    /// with `earlier` requests already in the slot's ledger, and Start.
    fn start_after(
        case: &str,
        from: LineOfSightTile,
        items: &[(i32, i32)],
        earlier: &[ScriptAct],
    ) -> Result<Self, String> {
        let case = CoreCase::parse(case).expect("a hunt case");
        let cell = case.hunt_cell().expect("a hunt cell");
        let watch = CoreWatch::default();
        watch.configure(case, "catalogtest");
        let mut ledger = ScriptActLedger::default();
        for act in earlier {
            ledger.record(act.clone());
        }
        let mut obs = Observation {
            ingame: true,
            scene_state: 2,
            player: Some("catalogtest".into()),
            tile: Some((from.x, from.z, from.level)),
            item_ids: items.iter().copied().collect(),
            ..Observation::default()
        };
        obs.hunt.available = true;
        obs.hunt.acts = ledger.published();
        watch.observe("catalogtest", obs.clone(), false);
        watch.begin_start("catalogtest")?;
        Ok(Self {
            cell,
            watch,
            obs,
            ledger,
        })
    }

    fn start(case: &str, from: LineOfSightTile) -> Self {
        Self::start_after(case, from, &[], &[]).expect("Start baseline")
    }

    fn frame(&mut self) -> &mut Self {
        self.obs.tick += 1;
        self.obs.hunt.acts = self.ledger.published();
        self.watch.observe("catalogtest", self.obs.clone(), false);
        self.obs.script_lifecycle = None;
        self
    }

    /// Host-play dispatched `act`, and a frame read the ledger.
    fn act(&mut self, act: ScriptAct) -> &mut Self {
        self.ledger.record(act);
        self.frame()
    }

    fn at(&mut self, tile: LineOfSightTile) -> &mut Self {
        self.obs.tile = Some((tile.x, tile.z, tile.level));
        self.frame()
    }

    fn holding(&mut self, id: i32, count: i32) -> &mut Self {
        self.obs.item_ids.insert(id, count);
        self
    }

    /// The host bank interface this frame.
    fn bank(&mut self, open: bool) -> &mut Self {
        self.obs.bank_open = open;
        self.obs.bank_loaded = open;
        self.frame()
    }

    /// The example's paint row, parsed as the host parses it.
    fn paint(&mut self, receipt: Value) -> &mut Self {
        let line = format!("{}{receipt}", self.cell.receipt_prefix());
        self.obs.hunt.receipt = parse_hunt_receipt_line(self.cell, &line);
        assert!(self.obs.hunt.receipt.is_some(), "unparsed receipt {line}");
        self.frame()
    }

    /// The example's named stop; whether the cell then qualifies.
    fn stop(&mut self) -> bool {
        self.obs.script_lifecycle = Some(script::ScriptLifecycleReceipt {
            runtime_generation: 1,
            state: script::ScriptTerminalState::Stopped,
            tick: u64::from(self.obs.tick) + 1,
            reason: self.cell.stop_reason().into(),
        });
        self.frame();
        self.watch.qualify().is_ok()
    }
}

fn hold_from() -> LineOfSightTile {
    ht(3201, 3201)
}

fn hold_dest() -> LineOfSightTile {
    ht(3203, 3201)
}

#[test]
fn hold_spot_passes_only_on_the_hosts_walk_and_arrival() {
    let (from, dest) = (hold_from(), hold_dest());
    let real = HuntRun::start("hold_spot_v2_ts", from)
        .act(walk_act(dest, true, 0, true))
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(real, "walked to dest, settled done, named stop");

    // The example without its Run line: no request, no move.
    let no_op = HuntRun::start("hold_spot_v2_ts", from)
        .paint(hunt_receipt(done(Value::Null), from, from, dest))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");

    let claimed = HuntRun::start("hold_spot_v2_ts", from)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(
        !claimed,
        "a painted arrival the host tile does not show cannot pass"
    );

    let unwalked = HuntRun::start("hold_spot_v2_ts", from)
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(!unwalked, "arrival without this run's walk cannot pass");

    let attacked = HuntRun::start("hold_spot_v2_ts", from)
        .act(walk_act(dest, true, 0, true))
        .act(ScriptAct::Npc {
            name: "Goblin".into(),
            action: "Attack".into(),
            index: 3,
        })
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(!attacked, "a hold run that attacks cannot pass");

    let boolean = HuntRun::start("hold_spot_v2_ts", from)
        .act(walk_act(dest, true, 0, true))
        .at(dest)
        .paint(hunt_receipt(done(json!(false)), from, dest, dest))
        .stop();
    assert!(!boolean, "hold settles done(null); any other outcome fails");
}

#[test]
fn hunt_cells_ignore_requests_from_before_start() {
    let (from, dest) = (hold_from(), hold_dest());
    let before = HuntRun::start_after(
        "hold_spot_v2_ts",
        from,
        &[],
        &[walk_act(dest, true, 0, true)],
    )
    .expect("Start baseline")
    .at(dest)
    .paint(hunt_receipt(done(Value::Null), from, dest, dest))
    .stop();
    assert!(!before, "a walk sent before Start is not this run's");
}

#[test]
fn hunt_cells_fail_closed_when_ledger_rows_are_lost() {
    let (from, dest) = (hold_from(), hold_dest());
    let mut run = HuntRun::start("hold_spot_v2_ts", from);
    // More requests than the ledger keeps, between two frames.
    for _ in 0..=HUNT_ACT_ROWS {
        run.ledger.record(walk_act(dest, true, 0, true));
    }
    let lost = run
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(!lost, "requests the watch never read cannot be vouched for");
}

#[test]
fn retreat_spot_passes_only_on_the_hosts_walk_to_and_arrival() {
    let (from, dest) = (ht(3201, 3201), ht(3198, 3203));
    let real = HuntRun::start("retreat_spot_v2_ts", from)
        .act(ScriptAct::WalkTo { dest })
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(real, "walked-to dest, settled done, named stop");

    let no_op = HuntRun::start("retreat_spot_v2_ts", from)
        .paint(hunt_receipt(done(Value::Null), from, from, dest))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");

    let world_walk = HuntRun::start("retreat_spot_v2_ts", from)
        .act(walk_act(dest, true, 0, false))
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(
        !world_walk,
        "a retreat hop is a scene walk-to, not a world walk"
    );
}

#[test]
fn walk_spot_passes_only_on_the_hosts_long_walk_and_arrival() {
    let (from, dest) = (ht(3201, 3201), ht(3215, 3201));
    let real = HuntRun::start("walk_spot_v2_ts", from)
        .act(walk_act(dest, true, 0, true))
        .at(ht(3208, 3201))
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(real, "walked 14 tiles to dest, settled done, named stop");

    let no_op = HuntRun::start("walk_spot_v2_ts", from)
        .paint(hunt_receipt(done(Value::Null), from, from, dest))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");

    let near = HuntRun::start("walk_spot_v2_ts", from)
        .act(walk_act(dest, false, 1, true))
        .at(dest)
        .paint(hunt_receipt(done(Value::Null), from, dest, dest))
        .stop();
    assert!(!near, "walk-to-spot walks radius 0, not walk-near");

    let short = ht(3212, 3201);
    let hold_back = HuntRun::start("walk_spot_v2_ts", from)
        .act(walk_act(short, true, 0, true))
        .at(short)
        .paint(hunt_receipt(done(Value::Null), from, short, short))
        .stop();
    assert!(
        !hold_back,
        "Chebyshev 11 is Hold's walk-back, not a walk to a spot"
    );
}

fn enter_receipt(outcome: Value, from: LineOfSightTile, here: LineOfSightTile) -> Value {
    let mut receipt = hunt_receipt(outcome, from, here, ht(3210, 3201));
    receipt["box"] = json!({ "minX": 3209, "maxX": 3211, "minZ": 3200, "maxZ": 3202, "level": 0 });
    receipt["key"] = json!("enter-lair");
    receipt
}

#[test]
fn enter_lair_passes_only_on_the_hosts_approach_walk_into_the_box() {
    let (from, approach, inside) = (ht(3201, 3201), ht(3210, 3201), ht(3209, 3201));
    let real = HuntRun::start("enter_lair_v2_ts", from)
        .act(walk_act(approach, true, 0, false))
        .at(inside)
        .paint(enter_receipt(done(json!(true)), from, inside))
        .stop();
    assert!(real, "walked into the lair box, settled true, named stop");

    let no_op = HuntRun::start("enter_lair_v2_ts", from)
        .paint(enter_receipt(done(json!(true)), from, from))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");

    let gave_up = HuntRun::start("enter_lair_v2_ts", from)
        .act(walk_act(approach, true, 0, false))
        .at(inside)
        .paint(enter_receipt(done(json!(false)), from, inside))
        .stop();
    assert!(!gave_up, "a run that settled false is not an entry");

    let open = HuntRun::start("enter_lair_v2_ts", from)
        .act(walk_act(approach, true, 0, true))
        .at(inside)
        .paint(enter_receipt(done(json!(true)), from, inside))
        .stop();
    assert!(
        !open,
        "the gateless approach walks with every route permission off"
    );
}

fn leave_receipt(outcome: Value, from: LineOfSightTile, here: LineOfSightTile) -> Value {
    let mut receipt = hunt_receipt(outcome, from, here, ht(3210, 3201));
    receipt["key"] = json!("leave-lair");
    receipt
}

#[test]
fn leave_lair_passes_only_on_the_hosts_walk_out_of_the_start_box() {
    let (from, walk_out, out) = (ht(3201, 3201), ht(3210, 3201), ht(3204, 3201));
    let real = HuntRun::start("leave_lair_v2_ts", from)
        .act(walk_act(walk_out, false, 3, false))
        .at(out)
        .paint(leave_receipt(done(json!(true)), from, out))
        .stop();
    assert!(
        real,
        "walked out of the Start box, settled true, named stop"
    );

    let no_op = HuntRun::start("leave_lair_v2_ts", from)
        .paint(leave_receipt(done(json!(true)), from, from))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");

    let inside = ht(3203, 3201);
    let stayed = HuntRun::start("leave_lair_v2_ts", from)
        .act(walk_act(walk_out, false, 3, false))
        .at(inside)
        .paint(leave_receipt(done(json!(true)), from, inside))
        .stop();
    assert!(!stayed, "a tile inside here ± 2 is still in the lair");

    let mut kbd_receipt = leave_receipt(done(json!(true)), from, out);
    kbd_receipt["key"] = json!("kbd-lair");
    let kbd = HuntRun::start("leave_lair_v2_ts", from)
        .act(walk_act(walk_out, false, 3, false))
        .at(out)
        .paint(kbd_receipt)
        .stop();
    assert!(!kbd, "a KBD site is never this cell");
}

/// Frozen `HERO_TILE.TAVERLEY_DUNGEON`, the key and cell cards' landing.
fn landing() -> LineOfSightTile {
    ht(2884, 9798)
}

fn jail_receipt(outcome: Value, here: LineOfSightTile) -> Value {
    let mut receipt = hunt_receipt(outcome, landing(), here, ACQUIRE_KEY_DEST);
    receipt["key"] = json!("taverley-blue");
    receipt
}

fn jailer_attack() -> ScriptAct {
    ScriptAct::Npc {
        name: JAILER.into(),
        action: "Attack".into(),
        index: 12,
    }
}

#[test]
fn acquire_key_passes_only_on_the_corridor_walk_the_attack_and_the_key() {
    let (corridor, fight) = (ht(2931, 9691), ht(2932, 9693));
    let real = HuntRun::start("acquire_key_v2_ts", landing())
        .act(walk_act(ACQUIRE_KEY_DEST, false, 1, false))
        .at(corridor)
        .act(jailer_attack())
        .at(fight)
        .holding(JAIL_KEY_ID, 1)
        .paint(jail_receipt(done(json!(true)), fight))
        .stop();
    assert!(
        real,
        "corridor walk, arrival, Jailer attack, key held, named stop"
    );

    let no_op = HuntRun::start("acquire_key_v2_ts", landing())
        .paint(jail_receipt(done(json!(true)), landing()))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");

    let no_key = HuntRun::start("acquire_key_v2_ts", landing())
        .act(walk_act(ACQUIRE_KEY_DEST, false, 1, false))
        .at(corridor)
        .act(jailer_attack())
        .paint(jail_receipt(done(json!(true)), corridor))
        .stop();
    assert!(
        !no_key,
        "a settled true without the jail key in the pack cannot pass"
    );

    let unarrived = HuntRun::start("acquire_key_v2_ts", landing())
        .act(walk_act(ACQUIRE_KEY_DEST, false, 1, false))
        .act(jailer_attack())
        .holding(JAIL_KEY_ID, 1)
        .paint(jail_receipt(done(json!(true)), landing()))
        .stop();
    assert!(!unarrived, "the host tile never reached the corridor");

    assert!(
        HuntRun::start_after("acquire_key_v2_ts", landing(), &[(JAIL_KEY_ID, 1)], &[]).is_err(),
        "a jail key already held at Start is not this run's"
    );
}

#[test]
fn cell_passes_only_on_the_frozen_fetch_from_velrak_sequence() {
    let (at_door, door_loc, inside) = (ht(2931, 9691), ht(2931, 9689), ht(2931, 9686));
    let unlock = ScriptAct::UseOnLoc {
        item: "Jail key".into(),
        tile: door_loc,
    };
    let talk = ScriptAct::Npc {
        name: VELRAK.into(),
        action: "Talk-to".into(),
        index: 4,
    };
    let open = ScriptAct::Loc {
        tile: door_loc,
        id: JAIL_DOOR_LOC,
        action: "Open".into(),
    };
    let fetch = |steps: &[ScriptAct]| {
        let mut run = HuntRun::start_after("cell_v2_ts", landing(), &[(JAIL_KEY_ID, 1)], &[])
            .expect("Start baseline");
        for act in steps {
            run.act(act.clone());
            let tile = match act {
                ScriptAct::Walk { .. } => at_door,
                ScriptAct::UseOnLoc { .. } => inside,
                ScriptAct::Loc { .. } => CELL_V2_DOOR,
                _ => continue,
            };
            run.at(tile);
        }
        run.holding(DUSTY_KEY_ID, 1)
            .paint(jail_receipt(done(json!(true)), CELL_V2_DOOR))
            .stop()
    };
    let door_near = walk_act(CELL_V2_DOOR, false, 1, false);
    assert!(
        fetch(&[
            door_near.clone(),
            unlock.clone(),
            talk.clone(),
            open.clone()
        ]),
        "walk near the jail door, unlock, talk, open: dusty key held outside"
    );
    assert!(
        !fetch(&[unlock.clone(), talk.clone(), open.clone()]),
        "frozen fetchFromVelrak walks near JAIL_DOOR before anything else"
    );
    assert!(
        !fetch(&[
            door_near.clone(),
            talk.clone(),
            unlock.clone(),
            open.clone()
        ]),
        "Velrak is behind the door: the unlock comes before the talk"
    );
    assert!(
        !fetch(&[door_near, unlock, open]),
        "a dusty key nobody talked to Velrak for cannot pass"
    );

    let no_op = HuntRun::start("cell_v2_ts", landing())
        .paint(jail_receipt(done(json!(true)), landing()))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");
}

fn bank_receipt(outcome: Value, from: LineOfSightTile, here: LineOfSightTile) -> Value {
    let mut receipt = hunt_receipt(outcome, from, here, BANK_V2_DEST);
    receipt["key"] = json!("bank");
    receipt
}

/// The `bank-open` child's booth approach as host-play records it: walk-near
/// radius 0, wilderness and bank fetch on, no walk-wait id.
fn booth_approach(stand: LineOfSightTile) -> ScriptAct {
    ScriptAct::Walk {
        dest: stand,
        radius: 0,
        exact: false,
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        request_id: 0,
    }
}

#[test]
fn bank_passes_only_on_the_hosts_approach_walk_and_a_restock() {
    let (from, near, stand) = (ht(2949, 3381), ht(2946, 3371), ht(2946, 3368));
    let trip = |acts: &[ScriptAct], opened: bool, outcome: Value| {
        let mut run = HuntRun::start("bank_v2_ts", from);
        for act in acts {
            run.act(act.clone());
        }
        run.at(near);
        if opened {
            run.bank(true).bank(false);
        }
        run.paint(bank_receipt(outcome, from, near)).stop()
    };
    let approach = walk_act(BANK_V2_DEST, false, 3, false);
    assert!(
        trip(&[approach.clone()], true, done(json!(true))),
        "walked near the bank, the host saw it open and shut, settled true"
    );
    assert!(
        trip(
            &[approach.clone(), booth_approach(stand)],
            true,
            done(json!(true))
        ),
        "the bank-open child's own booth approach is part of a real trip"
    );
    assert!(
        !trip(
            &[
                approach.clone(),
                booth_approach(stand),
                walk_act(ht(2960, 3390), false, 0, false)
            ],
            true,
            done(json!(true))
        ),
        "an extra unrelated walk cannot pass"
    );
    assert!(
        !trip(
            &[booth_approach(stand), approach.clone()],
            true,
            done(json!(true))
        ),
        "a booth approach before the bank walk is not the family's"
    );
    assert!(
        !trip(
            &[approach.clone(), booth_approach(ht(2960, 3390))],
            true,
            done(json!(true))
        ),
        "a booth approach far from the site bank cannot pass"
    );
    assert!(
        !trip(&[approach.clone()], false, done(json!(true))),
        "a restock the host never saw the bank open for cannot pass"
    );
    assert!(
        !trip(&[approach.clone()], true, done(json!(false))),
        "a trip that settled false did not restock"
    );
    assert!(
        !trip(
            &[walk_act(BANK_V2_DEST, false, 3, true)],
            true,
            done(json!(true))
        ),
        "the approach walks with bank fetch and the wilderness off"
    );

    let no_op = HuntRun::start("bank_v2_ts", from)
        .paint(bank_receipt(done(json!(true)), from, from))
        .stop();
    assert!(!no_op, "a no-op run cannot pass");
}
