//! Frozen `Traversal.walkTo` / `walkResilient` boat-fare recovery over the
//! whole plantation job: employment, picking, a full crate, the 30-coin
//! payment and the re-walk to the boat. Driven through the registered
//! machine families and synthetic posts only.

use crate::machine::{self, Outcome, Started, Take};
use crate::observed::{self, CarryRow, ChatLine, EntityRow, ItemRow, SceneRow};
use crate::shim::InteractReq;
use crate::walk::tests::{post_walk_outcome, reset, NoJs};
use api::snapshot::WorldTile;
use serde_json::json;

const MUSA: WorldTile = WorldTile {
    x: 2954,
    z: 3147,
    level: 0,
};
const PORT_SARIM: WorldTile = WorldTile {
    x: 3029,
    z: 3217,
    level: 0,
};
const LUTHAS: WorldTile = WorldTile {
    x: 2939,
    z: 3154,
    level: 0,
};
const CRATE: WorldTile = WorldTile {
    x: 2943,
    z: 3151,
    level: 0,
};
const GROVE: WorldTile = WorldTile {
    x: 2926,
    z: 3160,
    level: 0,
};

/// The synthetic scene: the tile, pack, chat and the plantation rows.
struct World {
    tick: u64,
    here: WorldTile,
    coins: i32,
    bananas: i32,
    filler: i32,
    modal: i32,
    cont: bool,
    options: Vec<String>,
    lines: Vec<ChatLine>,
    seq: i32,
    ours: bool,
}

impl World {
    fn new(here: WorldTile, coins: i32) -> Self {
        Self {
            tick: 1,
            here,
            coins,
            bananas: 0,
            filler: 0,
            modal: -1,
            cont: false,
            options: Vec::new(),
            lines: Vec::new(),
            seq: 0,
            ours: false,
        }
    }

    fn post(&mut self) {
        self.tick += 1;
        let mut inv = vec![ItemRow {
            id: 995,
            count: self.coins,
            name: Some("Coins".into()),
            slot: Some(0),
            ..ItemRow::default()
        }];
        inv.extend((0..self.bananas).map(|slot| ItemRow {
            id: 1963,
            count: 1,
            name: Some("Banana".into()),
            slot: Some(1 + slot),
            ..ItemRow::default()
        }));
        inv.extend((0..self.filler).map(|slot| ItemRow {
            id: 1511,
            count: 1,
            name: Some("Logs".into()),
            slot: Some(1 + self.bananas + slot),
            ..ItemRow::default()
        }));
        let here = self.here;
        observed::post(self.tick, |post| {
            post.session(true)
                .ours(self.ours)
                .here(observed::Tile {
                    x: here.x,
                    z: here.z,
                    level: here.level,
                })
                .inv_size(28)
                .inv(inv)
                .chat_modal_id(self.modal)
                .chat_continue(self.cont)
                .chat_options(self.options.clone())
                .chat_lines(self.lines.clone())
                .npcs(vec![EntityRow {
                    index: 50,
                    name: Some("Luthas".into()),
                    x: LUTHAS.x + 1,
                    z: LUTHAS.z,
                    level: 0,
                    distance: 1,
                    actions: vec!["Talk-to".into()].into(),
                    ..EntityRow::default()
                }])
                .locs(vec![
                    loc(2072, "Crate", CRATE.x, CRATE.z, 0),
                    loc(2073, "Banana Tree", GROVE.x - 1, GROVE.z, 1),
                ]);
        });
    }

    fn say(&mut self, text: &str) {
        self.seq += 1;
        self.lines.push(ChatLine {
            seq: self.seq,
            text: text.into(),
        });
    }
}

fn loc(id: i32, name: &str, x: i32, z: i32, distance: i32) -> SceneRow {
    SceneRow {
        id,
        name: Some(name.into()),
        x,
        z,
        level: 0,
        distance,
        actions: vec!["Search".into()].into(),
        shape: 10,
        angle: 0,
    }
}

