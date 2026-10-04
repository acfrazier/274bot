use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoldMode {
    Idle,
    NeedWalk,
    NeedAck,
    Waiting,
    RotateSet,
    NeedYield,
    Aborted,
}

pub(super) struct HoldRuntime {
    pub(super) clock: InstantTaskClock,
    token: u64,
    mode: HoldMode,
    dest: Option<Tile>,
    walk_token: Option<u64>,
    after_sustain: bool,
    threats: ThreatSet,
    pending_protect: PendingProtect,
    deferred_effect: Option<Value>,
    tables: Option<Arc<CombatTables>>,
    rotate_index: Option<i32>,
}

impl HoldRuntime {
    fn new(token: u64) -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token,
            mode: HoldMode::Idle,
            dest: None,
            walk_token: None,
            after_sustain: false,
            rotate_index: None,
            threats: ThreatSet::default(),
            pending_protect: PendingProtect::default(),
            deferred_effect: None,
            tables: None,
        }
    }

    fn protect_click(&mut self) -> Result<Option<i32>, ActionError> {
        if !protect_observed() {
            return Ok(None);
        }
        let tables = match self.tables.as_ref() {
            Some(tables) => tables,
            None => self.tables.insert(hunt_combat_tables()?),
        };
        Ok(protect_button(
            &mut self.threats,
            &mut self.pending_protect,
            tables,
        ))
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
        self.mode = HoldMode::Idle;
        self.walk_token = None;
        self.after_sustain = false;
        self.rotate_index = None;
        self.emit(json!({ "kind": "yield" }))
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.mode = HoldMode::Aborted;
        self.emit(json!({ "kind": "aborted", "reason": reason }))
    }
}

fn with_hold<T>(token: u64, f: impl FnOnce(&mut HoldRuntime) -> T) -> Option<T> {
    HOLD_RUNTIMES.with(|m| m.borrow_mut().get_mut(&token).map(f))
}

pub(super) fn spot_name(style: Style, index: i32) -> String {
    if uses_safespot(style) {
        format!("safespot {index}")
    } else {
        "the melee anchor".to_string()
    }
}

fn hold_validate_inner(proj: &Projection, obs: &FightObservation) -> bool {
    let Some(here) = obs.here else {
        return false;
    };
    if !proj.site.area.contains(here, 1) {
        return false;
    }
    if chase_mode(proj.style, proj.site.fire_at_range) && proj.target_idx.is_some() {
        return false;
    }
    if at_tile(obs, anchor(proj)) {
        return false;
    }
    if !hold_due(proj, obs) {
        return false;
    }
    proj.hp_fraction >= proj.panic_hp
}

pub(super) fn walk_wait_settled(token: u64) -> bool {
    crate::walk_wait::dispatch(&json!({ "op": "settled", "token": token }))
        .as_bool()
        .unwrap_or(false)
}

pub(super) fn reply_u64(reply: Option<&Value>, key: &str) -> Option<u64> {
    let v = reply?.get(key)?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
}

fn hold_rotate_index(proj: &Projection) -> i32 {
    let spots = proj.site.safespots.len();
    if uses_safespot(proj.style) && spots > 1 {
        (proj.safespot_index.max(0) as usize + 1) % spots
    } else {
        proj.safespot_index.max(0) as usize
    }
    .try_into()
    .unwrap_or(0)
}

fn hold_emit_rotate(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    let next = hold_rotate_index(proj);
    let name = spot_name(proj.style, proj.safespot_index);
    let extra = if next == proj.safespot_index {
        String::new()
    } else {
        format!(". Rotating to {next}")
    };
    rt.rotate_index = Some(next);
    rt.mode = HoldMode::RotateSet;
    rt.emit(json!({
        "kind": "log",
        "message": format!(
            "{name} at {},{},{} could not be reached{extra}.",
            dest.x, dest.z, dest.level
        ),
    }))
}

fn hold_poll_wait(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    let obs = observation();
    if obs.hold || obs.ours {
        return rt.yield_now();
    }
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    if at_tile(&obs, dest) {
        return rt.yield_now();
    }
    let wait_done = rt.walk_token.is_some_and(walk_wait_settled) || rt.clock.bound_reached();
    if wait_done {
        if at_tile(&obs, dest) {
            return rt.yield_now();
        }
        return hold_emit_rotate(rt, proj);
    }
    if !rt.after_sustain {
        rt.after_sustain = true;
        return rt.emit(json!({ "kind": "sustain" }));
    }
    rt.after_sustain = false;
    rt.emit(json!({ "kind": "delay-ticks", "n": 1 }))
}

