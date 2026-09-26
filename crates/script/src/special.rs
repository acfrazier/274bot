//! Special-attack arm, a [`crate::machine`] family. Rust owns the packed
//! bar identity, the confirm window and the armed gate; JavaScript starts
//! one arm and awaits the frozen boolean. The bar root and the armed varp
//! are read from the isolate scene, never echoed from a JS page.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, Scene};
use crate::shim::InteractReq;
use api::game_data::{SelectedGameData, SpecialControls};
use serde_json::{json, Value};

/// The frozen fallback when a cache posts no confirm window.
const ARM_CONFIRM_TICKS: i32 = 2;

/// The armed varp row and the value that means armed, read from the
/// isolate scene. `None` is the frozen unposted `Special.armed()` throw.
fn armed_observation(scene: &Scene, armed_varp: i32, armed_value: i32) -> Option<bool> {
    let session = scene.since_login();
    let rows = session.varps()?;
    rows.iter()
        .find(|row| row.index == armed_varp)
        .map(|row| row.value == armed_value)
}

/// One arm: the bar click went out at begin; each step reads the varp.
pub(crate) struct Special {
    armed_varp: i32,
    armed_value: i32,
    ticks_left: i32,
}

impl Family for Special {
    const NAME: &'static str = "special";
    /// One arm at a time: a newer start ends the older row `superseded`
    /// and its await settles false. The frozen surface has no guard.
    const EXCLUSIVE: bool = true;
    /// The frozen `arm()` takes no arguments.
    type Args = Value;
    type Output = bool;
    fn begin(_args: Value, cx: &mut Cx<'_>) -> Begin<Self> {
        let selected = crate::supply_v2::selected_data();
        let Some(data) = selected.as_deref() else {
            return Begin::Refuse(crate::supply_v2::GAME_DATA_UNAVAILABLE.into());
        };
        let Some(controls) = SelectedGameData::special_controls(data) else {
            return Begin::Refuse("missing special controls".into());
        };
        let armed = observed::with(|scene| {
            armed_observation(scene, controls.armed_varp, controls.armed_value)
        });
        // The frozen `Special.armed()` reads a posted varp: an unposted one
        // is a missing fact, not a false.
        let Some(armed) = armed else {
            return Begin::Refuse("missing special attack varp".into());
        };
        if armed {
            return Begin::Done(true);
        }
        let root = observed::with(|scene| scene.since_login().combat_tab_root());
        let bar = root.map_or(-1, |root| data.special_bar(root));
        if bar == -1 {
            return Begin::Done(false);
        }
        cx.emit(InteractReq::IfButton { component_id: bar });
        Begin::Run(Self {
            armed_varp: controls.armed_varp,
            armed_value: controls.armed_value,
            ticks_left: if controls.arm_confirm_ticks > 0 {
                controls.arm_confirm_ticks
            } else {
                ARM_CONFIRM_TICKS
            },
        })
    }

    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<bool> {
        let armed =
            observed::with(|scene| armed_observation(scene, self.armed_varp, self.armed_value));
        // An unposted varp mid-arm keeps the window: only the frozen
        // `arm_confirm_ticks` bound ends the arm.
        if armed == Some(true) {
            return Step::Done(true);
        }
        self.ticks_left -= 1;
        if self.ticks_left <= 0 {
            return Step::Done(false);
        }
        Step::Wait
    }
}

pub fn controls_json(data: Option<&SelectedGameData>) -> Value {
    match data.and_then(SelectedGameData::special_controls) {
        Some(controls) => json_controls(controls, true),
        None => json_controls(
            &SpecialControls {
                energy_varp: -1,
                armed_varp: -1,
                armed_value: 1,
                max_energy: 1000,
                arm_confirm_ticks: 2,
                bars: Vec::new(),
                weapons: Vec::new(),
            },
            false,
        ),
    }
}

fn json_controls(controls: &SpecialControls, available: bool) -> Value {
    json!({
        "available": available && controls.available(),
        "energy_varp": controls.energy_varp,
        "armed_varp": controls.armed_varp,
        "armed_value": controls.armed_value,
        "max_energy": controls.max_energy,
        "arm_confirm_ticks": controls.arm_confirm_ticks,
    })
}

fn cost_json(data: Option<&SelectedGameData>, weapon: &str) -> Value {
    match data.and_then(|selected| selected.special_cost(weapon)) {
        Some(cost) => json!(cost),
        None => Value::Null,
    }
}

