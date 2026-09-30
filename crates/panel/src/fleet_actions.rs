//! The Fleet window's commands on the marked rows: the panel adapter over the
//! shared [`frontend_core`] commands. Each one reads the shared marked set
//! ([`Session::fleet_selection`]) and puts its one bulk report in
//! [`Session::fleet_report`].

use crate::session::Session;

impl Session {
    /// Save the selected card as the assignment of every marked bot without
    /// starting it, so each bot's settings can be adjusted first.
    pub fn fleet_assign_selected(&mut self) {
        let Some(card) = self.script_sel.clone() else {
            self.fleet_report = Some("Assign: select a card in Scripts first".into());
            return;
        };
        let root = self.start_catalog_root();
        let report = frontend_core::assign_marked(
            &self.fleet_selection,
            &mut self.core,
            &mut self.scripts,
            &card,
            root.as_deref(),
        );
        self.apply_script_notice();
        self.fleet_report_follows_start = false;
        self.fleet_report = Some(report.summary());
    }

    /// Which marked bots an Assign & restart would interrupt, for the
    /// confirmation that names them.
    pub fn fleet_restart_scope(&self) -> frontend_core::RestartScope {
        frontend_core::restart_scope(&self.fleet_selection, &self.core)
    }

    /// Assign the selected card to every marked bot and start it there,
    /// stopping an active script first. The Start is paced like Start all and
    /// the fleet report follows the running Start report.
    pub fn fleet_assign_restart_selected(&mut self) {
        self.fleet_restart_confirm = false;
        let Some(card) = self.script_sel.clone() else {
            self.fleet_report = Some("Assign & restart: select a card in Scripts first".into());
            return;
        };
        let root = self.start_catalog_root();
        frontend_core::assign_and_restart_marked(
            &self.fleet_selection,
            &mut self.core,
            &mut self.scripts,
            &card,
            root.as_deref(),
        );
        self.apply_script_notice();
        self.fleet_report_follows_start = true;
        self.follow_fleet_report();
    }

    /// Log in every marked bot, loading the ones not loaded yet.
    pub fn fleet_login_selected(&mut self) {
        let report = self.login_marked_rows();
        self.fleet_report_follows_start = false;
        self.fleet_report = Some(report.summary());
    }

    /// Log out every marked bot that is logged in or logging in.
    pub fn fleet_logout_selected(&mut self) {
        let report =
            frontend_core::logout_marked(&self.fleet_selection, &mut self.core, &mut self.scripts);
        self.apply_script_notice();
        self.fleet_report_follows_start = false;
        self.fleet_report = Some(report.summary());
    }

    /// Prepare copying the focused bot's parameters for the selected card to
    /// the marked bots on that card. Nothing is written until the operator
    /// applies the frozen scope the Fleet window then shows.
    pub fn fleet_prepare_apply_settings(&mut self) {
        self.error = None;
        let (Some(source), Some(card)) = (self.focused_name(), self.script_sel.clone()) else {
            self.fleet_report = Some("Apply to marked: focus a bot and select a card".into());
            return;
        };
        if let Err(error) = frontend_core::prepare_apply_settings_marked(
            &self.fleet_selection,
            &self.core,
            &mut self.scripts,
            &source,
            &card,
        ) {
            self.fleet_report = Some(error);
        }
        self.apply_script_notice();
    }
}
