use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetreatMode {
    Idle,
    EmitSet,
    EmitStatus,
    MaybeHop,
    Waiting,
    FailSet,
    NeedYield,
    Aborted,
}

pub(super) struct RetreatRuntime {
    pub(super) clock: InstantTaskClock,
    token: u64,
    mode: RetreatMode,
    dest: Option<Tile>,
    index: i32,
    next: i32,
    attempts_left: u32,
    hop_issued: bool,
    after_sustain: bool,
    rotated: Option<i32>,
}

impl RetreatRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: RetreatMode::Idle,
            dest: None,
            index: 0,
            next: 0,
            attempts_left: 0,
            hop_issued: false,
            after_sustain: false,
            rotated: None,
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

    fn clear_hop(&mut self) {
        self.mode = RetreatMode::Idle;
        self.hop_issued = false;
        self.after_sustain = false;
        self.dest = None;
        self.attempts_left = 0;
    }

    fn yield_clear(&mut self) -> Value {
        self.clear_hop();
        self.clock.deadline = None;
        self.emit(json!({ "kind": "yield" }))
    }

    fn yield_keep(&mut self) -> Value {
        self.clear_hop();
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = RetreatMode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }

    fn dest_json(&self) -> Value {
        let dest = self.dest.unwrap_or(Tile {
            x: 0,
            z: 0,
            level: 0,
        });
        json!({
            "kind": "walk-to",
            "x": dest.x,
            "z": dest.z,
            "level": dest.level,
        })
    }
}

fn with_retreat<T>(token: u64, f: impl FnOnce(&mut RetreatRuntime) -> T) -> Option<T> {
    RETREAT_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

fn retreat_spot(index: i32, spots: &[Tile]) -> Option<Tile> {
    usize::try_from(index)
        .ok()
        .and_then(|i| spots.get(i).copied())
}

fn retreat_validate_inner(rt: &RetreatRuntime, proj: &Projection, obs: &FightObservation) -> bool {
    let retry_open = match rt.clock.deadline {
        None => true,
        Some(_) => rt.clock.bound_reached(),
    };
    retry_open && retreat_due(proj, obs)
}

fn retreat_start(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let obs = observation();
    let Some(here) = obs.here else {
        return rt.yield_keep();
    };
    if proj.site.safespots.is_empty() {
        return rt.yield_keep();
    }
    let (index, next) = retreat_aim(rt.rotated, here, &proj.site.safespots);
    rt.rotated = None;
    let Some(dest) = retreat_spot(index, &proj.site.safespots) else {
        return rt.yield_keep();
    };
    rt.index = index;
    rt.next = next;
    rt.dest = Some(dest);
    rt.attempts_left = RETREAT_HOPS;
    rt.hop_issued = false;
    rt.after_sustain = false;
    rt.clock.deadline = None;
    rt.mode = RetreatMode::EmitSet;
    rt.emit(json!({ "kind": "set-safespot", "index": index }))
}

fn retreat_emit_status(rt: &mut RetreatRuntime) -> Value {
    rt.mode = RetreatMode::EmitStatus;
    rt.emit(json!({
        "kind": "status",
        "message": format!("retreating to safespot {}", rt.index),
    }))
}

fn retreat_emit_log(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let here = observation().here.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let hp = (proj.hp_fraction * 100.0).round() as i32;
    let pack = if proj.has_food {
        ""
    } else {
        " with an empty pack"
    };
    rt.mode = RetreatMode::MaybeHop;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "retreating to safespot {} at {},{},{} from {},{} at {hp}% hp{pack}",
            rt.index, dest.x, dest.z, dest.level, here.x, here.z
        ),
    }))
}

fn retreat_emit_hop(rt: &mut RetreatRuntime) -> Value {
    rt.attempts_left = rt.attempts_left.saturating_sub(1);
    rt.hop_issued = true;
    rt.after_sustain = false;
    rt.clock.arm(RETREAT_HOP_MS);
    rt.mode = RetreatMode::Waiting;
    rt.emit(rt.dest_json())
}

fn retreat_maybe_hop(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let obs = observation();
    let dest = match rt.dest {
        Some(d) => d,
        None => return rt.yield_clear(),
    };
    if at_tile(&obs, dest) {
        return rt.yield_clear();
    }
    if proj.died {
        return rt.yield_clear();
    }
    retreat_emit_hop(rt)
}

