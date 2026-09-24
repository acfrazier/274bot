//! Selected-world autocast arm, a [`crate::machine`] family. Rust owns the
//! chosen controls, the four-press order, the varp waits, the deadlines and
//! the log line for every way the arm can end; JavaScript starts one arm
//! with the caller's spell name and maps the settled outcome to the frozen
//! boolean (or the frozen `not impl` throw).

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, Scene};
use crate::shim::InteractReq;
use api::game_data::{AutocastControls, SelectedGameData};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The frozen side-tab index and windows.
const COMBAT_TAB: i32 = 0;
const TAB_WAIT_MS: u64 = 2_000;
const STEP_MS: u64 = 3_000;

/// The posted facts one arm decides from, read from the isolate scene.
#[derive(Clone, Copy)]
struct ArmObservation {
    ingame: bool,
    active_side_tab: i32,
    combat_tab_root: i32,
    magic_varp_value: i32,
}

impl ArmObservation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene, magic_varp: i32) -> Self {
        let session = scene.since_login();
        Self {
            ingame: session.ingame().unwrap_or(false),
            active_side_tab: session.side_tab().unwrap_or(-1),
            combat_tab_root: session.combat_tab_root().unwrap_or(-1),
            magic_varp_value: session.varps().map_or(0, |rows| {
                rows.iter()
                    .find(|row| row.index == magic_varp)
                    .map_or(0, |row| row.value)
            }),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ArmArgs {
    spell: String,
}

/// Every way a frozen `Autocast.arm` ends, and the log line it carries.
/// The reason text is Rust's; the shim only prints what it is handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Armed,
    UnknownSpell,
    MissingControls,
    MissingFacts,
    StaffMissing,
    OpenTab,
    Chooser,
    Select,
    Toggle,
    Aborted,
}

impl Reason {
    /// The frozen log line, or `None` where the frozen table logs nothing.
    fn log(self, spell: &str) -> Option<String> {
        match self {
            Self::Armed => Some(format!("autocast armed: {spell}")),
            Self::UnknownSpell => Some(format!(
                "'{spell}' is not an autocastable spell — see SPELL_DB (Wind Strike … Fire Wave)"
            )),
            Self::MissingControls => Some(format!(
                "not impl: Autocast.arm needs posted coms for '{spell}'"
            )),
            Self::StaffMissing => {
                Some("combat tab is not the staff layout — is a staff wielded?".into())
            }
            Self::OpenTab => Some("could not open the combat tab".into()),
            Self::Chooser => Some("spell chooser did not open".into()),
            Self::Select => Some(format!(
                "choosing '{spell}' did not take — magic level too low?"
            )),
            Self::Toggle => Some("autocast toggle did not arm".into()),
            Self::MissingFacts | Self::Aborted => None,
        }
    }

    /// The `not impl` reason that makes `arm` throw instead of returning.
    fn not_impl(self, spell: &str) -> Option<String> {
        match self {
            Self::MissingControls => Some(format!("Autocast.arm needs posted coms for '{spell}'")),
            _ => None,
        }
    }
}

/// What the shim turns into the declared boolean, the log line and the
/// frozen `not impl` throw.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct ArmOutcome {
    ok: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    not_impl: Option<String>,
}

impl ArmOutcome {
    fn of(reason: Reason, spell: &str) -> Self {
        Self {
            ok: reason == Reason::Armed,
            message: reason.log(spell).unwrap_or_default(),
            not_impl: reason.not_impl(spell),
        }
    }
}

