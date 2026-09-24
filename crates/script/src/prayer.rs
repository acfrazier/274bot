//! Rust-owned prayer toggle and overlay sweep, a [`crate::machine`]
//! family. JavaScript starts `set`/`clear` with the caller name and the
//! classified `on`, and awaits one envelope; Rust owns the click, the
//! observed-varp wait, the overlay order and the deadline. The frozen
//! `points|max|full|known|available|active` queries stay one native call
//! each over the same selected rows.

use crate::machine::{self, Begin, Cx, Family, Step};
use crate::observed::{self, Scene};
use crate::shim::InteractReq;
use api::game_data::SelectedGameData;
use api::prayer::{
    active, available, lookup, matches_on, max, on_is_truthy, points, OnArg, PrayerObservation,
    PRAYER_COUNT, TOGGLE_MS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Prayer points/max and the overlay varps, read from the isolate scene. A
/// logout forgets the session: only pages posted since login count. A varp
/// row the last varps page did not carry stays unobserved.
fn prayer_observation(scene: &Scene) -> PrayerObservation {
    let session = scene.since_login();
    let mut obs = PrayerObservation::empty();
    if let Some(row) = session.stats().and_then(|skills| skills.prayer) {
        obs.points = row.effective;
        obs.max = row.base;
    }
    for row in session.varps().into_iter().flatten() {
        // Rows outside the 15 overlay varps are ignored by `set_varp`.
        obs.set_varp(row.index, row.value);
    }
    obs
}

/// The frozen `on` argument, already classified by the shim's coercion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum OnInput {
    Boolean {
        value: bool,
    },
    Other {
        truthy: bool,
    },
    #[default]
    Undefined,
}

impl OnInput {
    fn on_arg(self) -> OnArg {
        match self {
            Self::Boolean { value } => OnArg::Bool(value),
            Self::Other { truthy } => OnArg::Other { truthy },
            Self::Undefined => OnArg::Undefined,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Op {
    Set,
    Clear,
}

/// What a second start of this family does. The two declared surfaces
/// differ and Rust keeps both: v1 has no guard, so a newer set supersedes
/// the older row; v2 admits one operation and refuses the next `busy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Admit {
    #[default]
    Supersede,
    RefuseBusy,
}

#[derive(Deserialize)]
pub(crate) struct PrayerArgs {
    op: Op,
    #[serde(default)]
    name: String,
    #[serde(default)]
    on: OnInput,
    #[serde(default)]
    admit: Admit,
}

/// One settlement: the frozen `ok`/`reason` pair plus the value the
/// declared return carries when it has one.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct PrayerDone {
    ok: bool,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
}

impl PrayerDone {
    fn matched(reason: &'static str) -> Self {
        Self {
            ok: true,
            reason,
            value: Some(json!(true)),
        }
    }

    fn cleared(clicked: u32, timed_out: u32) -> Self {
        Self {
            ok: true,
            reason: "cleared",
            value: Some(json!({ "clicked": clicked, "timed_out": timed_out })),
        }
    }

    fn failed(reason: &'static str) -> Self {
        Self {
            ok: false,
            reason,
            value: None,
        }
    }
}

impl From<PrayerDone> for Value {
    fn from(done: PrayerDone) -> Self {
        serde_json::to_value(done).unwrap_or(Value::Null)
    }
}

/// `Prayer.set`: one toggle click out, then the observed varp.
pub(crate) struct Toggle {
    want: OnArg,
    varp: i32,
}

impl Toggle {
    fn step(&self, obs: &PrayerObservation, cx: &mut Cx<'_>) -> Step<PrayerDone> {
        // A matching observation wins over the deadline: a slow page that
        // arrives late still settles the toggle.
        if match self.want {
            OnArg::Bool(true) => obs.is_on(self.varp),
            OnArg::Bool(false) => obs.is_off(self.varp),
            OnArg::Undefined | OnArg::Other { .. } => false,
        } {
            return Step::Done(PrayerDone::matched("toggled"));
        }
        if cx.clock().bound_reached() {
            return Step::Done(PrayerDone::failed("toggle-timeout"));
        }
        Step::Wait
    }
}

/// `Prayer.clear`: click every active overlay in selected order.
pub(crate) struct Clear {
    index: usize,
    clicked: u32,
    timed_out: u32,
    varp: i32,
}

/// What one overlay sweep pass decided.
enum Sweep {
    Clicked,
    Done(PrayerDone),
}

impl Clear {
    fn step(
        &mut self,
        data: Option<&SelectedGameData>,
        obs: &PrayerObservation,
        cx: &mut Cx<'_>,
    ) -> Step<PrayerDone> {
        let settled = if obs.is_off(self.varp) {
            true
        } else if cx.clock().bound_reached() {
            // A dropped click is not the sweep's end: count it and take
            // the next active overlay.
            self.timed_out = self.timed_out.saturating_add(1);
            true
        } else {
            false
        };
        if !settled {
            return Step::Wait;
        }
        match self.sweep(data, obs, cx) {
            Sweep::Clicked => Step::Wait,
            Sweep::Done(done) => Step::Done(done),
        }
    }