fn retreat_fail_log(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or(Tile {
        x: 0,
        z: 0,
        level: 0,
    });
    let stand = if proj.has_food {
        "Eating where we stand"
    } else {
        "Handing to the bank run"
    };
    rt.mode = RetreatMode::FailSet;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "could not reach safespot {} at {},{},{}. {stand} and trying {} in {}s.",
            rt.index,
            dest.x,
            dest.z,
            dest.level,
            rt.next,
            RETREAT_RETRY_MS / 1000
        ),
    }))
}

fn retreat_poll_wait(rt: &mut RetreatRuntime, proj: &Projection) -> Value {
    let obs = observation();
    let dest = match rt.dest {
        Some(d) => d,
        None => return rt.yield_clear(),
    };
    if proj.died {
        return rt.yield_clear();
    }
    if at_tile(&obs, dest) {
        return rt.yield_clear();
    }
    if rt.hop_issued && (obs.hold || obs.ours) {
        return rt.yield_clear();
    }
    if rt.clock.bound_reached() {
        if rt.attempts_left > 0 {
            return retreat_emit_hop(rt);
        }
        return retreat_fail_log(rt, proj);
    }
    if !rt.after_sustain {
        rt.after_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.after_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn retreat_next_effect(rt: &mut RetreatRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == RetreatMode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) {
        return rt.aborted("unexpected eatOk");
    }
    if reply.is_some_and(|r| r.get("queued").is_some()) {
        return rt.aborted("unexpected queued");
    }
    if reply.is_some_and(|r| r.get("walkToken").is_some()) {
        return rt.aborted("unexpected walkToken");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        RetreatMode::Idle => retreat_start(rt, proj),
        RetreatMode::EmitSet => retreat_emit_status(rt),
        RetreatMode::EmitStatus => retreat_emit_log(rt, proj),
        RetreatMode::MaybeHop => retreat_maybe_hop(rt, proj),
        RetreatMode::Waiting => retreat_poll_wait(rt, proj),
        RetreatMode::FailSet => {
            rt.rotated = Some(rt.next);
            rt.clock.arm(RETREAT_RETRY_MS);
            rt.mode = RetreatMode::NeedYield;
            rt.emit(json!({ "kind": "set-safespot", "index": rt.next }))
        }
        RetreatMode::NeedYield => rt.yield_keep(),
        RetreatMode::Aborted => rt.aborted("aborted"),
    }
}

fn retreat_begin() -> Value {
    let token = alloc_token();
    RETREAT_RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, RetreatRuntime::new(token));
    });
    json!({ "kind": "started", "token": token })
}

pub fn retreat_dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => retreat_begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_retreat(
                token,
                |rt| json!({ "value": retreat_validate_inner(rt, &proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_retreat(token, |rt| retreat_next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn retreat_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_retreat(token, |rt| {
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

pub fn retreat_token_alive(token: u64) -> bool {
    RETREAT_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

/// Test helper: make `clock.bound_reached()` true without sleeping hop/retry.
pub fn retreat_force_bound_reached(token: u64) -> bool {
    with_retreat(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

/// `Retreat`.
pub(crate) struct RetreatKind;

impl HuntKind for RetreatKind {
    const NAME: &'static str = "hunt-retreat";
    const SESSION: bool = true;
    const BOOLEAN: bool = false;
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
        token_of(&retreat_begin())
    }

    fn ensure(token: u64) {
        RETREAT_RUNTIMES.with(|m| {
            m.borrow_mut()
                .entry(token)
                .or_insert_with(|| RetreatRuntime::new(token));
        });
    }

    fn renew(token: u64) {
        RETREAT_RUNTIMES.with(|m| m.borrow_mut().insert(token, RetreatRuntime::new(token)));
    }

    fn next(token: u64, proj: &Projection, reply: Option<&Value>) -> Value {
        with_retreat(token, |rt| retreat_next_effect(rt, proj, reply)).unwrap_or_else(unknown_token)
    }

    fn validate(token: u64, proj: &Projection) -> bool {
        let obs = observation();
        with_retreat(token, |rt| retreat_validate_inner(rt, proj, &obs)).unwrap_or(false)
    }
}
