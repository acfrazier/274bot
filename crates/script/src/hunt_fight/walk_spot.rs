use super::*;
use super::hold::{reply_u64, spot_name, walk_wait_settled};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkMode {
    Idle,
    PickLeg,
    NeedAck,
    Waiting,
    NeedYield,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkLeg {
    Approach,
    Dest,
}

pub(super) struct WalkRuntime {
    pub(super) clock: InstantTaskClock,
    token: u64,
    mode: WalkMode,
    dest: Option<Tile>,
    leg: Option<Tile>,
    approach_cursor: i32,
    leg_kind: WalkLeg,
    walk_token: Option<u64>,
    after_sustain: bool,
}

impl WalkRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: WalkMode::Idle,
            dest: None,
            leg: None,
            approach_cursor: 0,
            leg_kind: WalkLeg::Dest,
            walk_token: None,
            after_sustain: false,
        }
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    pub(super) fn apply_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn emit(&self, mut v: Value) -> Value {
        v["token"] = json!(self.token);
        v
    }

    fn yield_now(&mut self) -> Value {
        self.mode = WalkMode::Idle;
        self.walk_token = None;
        self.after_sustain = false;
        self.dest = None;
        self.leg = None;
        self.approach_cursor = 0;
        self.leg_kind = WalkLeg::Dest;
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = WalkMode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }
}

fn with_walk<T>(token: u64, f: impl FnOnce(&mut WalkRuntime) -> T) -> Option<T> {
    WALK_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn walk_validate_inner(proj: &Projection, obs: &FightObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    if !proj.site.area.contains(here, 1) {
        return false;
    }
    if chase_mode(proj.style, proj.site.fire_at_range) && proj.target_idx.is_some() {
        return false;
    }
    if proj.hp_fraction < proj.panic_hp {
        return false;
    }
    distance_to(anchor(proj), here) > APPROACH_RADIUS
}

fn walk_short_log(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    let name = spot_name(proj.style, proj.safespot_index);
    rt.mode = WalkMode::NeedYield;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "the walk in stopped short of {name} at ({}, {}, {}). Closing the gap from the walk-back task.",
            dest.x, dest.z, dest.level
        ),
    }))
}

fn walk_interrupt(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    let obs = observation();
    if at_tile(&obs, dest) {
        return rt.yield_now();
    }
    walk_short_log(rt, proj)
}