fn hold_start(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    rt.clock.arm(RETURN_MS);
    rt.dest = Some(anchor(proj));
    rt.walk_token = None;
    rt.after_sustain = false;
    rt.rotate_index = None;
    rt.mode = HoldMode::NeedWalk;
    let name = spot_name(proj.style, proj.safespot_index);
    rt.emit(json!({ "kind": "status", "message": format!("returning to {name}") }))
}

fn hold_emit_walk(rt: &mut HoldRuntime, proj: &Projection) -> Value {
    let dest = rt.dest.unwrap_or_else(|| anchor(proj));
    rt.dest = Some(dest);
    rt.mode = HoldMode::NeedAck;
    rt.emit(json!({
        "kind": "walk",
        "x": dest.x,
        "z": dest.z,
        "level": dest.level,
    }))
}

fn hold_next_effect(rt: &mut HoldRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    // Resume the deferred mode effect on the protect's ack. In particular,
    // Waiting never sees queued, and NeedAck keeps its own walkToken reply.
    if let Some(effect) = rt.deferred_effect.take() {
        return effect;
    }
    let effect = hold_next_mode_effect(rt, proj, reply);
    if matches!(effect["kind"].as_str(), Some("yield" | "aborted" | "wait")) {
        return effect;
    }
    match rt.protect_click() {
        Ok(Some(component_id)) => {
            rt.deferred_effect = Some(effect);
            rt.emit(json!({ "kind": "if-button", "component_id": component_id }))
        }
        Ok(None) => effect,
        Err(error) => {
            let reason = format!("Hunt protection unavailable: {error:?}");
            let mut effect = rt.aborted(&reason);
            effect["kind"] = json!("failed");
            effect["notes"] = json!([{ "kind": "log", "message": reason }]);
            effect
        }
    }
}

fn hold_next_mode_effect(rt: &mut HoldRuntime, proj: &Projection, reply: Option<&Value>) -> Value {
    if rt.mode == HoldMode::Aborted {
        return rt.aborted("aborted");
    }
    if reply.is_some_and(|r| r.get("eatOk").is_some()) {
        return rt.aborted("unexpected eatOk");
    }
    if rt.clock.frozen() {
        return rt.emit(json!({ "kind": "wait" }));
    }
    match rt.mode {
        HoldMode::Idle => hold_start(rt, proj),
        HoldMode::NeedWalk => hold_emit_walk(rt, proj),
        HoldMode::NeedAck => {
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
            rt.mode = HoldMode::Waiting;
            rt.after_sustain = false;
            hold_poll_wait(rt, proj)
        }
        HoldMode::Waiting => {
            let queued = reply
                .and_then(|r| r.get("queued"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if queued {
                return rt.aborted("unexpected queued");
            }
            hold_poll_wait(rt, proj)
        }
        HoldMode::RotateSet => {
            let index = rt.rotate_index.unwrap_or(proj.safespot_index);
            rt.mode = HoldMode::NeedYield;
            rt.emit(json!({ "kind": "set-safespot", "index": index }))
        }
        HoldMode::NeedYield => rt.yield_now(),
        HoldMode::Aborted => rt.aborted("aborted"),
    }
}

fn hold_begin() -> Value {
    let token = alloc_token();
    HOLD_RUNTIMES.with(|m| {
        m.borrow_mut().insert(token, HoldRuntime::new(token));
    });
    json!({ "kind": "started", "token": token })
}

pub fn hold_dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => hold_begin(),
        "validate" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let obs = observation();
            match with_hold(
                token,
                |_| json!({ "value": hold_validate_inner(&proj, &obs), "token": token }),
            ) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token", "value": false }),
            }
        }
        "next" => {
            let token = token_of(input);
            let proj = parse_projection(input);
            let reply = input.get("reply");
            match with_hold(token, |rt| hold_next_effect(rt, &proj, reply)) {
                Some(v) => v,
                None => json!({ "kind": "aborted", "reason": "unknown token" }),
            }
        }
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