/// The wielded weapon's spec bar, from the scene's posted side-tab root.
fn bar_json(data: Option<&SelectedGameData>) -> Value {
    json!(data
        .map(|selected| {
            selected.special_bar(observed::with(|scene| {
                scene.since_login().combat_tab_root().unwrap_or(-1)
            }))
        })
        .unwrap_or(-1))
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    match input
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("controls")
    {
        "cost" => cost_json(
            data,
            input.get("weapon").and_then(Value::as_str).unwrap_or(""),
        ),
        "bar" => bar_json(data),
        _ => controls_json(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, Called, Js, Outcome, Pending, Reply, Started, Take};
    use client::io::ClientRevision;

    /// The special family never calls a script callback.
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
            panic!("the special family calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("the special family calls no script callback");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    /// The posted side-tab root and the two special-attack varps.
    fn set_observation(combat_tab_root: i32, energy: i32, armed: i32) {
        observed::post(0, |post| {
            post.session(true)
                .combat_tab_root(combat_tab_root)
                .varps(vec![
                    observed::VarpRow {
                        index: 300,
                        value: energy,
                    },
                    observed::VarpRow {
                        index: 301,
                        value: armed,
                    },
                ]);
        });
    }

    fn prepare(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        machine::on_reset();
        observed::on_reset();
        let data = data(rev);
        crate::supply_v2::configure(Some(data.clone()));
        data
    }

    fn tick() {
        machine::step(&mut NoJs);
    }

    fn drain() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    fn start() -> Started {
        machine::start("special", json!({}), Vec::new(), 0)
    }

    fn running(started: Started) -> machine::Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    fn done(handle: machine::Handle) -> Value {
        match machine::take(handle) {
            Take::Settled(Outcome::Done(value)) => value,
            other => panic!("expected a done outcome, got {other:?}"),
        }
    }

    #[test]
    fn both_selected_caches_post_the_audited_special_controls() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            let controls = data.special_controls().expect("generated special controls");
            assert_eq!(controls.energy_varp, 300);
            assert_eq!(controls.armed_varp, 301);
            assert_eq!(controls.armed_value, 1);
            assert_eq!(controls.max_energy, 1000);
            assert_eq!(controls.arm_confirm_ticks, 2);
            assert!(controls.available());
            assert_eq!(controls.cost("Dragon dagger"), Some(250));
            assert_eq!(controls.cost("  dragon DAGGER(p) "), Some(250));
            assert_eq!(controls.cost("Magic shortbow"), Some(350));
            assert_eq!(controls.cost("Rune thrownaxe"), Some(100));
            assert_eq!(controls.cost("Dragon halberd"), Some(300));
            assert_eq!(controls.cost("Rune scimitar"), None);
            assert_eq!(controls.cost("Dragon battleaxe"), None);
            assert_eq!(controls.cost("Excalibur"), None);
            assert_eq!(controls.bar_for_root(425), 7462);
            assert_eq!(controls.bar_for_root(2423), 7587);
            assert_eq!(controls.bar_for_root(8460), 8481);
            assert_eq!(controls.bar_for_root(328), -1);
        }
    }

    #[test]
    fn missing_controls_do_not_invent_costs_or_bars() {
        machine::on_reset();
        observed::on_reset();
        crate::supply_v2::configure(None);
        assert_eq!(controls_json(None)["available"], false);
        assert_eq!(cost_json(None, "Dragon dagger"), Value::Null);
        assert_eq!(bar_json(None), json!(-1));

        // Controls exist, but nothing has posted the combat tab: no bar.
        let data = data(ClientRevision::R274);
        set_observation(-1, 1000, 0);
        assert_eq!(bar_json(Some(data.as_ref())), json!(-1));
    }

    #[test]
    fn already_armed_settles_true_without_a_click() {
        prepare(ClientRevision::R274);
        set_observation(2276, 1000, 1);
        assert_eq!(start(), Started::Settled(Outcome::Done(json!(true))));
        assert!(drain().is_empty());
    }

    #[test]
    fn a_tab_without_a_spec_bar_settles_false_without_a_click() {
        prepare(ClientRevision::R274);
        set_observation(328, 1000, 0);
        assert_eq!(start(), Started::Settled(Outcome::Done(json!(false))));
        assert!(drain().is_empty(), "the staff tab has no spec bar");
    }

    #[test]
    fn missing_varps_refuse_instead_of_faking_zero() {
        prepare(ClientRevision::R274);
        observed::post(0, |post| {
            post.session(true);
        });
        assert_eq!(
            start(),
            Started::Refused("missing special attack varp".into()),
            "an unposted sa_attack is not a proven zero"
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn the_bar_click_arms_within_the_frozen_window() {
        let selected = prepare(ClientRevision::R289);
        assert!(selected.special_controls().is_some());
        set_observation(2276, 1000, 0);
        let handle = running(start());
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 7562 }]);
        tick();
        assert_eq!(machine::take(handle), Take::Pending, "unarmed is not armed");

        set_observation(2276, 1000, 1);
        tick();
        assert_eq!(done(handle), json!(true));
        assert!(drain().is_empty());
    }

    #[test]
    fn the_window_ends_false_and_a_newer_arm_supersedes() {
        prepare(ClientRevision::R274);
        set_observation(2276, 1000, 0);
        let handle = running(start());
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 7562 }]);
        tick();
        tick();
        assert_eq!(done(handle), json!(false), "two ticks is the frozen window");

        let old = running(start());
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 7562 }]);
        let new = running(start());
        assert_eq!(
            machine::take(old),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Superseded))
        );
        assert_eq!(machine::take(new), Take::Pending);
    }

    #[test]
    fn an_unposted_varp_mid_arm_does_not_shorten_the_window() {
        prepare(ClientRevision::R274);
        set_observation(2276, 1000, 0);
        let handle = running(start());
        assert_eq!(drain(), vec![InteractReq::IfButton { component_id: 7562 }]);
        observed::post(0, |post| {
            post.session(true);
        });
        tick();
        assert_eq!(machine::take(handle), Take::Pending);
        set_observation(2276, 1000, 1);
        tick();
        assert_eq!(done(handle), json!(true));
    }
}