    /// Click the next observed-active overlay row, or settle with counts.
    fn sweep(
        &mut self,
        data: Option<&SelectedGameData>,
        obs: &PrayerObservation,
        cx: &mut Cx<'_>,
    ) -> Sweep {
        if let Some(data) = data {
            let rows = data.prayers();
            while self.index < rows.len() && self.index < PRAYER_COUNT {
                let row = &rows[self.index];
                self.index += 1;
                if obs.is_on(row.varp) {
                    self.clicked = self.clicked.saturating_add(1);
                    self.varp = row.varp;
                    cx.clock().arm(TOGGLE_MS);
                    cx.emit(InteractReq::IfButton {
                        component_id: row.button_com,
                    });
                    return Sweep::Clicked;
                }
            }
        }
        Sweep::Done(PrayerDone::cleared(self.clicked, self.timed_out))
    }
}

/// One `set` or `clear`; the row is the whole operation's state.
pub(crate) enum Prayer {
    Toggle(Toggle),
    Clear(Clear),
}

impl Family for Prayer {
    const NAME: &'static str = "prayer";
    /// One row per isolate: a newer start ends the older one. `admit`
    /// decides what a start while this row runs does: `refuse-busy` (the
    /// v2 surface) refuses before anything is emitted; `supersede` (v1,
    /// which has no guard) lets the newer start end the older row.
    const EXCLUSIVE: bool = true;
    type Args = PrayerArgs;
    type Output = PrayerDone;