impl From<ArmOutcome> for Value {
    fn from(outcome: ArmOutcome) -> Self {
        serde_json::to_value(outcome).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitTab,
    WaitPanel,
    WaitSelected,
    WaitArmed,
}

impl Phase {
    /// The frozen reason for a press that never showed its effect.
    fn timeout(self) -> Reason {
        match self {
            Self::WaitTab => Reason::OpenTab,
            Self::WaitPanel => Reason::Chooser,
            Self::WaitSelected => Reason::Select,
            Self::WaitArmed => Reason::Toggle,
        }
    }
}

/// One arm: the caller's spell, its grid component and the live phase.
pub(crate) struct Autocast {
    spell: String,
    spell_com: i32,
    phase: Phase,
}

impl Autocast {
    fn press(&mut self, phase: Phase, component_id: i32, cx: &mut Cx<'_>) -> Step<ArmOutcome> {
        self.phase = phase;
        cx.clock().arm(STEP_MS);
        cx.emit(InteractReq::IfButton { component_id });
        Step::Wait
    }
}

impl Family for Autocast {
    const NAME: &'static str = "autocast";
    /// A newer arm replaces the one in flight, as the frozen token bump did.
    const EXCLUSIVE: bool = true;
    type Args = ArmArgs;
    type Output = ArmOutcome;

