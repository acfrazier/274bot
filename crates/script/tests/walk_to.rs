// Task 8: compiled WalkTo. The port wraps `ctx.walk` (the slot's traveller
// hook), so `script` keeps no `nav` dependency: the port defines its own
// `Tile` and the factory hardcodes the rs2b0t default destination
// (Lumbridge) until the params editor lands. If the host has no traveller
// wired, `tick` errors instead of faking arrival.

use api::interact::Driver;
use api::prot::Out;
use script::ported::walk_to::{Tile, WalkToBot, DEFAULT_RADIUS};
use script::{CompiledId, Script, ScriptCtx};

/// Outbound writes a driver receives, as recorded by the stub.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OutByte {
    Enc(i32),
    P1(i32),
    P2(i32),
    P4(i32),
    Jstr(String),
}

#[derive(Default)]
struct OutSink(Vec<OutByte>);

impl Out for OutSink {
    fn p1_enc(&mut self, opcode: i32) {
        self.0.push(OutByte::Enc(opcode));
    }
    fn p1(&mut self, value: i32) {
        self.0.push(OutByte::P1(value));
    }
    fn p2(&mut self, value: i32) {
        self.0.push(OutByte::P2(value));
    }
    fn p4(&mut self, value: i32) {
        self.0.push(OutByte::P4(value));
    }
    fn pjstr(&mut self, s: &str) {
        self.0.push(OutByte::Jstr(s.to_string()));
    }
}

/// Recording driver stub: accepts every call, never sends.
#[derive(Default)]
struct Rec {
    out: OutSink,
}

impl Driver for Rec {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }
    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        true
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }
    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }
    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }
    fn out(&mut self) -> &mut dyn Out {
        &mut self.out
    }
    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

/// A ctx for one recorded tick: `here` where the player stands, `walk` the
/// host hook a walk request goes through.
fn ctx_with<'a>(
    d: &'a mut Rec,
    here: Option<(i32, i32, i32)>,
    walk: Option<&'a mut dyn FnMut(i32, i32, i32) -> bool>,
) -> ScriptCtx<'a> {
    ScriptCtx {
        driver: d,
        tick: 1,
        here,
        walk,
    }
}

#[test]
fn walk_to_requests_target_until_inside_radius() {
    let mut bot = WalkToBot::new(
        Tile {
            x: 10,
            z: 10,
            level: 0,
        },
        3,
    );
    let mut got = None;
    let mut d = Rec::default();
    let mut walk = |x: i32, z: i32, _l: i32| {
        got = Some((x, z));
        true
    };
    bot.tick(&mut ctx_with(&mut d, Some((0, 0, 0)), Some(&mut walk)));
    assert_eq!(got, Some((10, 10)));
}

#[test]
fn walk_to_noops_inside_radius() {
    let mut bot = WalkToBot::new(
        Tile {
            x: 10,
            z: 10,
            level: 0,
        },
        3,
    );
    let mut called = 0;
    let mut d = Rec::default();
    let mut walk = |_x: i32, _z: i32, _l: i32| {
        called += 1;
        true
    };
    // (10, 8) is 2 tiles from the target on the radius edge — no request.
    bot.tick(&mut ctx_with(&mut d, Some((10, 8, 0)), Some(&mut walk)));
    assert_eq!(called, 0);
}

#[test]
fn walk_to_waits_until_here_is_observed() {
    let mut bot = WalkToBot::new(
        Tile {
            x: 10,
            z: 10,
            level: 0,
        },
        3,
    );
    let mut called = 0;
    let mut d = Rec::default();
    let mut walk = |_x: i32, _z: i32, _l: i32| {
        called += 1;
        true
    };
    // The TS waits for `Game.ingame() && Game.tile() !== null` first.
    bot.tick(&mut ctx_with(&mut d, None, Some(&mut walk)));
    assert_eq!(called, 0);
}

#[test]
fn walk_to_without_walk_errors_instead_of_faking_arrival() {
    let mut bot = WalkToBot::new(
        Tile {
            x: 10,
            z: 10,
            level: 0,
        },
        3,
    );
    let mut d = Rec::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bot.tick(&mut ctx_with(&mut d, Some((0, 0, 0)), None));
    }));
    let payload = result.expect_err("no walk hook must error, never fake arrival");
    let msg = payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .expect("panic payload is a message");
    assert!(
        msg.contains("Traversal/nav not on ctx"),
        "error message was {msg}"
    );
}

#[test]
fn walk_rejected_retries_the_request_next_tick() {
    let mut bot = WalkToBot::new(
        Tile {
            x: 10,
            z: 10,
            level: 0,
        },
        3,
    );
    let mut calls = 0;
    let mut d = Rec::default();
    let mut walk = |x: i32, z: i32, _l: i32| {
        calls += 1;
        assert_eq!((x, z), (10, 10));
        false
    };
    bot.tick(&mut ctx_with(&mut d, Some((0, 0, 0)), Some(&mut walk)));
    bot.tick(&mut ctx_with(&mut d, Some((1, 0, 0)), Some(&mut walk)));
    assert_eq!(calls, 2);
}

#[test]
fn factory_walk_to_is_not_registered_until_the_traveller_hook_exists() {
    // The port itself is real, but `registry::factory` must not expose it:
    // Start would succeed and then panic on the first tick because the host
    // sets `ctx.walk = None`. A "not ported" Start is the honest surface
    // until host-play/panel wire a traveller into the ctx.
    assert!(
        script::factory(CompiledId("WalkTo")).is_none(),
        "WalkTo must not be startable while ctx.walk is always None"
    );
    // The constructor is still directly usable by the port's own tests.
    let bot = script::ported::walk_to::factory();
    assert_eq!(bot.name(), "WalkTo");
    assert_eq!(DEFAULT_RADIUS, 3, "rs2b0t arriveRadius default");
}
