//! Rust-owned prayer toggle and overlay sweep, a [`crate::machine`]
//! family. JavaScript starts `set`/`clear` with the caller name and the
//! classified `on`, and awaits one envelope; Rust owns the click, the
//! observed-varp wait, the overlay order and the deadline. The frozen
//! `points|max|full|known|available|active` queries stay one native call
//! each over the same selected rows.

use crate::combat::prayer::{PrayerSweep, PrayerToggle, ToggleProgress};
use crate::machine::{self, Begin, Cx, Family, Step};
use crate::observed::{self, Scene};
use crate::shim::InteractReq;
use api::game_data::SelectedGameData;
use api::prayer::{
    active, available, lookup, matches_on, max, on_is_truthy, points, OnArg, PrayerObservation,
    TOGGLE_MS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Prayer points/max and overlay varps from the isolate scene. A logout
/// forgets the session: only pages posted since login count, and a missing
/// overlay varp stays unobserved.
fn prayer_observation(scene: &Scene) -> PrayerObservation {
    let session = scene.since_login();
    let mut obs = PrayerObservation::empty();
    if let Some(row) = session.stats().and_then(|skills| skills.prayer) {
        obs.points = row.effective;
        obs.max = row.base;
    }
    for row in session.varps().into_iter().flatten() {
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
pub(crate) struct Toggle(PrayerToggle);

impl Toggle {
    fn step(&self, obs: &PrayerObservation, cx: &mut Cx<'_>) -> Step<PrayerDone> {
        // A matching observation wins over the deadline: a slow page that
        // arrives late still settles the toggle.
        match self.0.progress(obs, cx.clock().bound_reached()) {
            ToggleProgress::Matched => Step::Done(PrayerDone::matched("toggled")),
            ToggleProgress::TimedOut => Step::Done(PrayerDone::failed("toggle-timeout")),
            ToggleProgress::Pending => Step::Wait,
        }
    }
}

/// `Prayer.clear`: click every active overlay in selected order.
pub(crate) struct Clear {
    sweep: PrayerSweep,
}

impl Clear {
    fn step(
        &mut self,
        data: Option<&SelectedGameData>,
        obs: &PrayerObservation,
        cx: &mut Cx<'_>,
    ) -> Step<PrayerDone> {
        let timed_out = if cx.clock().bound_reached() {
            self.sweep.pending_mask()
        } else {
            0
        };
        if self.sweep.observe(obs, timed_out) != 0 {
            // The one pending click settled or timed out; the next candidate
            // gets a fresh capacity-one wait window.
            cx.clock().deadline = None;
        }
        if self.sweep.pending_mask() != 0 {
            return Step::Wait;
        }
        if let Some(click) = data.and_then(|data| self.sweep.candidates(data, obs).next()) {
            cx.emit(InteractReq::IfButton {
                component_id: click.button_com,
            });
            cx.clock().arm(TOGGLE_MS);
            debug_assert!(self.sweep.accepted(click));
            Step::Wait
        } else {
            let report = self.sweep.report();
            Step::Done(PrayerDone::cleared(report.clicked, report.timed_out))
        }
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
        let obs = observed::with(|scene| prayer_observation(scene));
        match args.op {
            Op::Set => begin_set(data.as_deref(), &args, &obs, cx),
            Op::Clear => begin_clear(data.as_deref(), &obs, cx),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<PrayerDone> {
        let data = crate::supply_v2::selected_data();
        let obs = observed::with(|scene| prayer_observation(scene));
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
        return Begin::Done(PrayerDone::failed(crate::supply_v2::GAME_DATA_UNAVAILABLE));
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
    Begin::Run(Prayer::Toggle(Toggle(PrayerToggle::new(row.varp, want))))
}

fn begin_clear(
    data: Option<&SelectedGameData>,
    obs: &PrayerObservation,
    cx: &mut Cx<'_>,
) -> Begin<Prayer> {
    let mut clear = Clear {
        sweep: PrayerSweep::new(),
    };
    match clear.step(data, obs, cx) {
        Step::Wait => Begin::Run(Prayer::Clear(clear)),
        Step::Done(done) => Begin::Done(done),
        Step::Call(_) | Step::Fail(_) => unreachable!("prayer clear has no callbacks"),
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
    let obs = observed::with(|scene| prayer_observation(scene));
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

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    fn protect_identity() -> (i32, i32, i32) {
        let data = crate::supply_v2::selected_data().expect("selected data configured");
        let prayer = data
            .prayer_by_name("Protect from Melee")
            .expect("Protect from Melee row");
        (prayer.varp, prayer.level, prayer.button_com)
    }

    fn set_protect(value: i32) {
        let (varp, level, _) = protect_identity();
        set_obs(level, level, varp, value);
    }

    fn preceding_prayer_identity() -> (i32, i32) {
        let data = crate::supply_v2::selected_data().expect("selected data configured");
        let (protect_varp, _, _) = protect_identity();
        let rows = data.prayers();
        let protect_index = rows
            .iter()
            .position(|prayer| prayer.varp == protect_varp)
            .expect("Protect from Melee row index");
        let previous = rows
            .get(
                protect_index
                    .checked_sub(1)
                    .expect("Protect from Melee has a preceding prayer row"),
            )
            .expect("preceding selected prayer row");
        (previous.varp, previous.button_com)
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
        set_protect(1);
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
    fn missing_selected_data_has_an_explicit_reason() {
        machine::on_reset();
        observed::on_reset();
        crate::supply_v2::configure(None);
        assert_eq!(
            start(set(on(true))),
            Started::Settled(Outcome::Done(json!({
                "ok": false,
                "reason": "game data unavailable: this server's content isn't verified (see profile/engine settings)"
            })))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn unavailable_on_does_not_click_off_ignores_available() {
        prepare(ClientRevision::R289);
        set_obs(0, 1, protect_identity().0, 0);
        assert_eq!(
            start(set(on(true))),
            Started::Settled(Outcome::Done(
                json!({ "ok": false, "reason": "unavailable" })
            ))
        );
        assert!(drain().is_empty());

        set_obs(0, 1, protect_identity().0, 1);
        let handle = running(start(set(on(false))));
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton {
                component_id: protect_identity().2
            }]
        );
        set_obs(0, 1, protect_identity().0, 0);
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": true, "reason": "toggled", "value": true })
        );
    }

    #[test]
    fn omitted_on_clicks_then_times_out_after_the_frozen_window() {
        prepare(ClientRevision::R274);
        set_protect(0);
        let handle = running(start(set(json!({ "kind": "undefined" }))));
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton {
                component_id: protect_identity().2
            }]
        );
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
        set_protect(0);
        let first = running(start(set(on(true))));
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton {
                component_id: protect_identity().2
            }]
        );
        let second = running(start(set(on(true))));
        assert_eq!(
            machine::take(first),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Superseded)),
            "the newer set ends the older row"
        );
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton {
                component_id: protect_identity().2
            }],
            "each admitted set clicks for itself"
        );
        set_protect(1);
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
        set_protect(0);
        let admitted = running(start(set(on(true))));
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton {
                component_id: protect_identity().2
            }]
        );
        let mut refusing = set(on(false));
        refusing["admit"] = json!("refuse-busy");
        assert_eq!(
            start(refusing),
            Started::Refused("busy".into()),
            "the admitted row keeps the click"
        );
        assert!(drain().is_empty(), "a refused start emits nothing");
        set_protect(1);
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
        set_protect(0);
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
        assert_eq!(
            drain(),
            vec![InteractReq::IfButton {
                component_id: protect_identity().2
            }]
        );
        machine::on_reset();
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Reset))
        );
        tick();
        assert!(drain().is_empty(), "an aborted row never clicks again");
        assert!(!machine::live("prayer"));
    }

}