    fn begin(args: ArmArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let data = crate::supply_v2::selected_data();
        let Some(controls) = data
            .as_deref()
            .and_then(SelectedGameData::autocast_controls)
        else {
            return Begin::Done(ArmOutcome::of(Reason::MissingControls, &args.spell));
        };
        let spell_com = data
            .as_deref()
            .map_or(-1, |data| data.spell_button_com(&args.spell));
        if spell_com < 0 {
            return Begin::Done(ArmOutcome::of(Reason::UnknownSpell, &args.spell));
        }
        let obs = observed::with(|scene| ArmObservation::from_scene(scene, controls.magic_varp));
        if !obs.ingame {
            return Begin::Done(ArmOutcome::of(Reason::MissingFacts, &args.spell));
        }
        if obs.combat_tab_root != controls.staff_tab_root {
            return Begin::Done(ArmOutcome::of(Reason::StaffMissing, &args.spell));
        }
        let mut arm = Self {
            spell: args.spell,
            spell_com,
            phase: Phase::WaitPanel,
        };
        if obs.active_side_tab != COMBAT_TAB {
            arm.phase = Phase::WaitTab;
            cx.clock().arm(TAB_WAIT_MS);
            cx.emit(InteractReq::SideTab { tab: COMBAT_TAB });
            return Begin::Run(arm);
        }
        arm.press(Phase::WaitPanel, controls.choose_com, cx);
        Begin::Run(arm)
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<ArmOutcome> {
        let data = crate::supply_v2::selected_data();
        let Some(controls) = data
            .as_deref()
            .and_then(SelectedGameData::autocast_controls)
        else {
            return Step::Done(ArmOutcome::of(Reason::MissingControls, &self.spell));
        };
        let obs = observed::with(|scene| ArmObservation::from_scene(scene, controls.magic_varp));
        if !obs.ingame {
            return Step::Done(ArmOutcome::of(Reason::Aborted, &self.spell));
        }
        match self.phase {
            Phase::WaitTab if obs.active_side_tab == COMBAT_TAB => {
                self.press(Phase::WaitPanel, controls.choose_com, cx)
            }
            Phase::WaitPanel if obs.combat_tab_root == controls.spell_panel_root => {
                self.press(Phase::WaitSelected, self.spell_com, cx)
            }
            Phase::WaitSelected if obs.magic_varp_value == controls.selected_value => {
                self.press(Phase::WaitArmed, controls.toggle_com, cx)
            }
            Phase::WaitArmed if obs.magic_varp_value == controls.armed_value => {
                Step::Done(ArmOutcome::of(Reason::Armed, &self.spell))
            }
            phase if cx.clock().bound_reached() => {
                Step::Done(ArmOutcome::of(phase.timeout(), &self.spell))
            }
            _ => Step::Wait,
        }
    }
}

pub fn controls_json(data: Option<&SelectedGameData>) -> Value {
    match data.and_then(SelectedGameData::autocast_controls) {
        Some(controls) => json_controls(controls, true),
        None => json_controls(
            &AutocastControls {
                staff_tab_root: -1,
                spell_panel_root: -1,
                choose_com: -1,
                toggle_com: -1,
                spell_grid_base: -1,
                magic_varp: -1,
                selected_value: 2,
                armed_value: 3,
            },
            false,
        ),
    }
}

fn json_controls(controls: &AutocastControls, available: bool) -> Value {
    json!({
        "available": available && controls.available(),
        "staff_tab_root": controls.staff_tab_root,
        "spell_panel_root": controls.spell_panel_root,
        "choose_com": controls.choose_com,
        "toggle_com": controls.toggle_com,
        "spell_grid_base": controls.spell_grid_base,
        "magic_varp": controls.magic_varp,
        "selected_value": controls.selected_value,
        "armed_value": controls.armed_value,
    })
}

pub fn observe(data: Option<&SelectedGameData>, combat_tab_root: i32, magic_varp: i32) -> Value {
    let mut out = controls_json(data);
    let staff = out
        .get("staff_tab_root")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let panel = out
        .get("spell_panel_root")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let selected = out
        .get("selected_value")
        .and_then(Value::as_i64)
        .unwrap_or(2);
    let armed = out.get("armed_value").and_then(Value::as_i64).unwrap_or(3);
    let available = out
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    out["staff_attached"] = json!(available && staff >= 0 && i64::from(combat_tab_root) == staff);
    out["panel_open"] = json!(available && panel >= 0 && i64::from(combat_tab_root) == panel);
    out["selected"] = json!(available && i64::from(magic_varp) == selected);
    out["armed"] = json!(available && i64::from(magic_varp) == armed);
    out["combat_tab_root"] = json!(combat_tab_root);
    out["magic_varp_value"] = json!(magic_varp);
    out
}

fn arm_observation(data: Option<&SelectedGameData>) -> ArmObservation {
    let magic_varp = data
        .and_then(SelectedGameData::autocast_controls)
        .map_or(-1, |controls| controls.magic_varp);
    observed::with(|scene| ArmObservation::from_scene(scene, magic_varp))
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    let obs = arm_observation(data);
    match input
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("controls")
    {
        "observe" => observe(data, obs.combat_tab_root, obs.magic_varp_value),
        _ => controls_json(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, Called, Js, Outcome, Pending, Reply, Started, Take};
    use client::io::ClientRevision;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// The arm family never calls a script callback.
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
            panic!("the autocast family calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("the autocast family calls no script callback");
        }
    }

    fn data(rev: ClientRevision) -> Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    fn prepare(rev: ClientRevision) -> Arc<SelectedGameData> {
        machine::on_reset();
        observed::on_reset();
        let data = data(rev);
        crate::supply_v2::configure(Some(data.clone()));
        data
    }

    /// The posted side tab and magic varp the arm decides from.
    fn set_observation(active_side_tab: i32, combat_tab_root: i32, magic_varp_value: i32) {
        let magic_varp = 108;
        observed::post(0, |post| {
            post.session(true)
                .side_tab(active_side_tab)
                .combat_tab_root(combat_tab_root)
                .varps(vec![crate::observed::VarpRow {
                    index: magic_varp,
                    value: magic_varp_value,
                }]);
        });
    }

    fn tick() {
        machine::step(&mut NoJs);
    }

    fn drain() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    fn start(spell: &str) -> Started {
        machine::start("autocast", json!({ "spell": spell }), Vec::new(), 0)
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

    fn press(id: i32) -> InteractReq {
        InteractReq::IfButton { component_id: id }
    }

    #[test]
    fn both_selected_caches_post_the_audited_staff_spell_controls() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = data(rev);
            let controls = data
                .autocast_controls()
                .expect("generated autocast controls");
            assert_eq!(controls.staff_tab_root, 328);
            assert_eq!(controls.choose_com, 353);
            assert_eq!(controls.spell_panel_root, 1829);
            assert_eq!(controls.spell_grid_base, 1830);
            assert_eq!(controls.toggle_com, 349);
            assert_eq!(controls.magic_varp, 108);
            assert_eq!(controls.selected_value, 2);
            assert_eq!(controls.armed_value, 3);
            assert!(controls.available());
            assert_eq!(data.spell_button_com("Wind Strike"), 1830);
        }
    }