fn tick() -> Vec<InteractReq> {
    machine::step(&mut NoJs);
    machine::merge_ops(Vec::new())
}

/// Tick until an op matches, with no other op on the way; the match.
fn until(world: &mut World, what: &str, want: impl Fn(&InteractReq) -> bool) -> InteractReq {
    for _ in 0..40 {
        world.post();
        let ops = tick();
        if let Some(hit) = ops.iter().find(|op| want(op)) {
            assert_eq!(ops.len(), 1, "{what}: one op, got {ops:?}");
            return hit.clone();
        }
        assert!(ops.is_empty(), "{what}: unexpected ops {ops:?}");
    }
    panic!("{what}: never emitted");
}

fn walk_near(to: WorldTile, radius: i32) -> impl Fn(&InteractReq) -> bool {
    move |op| {
        matches!(op, InteractReq::WalkNear { x, z, level, radius: r, .. }
            if *x == to.x && *z == to.z && *level == to.level && *r == radius)
    }
}

fn token(op: &InteractReq) -> u64 {
    match op {
        InteractReq::Walk { request_id, .. } | InteractReq::WalkNear { request_id, .. } => {
            *request_id
        }
        other => panic!("not a walk: {other:?}"),
    }
}

fn is_talk(op: &InteractReq) -> bool {
    matches!(op, InteractReq::Npc { name, action, index: Some(50) }
        if name == "Luthas" && action == "Talk-to")
}

fn is_search(id: i32) -> impl Fn(&InteractReq) -> bool {
    move |op| matches!(op, InteractReq::Loc { action, id: Some(got), .. } if action == "Search" && *got == id)
}

fn is_continue(op: &InteractReq) -> bool {
    matches!(op, InteractReq::ContinueDialog)
}

/// A failed walk whose navigator named `(id, count)` as the one short.
fn fail_short(world: &mut World, token: u64, dest: WorldTile, radius: i32, short: (i32, i32)) {
    world.tick += 1;
    post_walk_outcome(world.tick, token, world.here, dest, radius, true);
    observed::post(world.tick, |post| {
        post.walk_missing_carry(vec![CarryRow {
            id: short.0,
            count: short.1,
            name: Some("Coins".into()),
        }]);
    });
}

/// Talk-to Luthas, answer his option list with the preferred line, turn one
/// Continue page, then let the chat close.
fn talk(world: &mut World, options: &[&str], want: i32, after_answer: impl FnOnce(&mut World)) {
    until(world, "talk to Luthas", is_talk);
    world.modal = 4882;
    world.options = options.iter().map(|o| (*o).to_string()).collect();
    let answer = until(world, "answer Luthas", |op| {
        matches!(op, InteractReq::Answer { .. })
    });
    assert_eq!(answer, InteractReq::Answer { option: want });
    world.modal = 4883;
    world.options.clear();
    world.cont = true;
    after_answer(world);
    until(world, "continue Luthas", is_continue);
    world.modal = -1;
    world.cont = false;
}

fn start_walk_to(world: &mut World) -> machine::Handle {
    world.post();
    let args = json!({
        "tile": { "x": PORT_SARIM.x, "z": PORT_SARIM.z, "level": 0 },
    });
    match machine::start("walk-to", args, Vec::new(), 0) {
        Started::Running(handle) => handle,
        other => panic!("walk-to runs, got {other:?}"),
    }
}