pub fn hold_deadline_remaining_ms(token: u64) -> Option<i64> {
    with_hold(token, |rt| {
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

pub fn hold_token_alive(token: u64) -> bool {
    HOLD_RUNTIMES.with(|m| m.borrow().contains_key(&token))
}

/// Test helper: make `clock.bound_reached()` true without sleeping `RETURN_MS`.
pub fn hold_force_bound_reached(token: u64) -> bool {
    with_hold(token, |rt| {
        rt.clock.deadline = Some(rt.clock.now());
        true
    })
    .unwrap_or(false)
}

/// `HoldSafespot`.
pub(crate) struct HoldKind;

impl HuntKind for HoldKind {
    const NAME: &'static str = "hunt-hold";
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
        token_of(&hold_begin())
    }

    fn ensure(token: u64) {
        HOLD_RUNTIMES.with(|m| {
            m.borrow_mut()
                .entry(token)
                .or_insert_with(|| HoldRuntime::new(token));
        });
    }

    fn renew(token: u64) {
        HOLD_RUNTIMES.with(|m| m.borrow_mut().insert(token, HoldRuntime::new(token)));
    }

    fn next(token: u64, proj: &Projection, reply: Option<&Value>) -> Value {
        with_hold(token, |rt| hold_next_effect(rt, proj, reply)).unwrap_or_else(unknown_token)
    }

    fn validate(_token: u64, proj: &Projection) -> bool {
        hold_validate_inner(proj, &observation())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_consumer_emits_the_native_missiles_if_button() {
        let button_com = super::super::seed_protect_observation();
        let mut runtime = HoldRuntime::new(23);
        let projection = super::super::parse_projection(&serde_json::json!({}));

        let effect = hold_next_effect(&mut runtime, &projection, None);
        assert_eq!(effect["kind"], "if-button");
        assert_eq!(effect["component_id"], button_com);
    }

    #[test]
    fn hold_waiting_routes_the_protect_ack_without_aborting_or_advancing_the_mode() {
        let button = seed_protect_observation();
        let mut runtime = HoldRuntime::new(23);
        runtime.mode = HoldMode::Waiting;
        runtime.dest = Some(Tile {
            x: 1,
            z: 1,
            level: 0,
        });
        runtime.clock.arm(RETURN_MS);
        let projection = parse_projection(&json!({}));

        let effect = hold_next_effect(&mut runtime, &projection, None);
        assert_eq!(effect["kind"], "if-button");
        assert_eq!(effect["component_id"], button);
        assert_eq!(runtime.mode, HoldMode::Waiting);
        assert!(
            runtime.after_sustain,
            "the mode produced, but has not emitted, sustain"
        );
        let resumed = hold_next_effect(&mut runtime, &projection, Some(&json!({ "queued": true })));
        assert_eq!(resumed["kind"], "sustain");
        assert_eq!(runtime.mode, HoldMode::Waiting);
        assert_eq!(
            hold_next_effect(&mut runtime, &projection, None)["kind"],
            "delay-ticks"
        );
    }

    #[test]
    fn hold_need_ack_keeps_its_walk_token_when_protection_changes() {
        let button = seed_protect_observation();
        let mut runtime = HoldRuntime::new(23);
        runtime.mode = HoldMode::NeedAck;
        runtime.dest = Some(Tile {
            x: 1,
            z: 1,
            level: 0,
        });
        runtime.clock.arm(RETURN_MS);
        let projection = parse_projection(&json!({}));

        let effect = hold_next_effect(
            &mut runtime,
            &projection,
            Some(&json!({ "walkToken": 991 })),
        );
        assert_eq!(effect["kind"], "if-button");
        assert_eq!(effect["component_id"], button);
        assert_eq!(runtime.walk_token, Some(991));
        assert_eq!(runtime.mode, HoldMode::Waiting);
        assert_eq!(
            hold_next_effect(&mut runtime, &projection, Some(&json!({ "queued": true })))["kind"],
            "sustain"
        );
        assert_eq!(runtime.walk_token, Some(991));
        assert_eq!(
            hold_next_effect(&mut runtime, &projection, None)["kind"],
            "delay-ticks"
        );
    }

    #[test]
    fn hold_reports_unavailable_protection_instead_of_silently_disabling_it() {
        seed_protect_observation();
        crate::supply_v2::configure(None);
        let mut runtime = HoldRuntime::new(23);
        let effect = hold_next_effect(&mut runtime, &parse_projection(&json!({})), None);
        assert_eq!(effect["kind"], "failed");
        assert!(effect["reason"]
            .as_str()
            .unwrap()
            .contains("game data unavailable"));
        assert_eq!(effect["notes"][0]["message"], effect["reason"]);
    }
}