    #[test]
    fn observe_tracks_staff_panel_and_armed_without_faking_missing_controls() {
        let missing = observe(None, 328, 3);
        assert_eq!(missing["available"], false);
        assert_eq!(missing["choose_com"], -1);
        assert_eq!(missing["staff_attached"], false);
        assert_eq!(missing["armed"], false);

        let data = data(ClientRevision::R274);
        let staff = observe(Some(data.as_ref()), 328, 0);
        assert_eq!(staff["available"], true);
        assert_eq!(staff["staff_attached"], true);
        assert_eq!(staff["panel_open"], false);
        assert_eq!(staff["armed"], false);
        let panel = observe(Some(data.as_ref()), 1829, 2);
        assert_eq!(panel["panel_open"], true);
        assert_eq!(panel["selected"], true);
        assert_eq!(panel["armed"], false);
        let armed = observe(Some(data.as_ref()), 328, 3);
        assert_eq!(armed["armed"], true);
        assert_eq!(armed["staff_attached"], true);
    }

    #[test]
    fn missing_controls_refuse_with_their_own_not_impl_reason() {
        machine::on_reset();
        observed::on_reset();
        crate::supply_v2::configure(None);
        assert_eq!(
            start("Wind Strike"),
            Started::Settled(Outcome::Done(json!({
                "ok": false,
                "message": "not impl: Autocast.arm needs posted coms for 'Wind Strike'",
                "not_impl": "Autocast.arm needs posted coms for 'Wind Strike'",
            })))
        );
        assert!(drain().is_empty(), "a refused arm sends nothing");
    }

    #[test]
    fn an_unknown_spell_logs_the_frozen_line_and_presses_nothing() {
        prepare(ClientRevision::R274);
        set_observation(0, 328, 0);
        assert_eq!(
            start("Not a spell"),
            Started::Settled(Outcome::Done(json!({
                "ok": false,
                "message": "'Not a spell' is not an autocastable spell — see SPELL_DB (Wind Strike … Fire Wave)",
            })))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn missing_facts_and_a_foreign_tab_refuse_without_pressing() {
        prepare(ClientRevision::R274);
        machine::on_reset();
        observed::on_reset();
        assert_eq!(
            start("Wind Strike"),
            Started::Settled(Outcome::Done(json!({ "ok": false, "message": "" }))),
            "no posted facts is not a click"
        );
        assert!(drain().is_empty());

        set_observation(0, 192, 0);
        assert_eq!(
            start("Wind Strike"),
            Started::Settled(Outcome::Done(json!({
                "ok": false,
                "message": "combat tab is not the staff layout — is a staff wielded?",
            })))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn the_four_presses_own_their_order_and_each_deadline() {
        let selected = prepare(ClientRevision::R274);
        assert_eq!(selected.spell_button_com("Wind Strike"), 1830);
        set_observation(1, 328, 0);
        let handle = running(start("Wind Strike"));
        assert_eq!(drain(), vec![InteractReq::SideTab { tab: COMBAT_TAB }]);

        set_observation(0, 328, 0);
        tick();
        assert_eq!(drain(), vec![press(353)], "choose is the first press");

        set_observation(0, 1829, 0);
        tick();
        assert_eq!(drain(), vec![press(1830)]);

        set_observation(0, 1829, 2);
        tick();
        assert_eq!(drain(), vec![press(349)]);

        set_observation(0, 328, 3);
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": true, "message": "autocast armed: Wind Strike" })
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn a_press_that_never_lands_times_out_with_its_own_reason() {
        prepare(ClientRevision::R289);
        set_observation(0, 328, 0);
        let handle = running(start("Wind Strike"));
        assert_eq!(drain(), vec![press(353)]);
        thread::sleep(Duration::from_millis(STEP_MS + 50));
        tick();
        assert_eq!(
            done(handle),
            json!({ "ok": false, "message": "spell chooser did not open" })
        );
    }

    #[test]
    fn a_newer_arm_supersedes_the_one_in_flight() {
        prepare(ClientRevision::R274);
        set_observation(0, 328, 0);
        let old = running(start("Wind Strike"));
        assert_eq!(drain(), vec![press(353)]);
        let new = running(start("Wind Bolt"));
        assert_eq!(
            machine::take(old),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Superseded))
        );
        assert_eq!(drain(), vec![press(353)], "the newer arm presses choose");
        tick();
        assert_eq!(machine::take(new), Take::Pending);
    }
}