/// The whole plantation job up to the re-walk to the boat; returns the
/// row, the world, the first walk and the re-walk.
fn earn_the_fare() -> (machine::Handle, World, InteractReq, InteractReq) {
    reset();
    let mut world = World::new(MUSA, 5);
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    assert!(
        matches!(first.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "the first walk goes out in the caller's tick: {first:?}"
    );

    // The navigator refuses the boat: 5 coins, the 30-coin fare missing.
    fail_short(&mut world, token(&first[0]), PORT_SARIM, 2, (995, 30));
    let luthas = until(&mut world, "walk to Luthas", walk_near(LUTHAS, 2));
    assert_ne!(token(&luthas), token(&first[0]));
    world.here = LUTHAS;

    // Employment (luthas.rs2:8–10, the multi2's first line).
    talk(
        &mut world,
        &[
            "Could you offer me employment on your plantation?",
            "That customs officer is annoying isn't she?",
        ],
        1,
        |_| {},
    );

    // fillCrate: the empty crate, then the grove.
    until(&mut world, "walk to the crate", walk_near(CRATE, 2));
    world.here = CRATE;
    until(&mut world, "search the crate", is_search(2072));
    world.say("The crate is completely empty.");
    until(&mut world, "walk to the grove", walk_near(GROVE, 4));
    world.here = GROVE;
    for picked in 0..10 {
        until(&mut world, "search a banana tree", is_search(2073));
        world.bananas = picked + 1;
        world.say("You pick a banana.");
    }

    // Pack each banana (banana_crate.rs2 `oplocu`): one use per banana.
    until(&mut world, "walk back to the crate", walk_near(CRATE, 2));
    world.here = CRATE;
    for packed in 0..10 {
        let use_on = until(&mut world, "use a banana on the crate", |op| {
            matches!(op, InteractReq::UseOn { .. })
        });
        match use_on {
            InteractReq::UseOn {
                kind,
                x,
                z,
                source_item_id,
                ..
            } => {
                assert_eq!(kind, "loc");
                assert_eq!((x, z), (CRATE.x, CRATE.z));
                assert_eq!(source_item_id, Some(1963));
            }
            _ => unreachable!(),
        }
        world.bananas = 9 - packed;
    }
    assert_eq!(world.bananas, 0);

    // The closing read says full; back to Luthas for the 30 coins.
    until(&mut world, "re-read the crate", is_search(2072));
    world.say("The crate is full of bananas.");
    until(&mut world, "walk back to Luthas", walk_near(LUTHAS, 2));
    world.here = LUTHAS;
    talk(
        &mut world,
        &[
            "Will you pay me for another crate full?",
            "Thank you, I'll be on my way",
            "So where are these bananas going to be delivered to?",
            "That customs officer is annoying isn't she?",
        ],
        2,
        |world| {
            world.coins = 35;
            world.say("Luthas hands you 30 coins.");
        },
    );

    let again = until(
        &mut world,
        "walk to the boat again",
        walk_near(PORT_SARIM, 2),
    );
    (h, world, first[0].clone(), again)
}

#[test]
fn walk_to_earns_the_fare_at_the_plantation_then_takes_the_boat() {
    let (h, mut world, first, again) = earn_the_fare();
    assert_eq!(machine::take(h), Take::Pending);
    world.here = PORT_SARIM;
    world.post();
    assert!(tick().is_empty());
    assert_ne!(token(&again), token(&first));
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(true))));
}

#[test]
fn an_interrupt_during_the_re_walk_stops_it_and_ends_false() {
    // Frozen follow pass: EventSignal.pending ends the walk (WalkExecutor.ts:844-853).
    let (h, mut world, _, again) = earn_the_fare();
    world.ours = true;
    world.post();
    assert_eq!(
        tick(),
        vec![InteractReq::AbortWalk {
            request_id: token(&again)
        }],
        "the interrupted re-walk stops its host follow"
    );
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
}

#[test]
fn walk_to_does_not_recover_a_short_that_is_not_the_boat_fare() {
    for (from, short) in [
        // Karamja, but the missing gate is a 10-coin toll.
        (MUSA, (995, 10)),
        // The fare row, but walking from the mainland.
        (
            WorldTile {
                x: 3100,
                z: 3200,
                level: 0,
            },
            (995, 30),
        ),
    ] {
        reset();
        let mut world = World::new(from, 5);
        let h = start_walk_to(&mut world);
        let first = machine::merge_ops(Vec::new());
        fail_short(&mut world, token(&first[0]), PORT_SARIM, 2, short);
        assert!(
            tick().is_empty(),
            "{short:?} from {from:?}: no recovery walk"
        );
        assert_eq!(
            machine::take(h),
            Take::Settled(Outcome::Done(json!(false))),
            "frozen walkTo returns the failed walk as is"
        );
    }
}

