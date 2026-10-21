//! The Fleet window's commands on the marked rows: the panel adapter over the
//! shared [`frontend_core`] commands. Each one reads the shared marked set
//! (the marks of [`Session::fleet_model`]) and puts its one bulk report in
//! [`Session::fleet_report`].

use frontend_core::{OperatorCommand, Targets};

use crate::session::Session;

impl Session {
    /// Save the selected card as the assignment of every marked bot without
    /// starting it, so each bot's settings can be adjusted first.
    pub fn fleet_assign_selected(&mut self) {
        let Some(card) = self
            .script_sel
            .as_ref()
            .map(frontend_core::scripts::ScriptChoice::duplicate)
        else {
            self.fleet_report = Some(frontend_core::ASSIGN_NO_CARD.into());
            return;
        };
        self.scripts.stage_browse_pick(card);
        if self
            .submit_script_command(OperatorCommand::PickBrowseCard, Targets::Application)
            .is_none()
        {
            self.fleet_report = self.error.clone();
            return;
        }
        let root = self.start_catalog_root();
        self.scripts.stage_start_catalog_root(root);
        let submitted = self.submit_script_command(OperatorCommand::AssignScript, Targets::Marked);
        self.apply_script_notice();
        self.fleet_report_follows_start = false;
        self.fleet_report =
            submitted.and_then(|submitted| submitted.bulk().map(|report| report.summary()));
    }

    /// Which marked bots an Assign & restart would interrupt, for the
    /// confirmation that names them.
    pub fn fleet_restart_scope(&self) -> frontend_core::RestartScope {
        frontend_core::restart_scope(self.fleet_model.marks(), &self.core)
    }

    /// Assign the selected card to every marked bot and start it there,
    /// stopping an active script first. The Start is paced like Start all and
    /// the fleet report follows the running Start report.
    pub fn fleet_assign_restart_selected(&mut self) {
        self.fleet_restart_confirm = false;
        let Some(card) = self
            .script_sel
            .as_ref()
            .map(frontend_core::scripts::ScriptChoice::duplicate)
        else {
            self.fleet_report = Some(frontend_core::ASSIGN_RESTART_NO_CARD.into());
            return;
        };
        self.scripts.stage_browse_pick(card);
        if self
            .submit_script_command(OperatorCommand::PickBrowseCard, Targets::Application)
            .is_none()
        {
            self.fleet_report = self.error.clone();
            return;
        }
        let root = self.start_catalog_root();
        self.scripts.stage_start_catalog_root(root);
        self.submit_script_command(OperatorCommand::AssignAndRestart, Targets::Marked);
        self.apply_script_notice();
        self.fleet_report_follows_start = true;
        self.follow_fleet_report();
    }

    /// Log in every marked bot, loading the ones not loaded yet.
    pub fn fleet_login_selected(&mut self) {
        let report = self.login_marked_rows();
        self.fleet_report_follows_start = false;
        self.fleet_report = report.or_else(|| self.error.clone());
    }

    /// Log out every marked bot that is logged in or logging in.
    pub fn fleet_logout_selected(&mut self) {
        let report = self.submit_marked(OperatorCommand::Logout);
        self.apply_script_notice();
        self.fleet_report_follows_start = false;
        self.fleet_report = report.or_else(|| self.error.clone());
    }

    /// Prepare copying the focused bot's parameters for the selected card to
    /// the marked bots on that card. Nothing is written until the operator
    /// applies the frozen scope the Fleet window then shows.
    pub fn fleet_prepare_apply_settings(&mut self) {
        self.error = None;
        let (Some(source), Some(card)) = (
            self.focused_name(),
            self.script_sel
                .as_ref()
                .map(frontend_core::scripts::ScriptChoice::duplicate),
        ) else {
            self.fleet_report = Some("Apply to marked: focus a bot and select a card".into());
            return;
        };
        let kind = if card.compiled_id().is_some() {
            frontend_core::command::CardKind::Native
        } else {
            frontend_core::command::CardKind::Script
        };
        self.scripts.stage_settings_sync_prepare(&source, &card);
        self.submit_script_command(
            OperatorCommand::ApplySettingsToMarked(kind),
            Targets::Marked,
        );
        self.apply_script_notice();
        self.fleet_report = self.error.clone();
    }
}