    fn begin(args: PrayerArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        if args.admit == Admit::RefuseBusy && machine::live(Self::NAME) {
            return Begin::Refuse("busy".into());
        }
        let data = crate::supply_v2::selected_data();
        let obs = observed::with(prayer_observation);
        match args.op {
            Op::Set => begin_set(data.as_deref(), &args, &obs, cx),
            Op::Clear => begin_clear(data.as_deref(), &obs, cx),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<PrayerDone> {
        let data = crate::supply_v2::selected_data();
        let obs = observed::with(prayer_observation);
        match self {
            Self::Toggle(toggle) => toggle.step(&obs, cx),
            Self::Clear(clear) => clear.step(data.as_deref(), &obs, cx),
        }
    }
}

fn begin_set(
    data: Option<&SelectedGameData>,
    args: &PrayerArgs,
    obs: &PrayerObservation,
    cx: &mut Cx<'_>,
) -> Begin<Prayer> {
    let Some(data) = data else {
        return Begin::Done(PrayerDone::failed("unknown-prayer"));
    };
    let Some(row) = lookup(data, &args.name) else {
        return Begin::Done(PrayerDone::failed("unknown-prayer"));
    };
    let want = args.on.on_arg();
    if matches_on(active(data, &args.name, obs), want) {
        return Begin::Done(PrayerDone::matched("matched"));
    }
    if on_is_truthy(want) && !available(data, &args.name, obs) {
        return Begin::Done(PrayerDone::failed("unavailable"));
    }
    cx.clock().arm(TOGGLE_MS);
    cx.emit(InteractReq::IfButton {
        component_id: row.button_com,
    });
    Begin::Run(Prayer::Toggle(Toggle {
        want,
        varp: row.varp,
    }))
}

fn begin_clear(
    data: Option<&SelectedGameData>,
    obs: &PrayerObservation,
    cx: &mut Cx<'_>,
) -> Begin<Prayer> {
    let mut clear = Clear {
        index: 0,
        clicked: 0,
        timed_out: 0,
        varp: -1,
    };
    match clear.sweep(data, obs, cx) {
        Sweep::Clicked => Begin::Run(Prayer::Clear(clear)),
        Sweep::Done(done) => Begin::Done(done),
    }
}

fn helper_ok(value: Value) -> Value {
    json!({ "ok": true, "value": value })
}

fn query(data: Option<&SelectedGameData>, obs: &PrayerObservation, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "points" => helper_ok(json!(points(obs))),
        "max" => helper_ok(json!(max(obs))),
        "full" => helper_ok(json!(api::prayer::full(obs))),
        "known" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            helper_ok(json!(
                data.is_some_and(|data| api::prayer::known(data, name))
            ))
        }
        "available" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            helper_ok(json!(data.is_some_and(|data| available(data, name, obs))))
        }
        "active" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            helper_ok(json!(data.is_some_and(|data| active(data, name, obs))))
        }
        _ => json!({ "ok": false, "error": "unknown-op" }),
    }
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    let obs = observed::with(prayer_observation);
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "points" | "max" | "full" | "known" | "available" | "active" => query(data, &obs, input),
        _ => json!({ "ok": false, "error": "unknown-op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, Called, Js, Outcome, Pending, Reply, Started, Take};
    use client::io::ClientRevision;
    use std::thread;
    use std::time::Duration;

    /// The prayer family never calls a script callback.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(
            &mut self,
            _hook: Option<&crate::load::callback_v8::HeldCallback>,
            _args: &[Value],
        ) -> Called {
            panic!("the prayer family calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("the prayer family calls no script callback");
        }
    }

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    /// Stand in for a post carrying the prayer stat and one more overlay
    /// varp; earlier varp rows stay on the page.
    fn set_obs(points: i32, max: i32, varp: i32, value: i32) {
        let mut varps = observed::with(|scene| scene.latest().varps().cloned().unwrap_or_default());
        varps.retain(|row| row.index != varp);
        varps.push(observed::VarpRow { index: varp, value });
        observed::post(0, |post| {
            post.stats(observed::Skills {
                prayer: Some(observed::Skill {
                    effective: points,
                    base: max,
                    xp: 0,
                }),
                ..observed::Skills::default()
            })
            .varps(varps);
        });
    }

    fn prepare(rev: ClientRevision) {
        machine::on_reset();
        observed::on_reset();
        crate::supply_v2::configure(Some(data(rev)));
    }

    fn tick() {
        machine::step(&mut NoJs);
    }

    fn drain() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    fn start(args: Value) -> Started {
        machine::start("prayer", args, Vec::new(), 0)
    }

    fn running(started: Started) -> machine::Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    /// The settled envelope exactly as JS receives it.
    fn done(handle: machine::Handle) -> Value {
        match machine::take(handle) {
            Take::Settled(Outcome::Done(value)) => value,
            other => panic!("expected a done outcome, got {other:?}"),
        }
    }

    fn set(on: Value) -> Value {
        json!({ "op": "set", "name": "Protect from Melee", "on": on })
    }

    fn on(value: bool) -> Value {
        json!({ "kind": "boolean", "value": value })
    }

    #[test]
    fn matching_set_does_not_click_and_unknown_is_error() {
        prepare(ClientRevision::R274);
        set_obs(43, 43, 97, 1);
        assert_eq!(
            start(set(on(true))),
            Started::Settled(Outcome::Done(
                json!({ "ok": true, "reason": "matched", "value": true })
            ))
        );
        assert_eq!(
            start(json!({ "op": "set", "name": "Nope", "on": on(true) })),
            Started::Settled(Outcome::Done(
                json!({ "ok": false, "reason": "unknown-prayer" })
            ))
        );
        assert!(drain().is_empty(), "neither path clicks");
        assert!(!machine::live("prayer"));
    }

    #[test]
    fn unavailable_on_does_not_click_off_ignores_available() {
        prepare(ClientRevision::R289);
        set_obs(0, 1, 97, 0);
        assert_eq!(
            start(set(on(true))),
            Started::Settled(Outcome::Done(
                json!({ "ok": false, "reason": "unavailable" })
            ))
        );
        assert!(drain().is_empty());

        set_obs(0, 1, 97, 1);
        let handle = running(start(set(on(false))));
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5623 }]);
        set_obs(0, 1, 97, 0);
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": true, "reason": "toggled", "value": true })
        );
    }

    #[test]
    fn omitted_on_clicks_then_times_out_after_the_frozen_window() {
        prepare(ClientRevision::R274);
        set_obs(43, 43, 97, 0);
        let handle = running(start(set(json!({ "kind": "undefined" }))));
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5623 }]);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "unobserved varp waits"
        );
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": false, "reason": "toggle-timeout" })
        );
        assert_eq!(TOGGLE_MS, 2_000);
    }

    #[test]
    fn a_second_start_supersedes_the_first() {
        prepare(ClientRevision::R289);
        set_obs(43, 43, 97, 0);
        let first = running(start(set(on(true))));
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5623 }]);
        let second = running(start(set(on(true))));
        assert_eq!(
            machine::take(first),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Superseded)),
            "the newer set ends the older row"
        );
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton { component_id: 5623 }],
            "each admitted set clicks for itself"
        );
        set_obs(43, 43, 97, 1);
        tick();
        assert_eq!(
            done(second),
            json!({ "ok": true, "reason": "toggled", "value": true })
        );
        assert!(!machine::live("prayer"), "a settled row is not live");
    }

    #[test]
    fn a_v2_admit_refuses_busy_before_anything_is_emitted() {
        prepare(ClientRevision::R289);
        set_obs(43, 43, 97, 0);
        let admitted = running(start(set(on(true))));
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5623 }]);
        let mut refusing = set(on(false));
        refusing["admit"] = json!("refuse-busy");
        assert_eq!(
            start(refusing),
            Started::Refused("busy".into()),
            "the admitted row keeps the click"
        );
        assert!(drain().is_empty(), "a refused start emits nothing");
        set_obs(43, 43, 97, 1);
        tick();
        assert_eq!(
            done(admitted),
            json!({ "ok": true, "reason": "toggled", "value": true })
        );
        // The row is gone: the same start is admitted now.
        let mut next = set(on(false));
        next["admit"] = json!("refuse-busy");
        assert!(matches!(start(next), Started::Running(_)));
    }

    #[test]
    fn pause_and_hold_freeze_the_deadline_and_reset_settles_the_row() {
        prepare(ClientRevision::R289);
        set_obs(43, 43, 97, 0);
        let handle = running(start(set(on(true))));
        machine::on_pause();
        // The frozen clock reads the pause instant, so a row that stepped
        // here would already be past its 2s toggle window.
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "paused rows do not step"
        );
        machine::on_resume();
        machine::on_hold(true);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "held rows do not step"
        );
        // The pause and hold spans are reclaimed on the thaw: a row that
        // spent them would settle here instead of waiting its window out.
        machine::on_hold(false);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "the frozen span does not count toward the deadline"
        );
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": false, "reason": "toggle-timeout" }),
            "the window still runs from the thaw"
        );

        let _ = drain();
        let handle = running(start(set(on(true))));
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5623 }]);
        machine::on_reset();
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Reset))
        );
        tick();
        assert!(drain().is_empty(), "an aborted row never clicks again");
        assert!(!machine::live("prayer"));
    }

    #[test]
    fn clear_continues_after_timeout_with_counts() {
        prepare(ClientRevision::R274);
        set_obs(43, 43, 96, 1);
        set_obs(43, 43, 97, 1);
        let handle = running(start(json!({ "op": "clear" })));
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5622 }]);
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        tick();
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 5623 }]);
        set_obs(43, 43, 97, 0);
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": true, "reason": "cleared", "value": { "clicked": 2, "timed_out": 1 } }),
            "counts stay in the settled value"
        );
    }
}