#[test]
fn walk_to_with_a_full_pack_and_no_banana_does_not_recover() {
    reset();
    let mut world = World::new(MUSA, 5);
    world.filler = 27;
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    fail_short(&mut world, token(&first[0]), PORT_SARIM, 2, (995, 30));
    assert!(tick().is_empty(), "no free slot: no walk to Luthas");
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
}

#[test]
fn walk_resilient_baked_failure_short_of_the_fare_goes_to_luthas() {
    reset();
    let mut world = World::new(MUSA, 5);
    world.post();
    let args = json!({
        "tile": { "x": PORT_SARIM.x, "z": PORT_SARIM.z, "level": 0 },
        "opts": { "radius": 2 },
    });
    let Started::Running(h) = machine::start("walk-resilient", args, Vec::new(), 0) else {
        panic!("walk-resilient runs");
    };
    machine::step(&mut NoJs);
    let baked = machine::merge_ops(Vec::new());
    assert!(
        matches!(baked.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "{baked:?}"
    );
    fail_short(&mut world, token(&baked[0]), PORT_SARIM, 2, (995, 30));
    let ops = tick();
    assert!(
        matches!(ops.as_slice(), [op] if walk_near(LUTHAS, 2)(op)),
        "frozen walkResilient's baked Traversal.walkTo recovers the fare, not a scene step: {ops:?}"
    );
    assert_eq!(machine::take(h), Take::Pending);
}

#[test]
fn walk_to_waits_out_the_frozen_300_second_default() {
    // Frozen `opts?.timeoutMs ?? 300_000` (WalkExecutor.ts:228).
    reset();
    let mut world = World::new(
        WorldTile {
            x: 3100,
            z: 3200,
            level: 0,
        },
        5,
    );
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    assert!(
        matches!(first.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "{first:?}"
    );
    machine::age(h, 60_001);
    world.post();
    assert!(tick().is_empty(), "no stop at a minute");
    assert_eq!(machine::take(h), Take::Pending);
    machine::age(h, 240_000);
    world.post();
    assert_eq!(
        tick(),
        vec![InteractReq::AbortWalk {
            request_id: token(&first[0])
        }],
        "the bound stops the host follow"
    );
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
}

#[test]
fn pause_and_resume_mid_leg_reissues_the_same_walk() {
    // Operator Pause drops the host route (`abort_script_walk`); frozen
    // resumes the same walk and repaths.
    reset();
    let mut world = World::new(MUSA, 5);
    world.post();
    let args = json!({
        "tile": { "x": PORT_SARIM.x, "z": PORT_SARIM.z, "level": 0 },
        "radius": 2,
    });
    let Started::Running(h) = machine::start("walk-to", args, Vec::new(), 0) else {
        panic!("walk-to runs");
    };
    let first = machine::merge_ops(Vec::new());
    fail_short(&mut world, token(&first[0]), PORT_SARIM, 2, (995, 30));
    let leg = until(&mut world, "walk to Luthas", walk_near(LUTHAS, 2));
    crate::walk_wait::on_pause();
    crate::walk_wait::on_operator_pause();
    machine::on_pause();
    world.post();
    assert!(tick().is_empty(), "a paused leg sends nothing");
    crate::walk_wait::on_resume();
    machine::on_resume();
    world.post();
    assert_eq!(
        tick(),
        vec![leg.clone()],
        "Resume re-issues the same request"
    );
    world.post();
    assert!(tick().is_empty(), "once");
    world.here = LUTHAS;
    until(&mut world, "talk to Luthas after the resumed leg", is_talk);
    assert_eq!(machine::take(h), Take::Pending);
}
