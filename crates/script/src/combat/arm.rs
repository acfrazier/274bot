//! Shared, allocation-free autocast arm progression for native combat and the isolate family.

#[cfg(feature = "load")]
use crate::observed::Scene;
use crate::shim::InteractReq;
use api::game_data::AutocastControls;

pub(crate) const TAB_WAIT_MS: u64 = 2_000;
pub(crate) const STEP_MS: u64 = 3_000;
const COMBAT_TAB: i32 = 0;

/// The posted facts an autocast arm decides from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArmObservation {
    pub(crate) ingame: bool,
    pub(crate) active_side_tab: i32,
    pub(crate) combat_tab_root: i32,
    pub(crate) magic_varp_value: i32,
}

impl ArmObservation {
    #[cfg(feature = "load")]
    /// A logout forgets the session: only pages posted since login count.
    pub(crate) fn from_scene(scene: &Scene, magic_varp: i32) -> Self {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArmFailure {
    Aborted,
    StaffMissing,
    TabTimeout,
    ChooserTimeout,
    SelectTimeout,
    ToggleTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    PendingTab,
    WaitTab,
    PendingChoose,
    WaitPanel,
    PendingSelect,
    WaitSelected,
    PendingToggle,
    WaitArmed,
    Done(Result<(), ArmFailure>),
}

/// One autocast arm. `poll` stages an emission; `emitted` commits its phase
/// transition only after the caller admits that interaction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Arm {
    spell_com: i32,
    phase: Phase,
}

#[derive(Debug, PartialEq)]
pub(crate) enum ArmStep {
    Emit(InteractReq, u64),
    Wait,
    Done(Result<(), ArmFailure>),
}

impl Arm {
    pub(crate) fn new(spell_com: i32) -> Self {
        Self {
            spell_com,
            phase: Phase::Start,
        }
    }

    pub(crate) fn poll(
        &mut self,
        obs: ArmObservation,
        controls: &AutocastControls,
        timeout: bool,
    ) -> ArmStep {
        if let Phase::Done(result) = self.phase {
            return ArmStep::Done(result);
        }
        if !obs.ingame {
            return self.finish(Err(ArmFailure::Aborted));
        }

        match self.phase {
            Phase::Start => {
                if obs.combat_tab_root != controls.staff_tab_root {
                    return self.finish(Err(ArmFailure::StaffMissing));
                }
                if obs.active_side_tab != COMBAT_TAB {
                    self.emit(
                        Phase::PendingTab,
                        InteractReq::SideTab { tab: COMBAT_TAB },
                        TAB_WAIT_MS,
                    )
                } else {
                    self.choose(controls.choose_com)
                }
            }
            Phase::PendingTab => {
                ArmStep::Emit(InteractReq::SideTab { tab: COMBAT_TAB }, TAB_WAIT_MS)
            }
            Phase::WaitTab if obs.active_side_tab == COMBAT_TAB => self.choose(controls.choose_com),
            Phase::WaitTab if timeout => self.finish(Err(ArmFailure::TabTimeout)),
            Phase::WaitTab => ArmStep::Wait,
            Phase::PendingChoose => ArmStep::Emit(
                InteractReq::IfButton {
                    component_id: controls.choose_com,
                },
                STEP_MS,
            ),
            Phase::WaitPanel if obs.combat_tab_root == controls.spell_panel_root => self.emit(
                Phase::PendingSelect,
                InteractReq::IfButton {
                    component_id: self.spell_com,
                },
                STEP_MS,
            ),
            Phase::WaitPanel if timeout => self.finish(Err(ArmFailure::ChooserTimeout)),
            Phase::WaitPanel => ArmStep::Wait,
            Phase::PendingSelect => ArmStep::Emit(
                InteractReq::IfButton {
                    component_id: self.spell_com,
                },
                STEP_MS,
            ),
            Phase::WaitSelected if obs.magic_varp_value == controls.selected_value => self.emit(
                Phase::PendingToggle,
                InteractReq::IfButton {
                    component_id: controls.toggle_com,
                },
                STEP_MS,
            ),
            Phase::WaitSelected if timeout => self.finish(Err(ArmFailure::SelectTimeout)),
            Phase::WaitSelected => ArmStep::Wait,
            Phase::PendingToggle => ArmStep::Emit(
                InteractReq::IfButton {
                    component_id: controls.toggle_com,
                },
                STEP_MS,
            ),
            Phase::WaitArmed if obs.magic_varp_value == controls.armed_value => self.finish(Ok(())),
            Phase::WaitArmed if timeout => self.finish(Err(ArmFailure::ToggleTimeout)),
            Phase::WaitArmed => ArmStep::Wait,
            Phase::Done(_) => unreachable!("terminal phase returned above"),
        }
    }

    /// Commit the transition associated with the most recent `Emit`, after
    /// the caller admits it. Calling this in any other phase is a no-op.
    pub(crate) fn emitted(&mut self) {
        self.phase = match self.phase {
            Phase::PendingTab => Phase::WaitTab,
            Phase::PendingChoose => Phase::WaitPanel,
            Phase::PendingSelect => Phase::WaitSelected,
            Phase::PendingToggle => Phase::WaitArmed,
            phase => phase,
        };
    }

    fn choose(&mut self, component_id: i32) -> ArmStep {
        self.emit(
            Phase::PendingChoose,
            InteractReq::IfButton { component_id },
            STEP_MS,
        )
    }

