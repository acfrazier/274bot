//! One-shot **open-and-focus** for the panel's non-toggle window openers
//! (Profiles, General config, Nav config, Loadouts, Script prefs, Log,
//! Fleet, Debug, Browse, Load script, Import catalog, Fleet's WalkTo).
//!
//! Every click of such an opener calls [`Session::open_window`]: a closed
//! window opens with its usual first placement and docking, an open one is
//! raised and focused (its dock tab selected). The focus is requested once
//! and consumed by the target's next `Begin` ([`Session::take_window_focus`]),
//! never applied every frame. Toggles (WalkTo button, MultiBox, Grid) and
//! popups do not use it.

use super::Session;

/// A window that a non-toggle opener opens and focuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelWindow {
    Profiles,
    GeneralConfig,
    NavConfig,
    Loadouts,
    ScriptPrefs,
    Log,
    Fleet,
    Debug,
    Browse,
    LoadScript,
    ImportCatalog,
    WalkTo,
}

impl PanelWindow {
    pub const ALL: [PanelWindow; 12] = [
        PanelWindow::Profiles,
        PanelWindow::GeneralConfig,
        PanelWindow::NavConfig,
        PanelWindow::Loadouts,
        PanelWindow::ScriptPrefs,
        PanelWindow::Log,
        PanelWindow::Fleet,
        PanelWindow::Debug,
        PanelWindow::Browse,
        PanelWindow::LoadScript,
        PanelWindow::ImportCatalog,
        PanelWindow::WalkTo,
    ];

    /// The stable ImGui window name its `Begin` uses (the `###` part keeps
    /// Fleet's identity), so focus and docking find the same window.
    pub const fn title(self) -> &'static str {
        match self {
            PanelWindow::Profiles => "Profiles",
            PanelWindow::GeneralConfig => "General config",
            PanelWindow::NavConfig => "Nav config",
            PanelWindow::Loadouts => "Loadouts",
            PanelWindow::ScriptPrefs => "Script prefs",
            PanelWindow::Log => "Log",
            PanelWindow::Fleet => "Fleet###fleet-window",
            PanelWindow::Debug => "Debug",
            PanelWindow::Browse => crate::script_picker::BROWSE_WINDOW_TITLE,
            PanelWindow::LoadScript => "Load script",
            PanelWindow::ImportCatalog => "Import rs2b0t catalog",
            PanelWindow::WalkTo => "WalkTo",
        }
    }
}

impl Session {
    fn window_open_flag(&mut self, window: PanelWindow) -> &mut bool {
        match window {
            PanelWindow::Profiles => &mut self.wall.chooser_open,
            PanelWindow::GeneralConfig => &mut self.global_settings_open,
            PanelWindow::NavConfig => &mut self.nav_settings_open,
            PanelWindow::Loadouts => &mut self.loadouts_open,
            PanelWindow::ScriptPrefs => &mut self.script_prefs_open,
            PanelWindow::Log => &mut self.log_window_open,
            PanelWindow::Fleet => &mut self.fleet_open,
            PanelWindow::Debug => &mut self.debug_panel_open,
            PanelWindow::Browse => &mut self.script_browse_open,
            PanelWindow::LoadScript => &mut self.script_load_open,
            PanelWindow::ImportCatalog => &mut self.rs2b0t_catalog_open,
            PanelWindow::WalkTo => &mut self.walkto_open,
        }
    }

    /// Open `window` and ask for its focus once. Returns whether it was
    /// closed, so the caller initialises its dialog or draft only then: a
    /// click on an already-open window only brings it forward.
    pub fn open_window(&mut self, window: PanelWindow) -> bool {
        let opened = !std::mem::replace(self.window_open_flag(window), true);
        self.request_window_focus(window);
        opened
    }

    /// Bring `window` forward at its next `Begin` (once). A later request
    /// replaces an earlier one: only one window can hold the focus.
    pub fn request_window_focus(&mut self, window: PanelWindow) {
        self.window_focus = Some(window);
    }

    /// Consume a pending focus request for `window`; call immediately
    /// before its `Begin` and focus it when this returns true.
    pub fn take_window_focus(&mut self, window: PanelWindow) -> bool {
        if self.window_focus == Some(window) {
            self.window_focus = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_window_opens_once_and_requests_focus_on_every_call() {
        let mut s = Session::new();
        for window in PanelWindow::ALL {
            *s.window_open_flag(window) = false;
            assert!(s.open_window(window), "{window:?}: a closed window opens");
            assert!(*s.window_open_flag(window));
            assert!(s.take_window_focus(window), "{window:?}: focus requested");
            assert!(
                !s.take_window_focus(window),
                "{window:?}: focus is one-shot"
            );
            assert!(!s.open_window(window), "{window:?}: already open");
            assert!(*s.window_open_flag(window), "{window:?}: never toggles");
            assert!(
                s.take_window_focus(window),
                "{window:?}: open one refocuses"
            );
        }
    }

    #[test]
    fn a_focus_request_belongs_to_its_window_only() {
        let mut s = Session::new();
        s.open_window(PanelWindow::Log);
        assert!(!s.take_window_focus(PanelWindow::Profiles));
        s.open_window(PanelWindow::Fleet);
        assert!(
            !s.take_window_focus(PanelWindow::Log),
            "the later request wins"
        );
        assert!(s.take_window_focus(PanelWindow::Fleet));
    }
}
