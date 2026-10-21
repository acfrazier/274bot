//! The fleet plan flow: the one path every fleet action button and key
//! takes through the shared [`frontend_core`] commands.
//!
//! The core decides everything (which bots, which card, what is skipped,
//! whether to ask first, what the prompt says). This adapter only submits:
//! it plans the action, runs the plan at once when it needs no
//! confirmation, and otherwise leaves it waiting for the plan row's
//! Confirm or Cancel. The one result of the run is
//! [`frontend_core::OperatorSession::fleet_result`]; nothing here copies it.

use frontend_core::{FleetAction, OperatorCommand, Targets};

use crate::session::Session;

impl Session {
    /// Plan `action` over the fleet model's marks (or, with none, its loaded
    /// shown rows). A refusal shows its reason on the banner. A plan that
    /// needs no confirmation runs now; any other waits for
    /// [`Self::confirm_fleet_plan`] or [`Self::cancel_fleet_plan`]. A new
    /// plan replaces a waiting one.
    pub(crate) fn plan_fleet_action(&mut self, action: FleetAction) {
        let Some(planned) = self.submit(
            OperatorCommand::PlanFleetAction(action),
            Targets::Application,
        ) else {
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
    /// show on the result line, not on the banner.
    pub(crate) fn confirm_fleet_plan(&mut self) {
        let Some(plan) = self.core.fleet_plan() else {
            return;
        };
        let action = plan.action();
        let command = plan.command();
        if matches!(
            action,
            FleetAction::Start | FleetAction::Assign | FleetAction::AssignRestart
        ) {
            let root = self.start_catalog_root();
            self.scripts.stage_start_catalog_root(root);
        }
        self.submit(command, Targets::Plan);
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
        let Some(focused) = self.focused_name() else {
            self.error = Some("Copy settings: focus a bot first".into());
            return;
        };
        let Some(card) = self.scripts.heading(&self.core, &focused) else {
            self.error = Some("Copy settings: the focused bot has no card".into());
            return;
        };
        self.core.stage_plan_card(card);
        self.plan_fleet_action(FleetAction::CopySettings);
    }
}