    fn emit(&mut self, pending: Phase, request: InteractReq, wait_ms: u64) -> ArmStep {
        self.phase = pending;
        ArmStep::Emit(request, wait_ms)
    }

    fn finish(&mut self, result: Result<(), ArmFailure>) -> ArmStep {
        self.phase = Phase::Done(result);
        ArmStep::Done(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls() -> AutocastControls {
        AutocastControls {
            staff_tab_root: 328,
            spell_panel_root: 1829,
            choose_com: 353,
            toggle_com: 349,
            spell_grid_base: 1830,
            magic_varp: 108,
            selected_value: 2,
            armed_value: 3,
        }
    }

    fn obs(active_side_tab: i32, combat_tab_root: i32, magic_varp_value: i32) -> ArmObservation {
        ArmObservation {
            ingame: true,
            active_side_tab,
            combat_tab_root,
            magic_varp_value,
        }
    }

    fn press(component_id: i32) -> InteractReq {
        InteractReq::IfButton { component_id }
    }

    #[test]
    fn tab_open_is_optional_and_only_admitted_emits_advance() {
        let controls = controls();
        let mut arm = Arm::new(1830);
        let side_tab = ArmStep::Emit(InteractReq::SideTab { tab: COMBAT_TAB }, TAB_WAIT_MS);
        assert_eq!(arm.poll(obs(1, 328, 0), &controls, false), side_tab);
        assert_eq!(arm.poll(obs(1, 328, 0), &controls, false), side_tab);
        arm.emitted();
        assert_eq!(arm.poll(obs(1, 328, 0), &controls, false), ArmStep::Wait);
        assert_eq!(
            arm.poll(obs(0, 328, 0), &controls, false),
            ArmStep::Emit(press(controls.choose_com), STEP_MS)
        );

        let mut arm = Arm::new(1830);
        assert_eq!(
            arm.poll(obs(0, 328, 0), &controls, false),
            ArmStep::Emit(press(controls.choose_com), STEP_MS)
        );
    }

    #[test]
    fn an_initial_armed_varp_still_selects_the_spell_then_toggles() {
        let controls = controls();
        let mut arm = Arm::new(1907);
        let initial = obs(0, 328, controls.armed_value);

        assert_eq!(
            arm.poll(initial, &controls, false),
            ArmStep::Emit(press(controls.choose_com), STEP_MS)
        );
        arm.emitted();
        assert_eq!(arm.poll(initial, &controls, false), ArmStep::Wait);

        assert_eq!(
            arm.poll(
                obs(0, controls.spell_panel_root, controls.armed_value),
                &controls,
                false
            ),
            ArmStep::Emit(press(1907), STEP_MS)
        );
        arm.emitted();
        assert_eq!(
            arm.poll(
                obs(0, controls.spell_panel_root, controls.selected_value),
                &controls,
                false
            ),
            ArmStep::Emit(press(controls.toggle_com), STEP_MS)
        );
        arm.emitted();
        assert_eq!(
            arm.poll(obs(0, 328, controls.armed_value), &controls, false),
            ArmStep::Done(Ok(()))
        );
    }

    #[test]
    fn timeout_failure_tracks_the_waiting_action() {
        let controls = controls();

        let mut arm = Arm::new(1830);
        assert!(matches!(
            arm.poll(obs(1, 328, 0), &controls, false),
            ArmStep::Emit(InteractReq::SideTab { tab: 0 }, _)
        ));
        arm.emitted();
        assert_eq!(
            arm.poll(obs(1, 328, 0), &controls, true),
            ArmStep::Done(Err(ArmFailure::TabTimeout))
        );

        let mut arm = Arm::new(1830);
        arm.poll(obs(0, 328, 0), &controls, false);
        arm.emitted();
        assert_eq!(
            arm.poll(obs(0, 328, 0), &controls, true),
            ArmStep::Done(Err(ArmFailure::ChooserTimeout))
        );

        let mut arm = Arm::new(1830);
        arm.poll(obs(0, 328, 0), &controls, false);
        arm.emitted();
        arm.poll(obs(0, 1829, 0), &controls, false);
        arm.emitted();
        assert_eq!(
            arm.poll(obs(0, 1829, 0), &controls, true),
            ArmStep::Done(Err(ArmFailure::SelectTimeout))
        );

        let mut arm = Arm::new(1830);
        arm.poll(obs(0, 328, 0), &controls, false);
        arm.emitted();
        arm.poll(obs(0, 1829, 0), &controls, false);
        arm.emitted();
        arm.poll(obs(0, 1829, 2), &controls, false);
        arm.emitted();
        assert_eq!(
            arm.poll(obs(0, 1829, 2), &controls, true),
            ArmStep::Done(Err(ArmFailure::ToggleTimeout))
        );
    }

    #[test]
    fn staff_is_a_start_precondition_and_logout_aborts() {
        let controls = controls();
        let mut arm = Arm::new(1830);
        assert_eq!(
            arm.poll(obs(0, 192, 0), &controls, false),
            ArmStep::Done(Err(ArmFailure::StaffMissing))
        );

        let mut arm = Arm::new(1830);
        arm.poll(obs(0, 328, 0), &controls, false);
        arm.emitted();
        assert_eq!(arm.poll(obs(0, 192, 0), &controls, false), ArmStep::Wait);
        assert_eq!(
            arm.poll(
                ArmObservation {
                    ingame: false,
                    ..obs(0, 192, 0)
                },
                &controls,
                false
            ),
            ArmStep::Done(Err(ArmFailure::Aborted))
        );
    }
}
