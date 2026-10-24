//! The fleet plan flow: the one path every fleet action button and key
//! takes through the shared [`frontend_core`] commands.
//!
//! The core decides everything (which bots, which card, what is skipped,
//! whether to ask first, what the prompt says). This adapter only submits:
//! it plans the action, runs the plan at once when it needs no
//! confirmation, and otherwise leaves it waiting for the plan row's
//! Confirm or Cancel. The one result of the run is
//! [`frontend_core::OperatorSession::fleet_result`]; nothing here copies it.
//! A pick in the world picker takes the same path ([`Session::plan_world_switch`]).

use frontend_core::{ErrorNeed, FleetAction, OperatorCommand, Targets};

use crate::session::{PanelWindow, Session};

impl Session {
    /// Plan `action` over the fleet model's marks (or, with none, its loaded
    /// shown rows). A refusal shows its reason on the banner. A plan that
    /// needs no confirmation runs now; any other waits for
    /// [`Self::confirm_fleet_plan`] or [`Self::cancel_fleet_plan`]. A new
    /// plan replaces a waiting one.
    pub(crate) fn plan_fleet_action(&mut self, action: FleetAction) {
        if matches!(
            action,
            FleetAction::Start | FleetAction::Assign | FleetAction::AssignRestart
        ) {
            // The plan freezes the catalog root for the row that runs it.
            let root = self.start_catalog_root();
            self.scripts.stage_start_catalog_root(root);
        }
        self.submit_fleet_plan(OperatorCommand::PlanFleetAction(action));
    }

    /// Plan moving `target` (a world marker's bot; `None`: the marks, or
    /// the loaded shown rows when none is marked) to the public world
    /// `world`: stage the one profile, then plan `SwitchWorld` through the
    /// same flow as the fleet buttons. A Switch world plan always asks: its
    /// plan row with Confirm and Cancel shows in every fleet toolbar, and
    /// the Fleet window opens (or comes forward) to show it when no toolbar
    /// is on screen. A refusal ("no bot to act on (alice: queued for
    /// login)") shows its reason on the banner. Nothing runs before
    /// Confirm.
    pub(crate) fn plan_world_switch(&mut self, target: Option<&str>, world: u16) {
        if let Some(name) = target {
            self.core.stage_plan_target(name);
        }
        self.submit_fleet_plan(OperatorCommand::PlanFleetAction(FleetAction::SwitchWorld(
            world,
        )));
        if self.core.fleet_plan().is_some() && !self.fleet_toolbar_shown() {
            self.open_window(PanelWindow::Fleet);
        }
    }

    /// Whether a fleet toolbar is on screen: the Fleet window's, the rail's,
    /// or the strip's fleet section (drawn while the rail is hidden, unless
    /// the operator turned the section off).
    pub(crate) fn fleet_toolbar_shown(&self) -> bool {
        self.fleet_open
            || self.rail_shown()
            || crate::ui_state::panel_section_visible(&self.ui, "fleet")
    }

    /// Submit a `PlanFleetAction` row and run the plan at once when it
    /// needs no confirmation.
    fn submit_fleet_plan(&mut self, plan: OperatorCommand) {
        let Some(planned) = self.submit(plan, Targets::Application) else {
            return;
        };
        if let Some(reason) = self.core.command_failure(planned.request()) {
            self.error = Some(reason.to_owned());
            return;
        }
        if self
            .core
            .fleet_plan()
            .is_some_and(|plan| !plan.needs_confirm())
        {
            self.confirm_fleet_plan();
        }
    }

    /// Run the waiting plan (the plan row's Confirm). The members' results
    /// show on the result line, not on the banner. An Assign changes the
    /// focused bot's heading when it was a target, so the Script section
    /// re-reads it (a pending Browse pick stays).
    pub(crate) fn confirm_fleet_plan(&mut self) {
        let Some(plan) = self.core.fleet_plan() else {
            return;
        };
        let command = plan.command();
        let assigns = matches!(
            plan.action(),
            FleetAction::Assign | FleetAction::AssignRestart
        );
        self.submit(command, Targets::Plan);
        if assigns {
            if let Some(name) = self.focused_name() {
                self.restore_script_heading(&name);
            }
        }
        self.apply_script_notice();
    }

    /// Drop the waiting plan; nothing is written.
    pub(crate) fn cancel_fleet_plan(&mut self) {
        self.submit(OperatorCommand::CancelFleetPlan, Targets::Application);
    }

    /// The Scripts window's copy entry: copy the focused bot's settings for
    /// its own card to the bots the fleet buttons cover (the marks, or the
    /// loaded shown rows), through the same plan flow with the plan row's
    /// confirmation.
    pub(crate) fn copy_focused_settings_to_same_card(&mut self) {
        let command = OperatorCommand::PlanFleetAction(FleetAction::CopySettings);
        let Some(focused) = self.focused_name() else {
            self.refuse_command(
                command,
                &[],
                Some(ErrorNeed::FocusedBot),
                "Copy settings: focus a bot first",
            );
            return;
        };
        let Some(card) = self.scripts.heading(&self.core, &focused) else {
            self.refuse_command(
                command,
                &[&focused],
                Some(ErrorNeed::FocusedBotCard),
                "Copy settings: the focused bot has no card",
            );
            return;
        };
        self.core.stage_plan_card(card);
        self.submit_fleet_plan(command);
    }
}