fn walk_sustain(rt: &mut WalkRuntime) -> Value {
    if !rt.after_sustain {
        rt.after_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.after_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn walk_emit_walk(rt: &mut WalkRuntime, tile: Tile, kind: WalkLeg) -> Value {
    rt.leg = Some(tile);
    rt.leg_kind = kind;
    rt.clock.arm(APPROACH_MS);
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.mode = WalkMode::NeedAck;
    rt.emit(json!({
        "kind": "walk",
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
    }))
}

fn walk_pick_leg(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    if observation().hold || observation().ours {
        return walk_interrupt(rt, proj);
    }
    if rt.leg_kind == WalkLeg::Approach {
        let here = observation().here;
        while let Some(stop) = usize::try_from(rt.approach_cursor)
            .ok()
            .and_then(|i| proj.site.approach.get(i).copied())
        {
            if here.is_some_and(|h| distance_to(stop, h) <= 1) {
                rt.approach_cursor += 1;
                continue;
            }
            return walk_emit_walk(rt, stop, WalkLeg::Approach);
        }
        rt.leg_kind = WalkLeg::Dest;
    }
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    rt.dest = Some(dest);
    walk_emit_walk(rt, dest, WalkLeg::Dest)
}

fn walk_advance_approach(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    rt.approach_cursor += 1;
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.mode = WalkMode::PickLeg;
    rt.leg_kind = WalkLeg::Approach;
    walk_pick_leg(rt, proj)
}

fn walk_poll(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let obs = observation();
    if obs.hold || obs.ours {
        return walk_interrupt(rt, proj);
    }
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    if rt.leg_kind == WalkLeg::Dest {
        if at_tile(&obs, dest) {
            return rt.yield_now();
        }
        let done = rt.walk_token.is_some_and(walk_wait_settled) || rt.clock.bound_reached();
        if done {
            let obs = observation();
            if at_tile(&obs, dest) {
                return rt.yield_now();
            }
            return walk_short_log(rt, proj);
        }
        return walk_sustain(rt);
    }
    let leg = rt.leg.unwrap_or(dest);
    let done = at_tile(&obs, leg)
        || rt.walk_token.is_some_and(walk_wait_settled)
        || rt.clock.bound_reached();
    if done {
        return walk_advance_approach(rt, proj);
    }
    walk_sustain(rt)
}

fn walk_start(rt: &mut WalkRuntime, proj: &Projection) -> Value {
    let dest = anchor(proj);
    rt.dest = Some(dest);
    rt.leg = None;
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.clock.deadline = None;
    let obs = observation();
    if let Some(here) = obs.here.filter(|_| !proj.site.approach.is_empty()) {
        rt.leg_kind = WalkLeg::Approach;
        rt.approach_cursor = nearest_spot(here, &proj.site.approach);
    } else {
        rt.leg_kind = WalkLeg::Dest;
        rt.approach_cursor = 0;
    }
    rt.mode = WalkMode::PickLeg;
    rt.emit(json!({ "kind": "status", "message": "walking to the fight spot" }))
}

fn walk_next_effect(rt: &mut WalkRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == WalkMode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) {
        return rt.aborted("unexpected eatOk");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        WalkMode::Idle => walk_start(rt, proj),
        WalkMode::PickLeg => walk_pick_leg(rt, proj),
        WalkMode::NeedAck => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let Some(walk_token) = reply_u64(reply, "walkToken") else {
                if queued {
                    return rt.aborted("unexpected queued");
                }
                return rt.aborted("missing walkToken");
            };
            rt.walk_token = Some(walk_token);
            rt.mode = WalkMode::Waiting;
            rt.after_sustain = false;
            walk_poll(rt, proj)
        }
        WalkMode::Waiting => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if queued {
                return rt.aborted("unexpected queued");
            }
            walk_poll(rt, proj)
        }
        WalkMode::NeedYield => rt.yield_now(),
        WalkMode::Aborted => rt.aborted("aborted"),
    }
}

fn walk_begin() -> Value {
    let token = alloc_token();
    WALK_RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, WalkRuntime::new(token));
    });
    json!({ "kind": "started", "token": token })
}

pub fn walk_dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => walk_begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_walk(
                token,
                |_| json!({ "value": walk_validate_inner(&proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_walk(token, |rt| walk_next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn walk_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_walk(token, |rt| {
        rt.clock.deadline.map(|d| {
            if d > rt.now() {
                d.saturating_duration_since(rt.now()).as_millis() as i64
            } else {
                0
            }
        })
    })
    .flatten()
}

pub fn walk_token_alive(token: u64) -> bool {
    WALK_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

/// Test helper: make `clock.bound_reached()` true without sleeping `APPROACH_MS`.
pub fn walk_force_bound_reached(token: u64) -> bool {
    with_walk(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

/// `WalkToSpot`.
pub(crate) struct WalkSpotKind;

impl HuntKind for WalkSpotKind {
    const NAME: &'static str = "hunt-walkspot";
    const SESSION: bool = true;
    const BOOLEAN: bool = false;
    const WORLD_WALK: bool = true;
    type Proj = Projection;

    fn parse(site: &Value) -> Projection {
        parse_projection(site)
    }

    fn area(proj: &mut Projection) -> &mut Area {
        &mut proj.site.area
    }

    fn refresh(proj: &mut Projection, host: &mut dyn Host) -> Result<(), Ended> {
        refresh_projection(proj, host)
    }

    fn mint() -> u64 {
        token_of(&walk_begin())
    }

    fn ensure(token: u64) {
        WALK_RUNTIMES.with(|m| {
            m.borrow_mut()
                .entry(token)
                .or_insert_with(|| WalkRuntime::new(token));
        });
    }

    fn renew(token: u64) {
        WALK_RUNTIMES.with(|m| m.borrow_mut().insert(token, WalkRuntime::new(token)));
    }

    fn next(token: u64, proj: &Projection, reply: Option<&Value>) -> Value {
        with_walk(token, |rt| walk_next_effect(rt, proj, reply)).unwrap_or_else(unknown_token)
    }

    fn validate(_token: u64, proj: &Projection) -> bool {
        walk_validate_inner(proj, &observation())
    }
}
