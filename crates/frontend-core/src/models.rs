//! The operator models a front end holds, lent to the command seam for one
//! command (T3.3).
//!
//! The panel, the TUI and the headless driver each own their operator
//! models (the script coordinator, the marks, the profile form, the loadout
//! form, the map and preference models). Their changes are dispatch targets
//! of [`crate::OperatorSession::submit`] only: the front end lends the
//! models it has through [`crate::SlotSurface::models`], and the session
//! changes them while it dispatches a command. A front end reads them
//! through their own accessors; their mutators are crate-private.
//!
//! A model a front end does not hold stays `None`; a row that needs it is
//! then refused with [`crate::command::Rejection::NoModel`].

use crate::fleet::FleetFilter;
use crate::profile_editor::ProfileEditor;
use crate::profile_form::ProfileFormSave;
use crate::quester_paths::QuesterPathsController;
use crate::rs2b0t_import::ProfileImport;
use crate::scripts::{LoadoutForm, Scripts};
use crate::selection::MarkedSelection;
use crate::session::map::MapDesk;
use crate::session::prefs::DrawPolicy;
use crate::walk_permissions::WalkGlobalsView;

/// The models lent for one command. Build it with [`Models::none`] and the
/// `with_*` methods from the front end's own fields.
#[derive(Default)]
pub struct Models<'a> {
    pub(crate) scripts: Option<&'a mut Scripts>,
    pub(crate) marks: Option<&'a mut MarkedSelection>,
    pub(crate) fleet_filter: Option<&'a mut FleetFilter>,
    pub(crate) editor: Option<&'a mut ProfileEditor>,
    pub(crate) import: Option<&'a mut ProfileImport>,
    pub(crate) form_save: Option<&'a mut ProfileFormSave>,
    pub(crate) loadouts: Option<&'a mut LoadoutForm>,
    pub(crate) map: Option<&'a mut MapDesk>,
    pub(crate) quester: Option<&'a mut QuesterPathsController>,
    pub(crate) walk_globals: Option<&'a mut WalkGlobalsView>,
    pub(crate) draw: Option<&'a mut DrawPolicy>,
}

impl<'a> Models<'a> {
    /// No models (a surface that only owns slot devices).
    pub fn none() -> Self {
        Self::default()
    }

    /// The same models for a shorter borrow (a surface that keeps its
    /// bundle hands out one per command).
    pub fn reborrow(&mut self) -> Models<'_> {
        Models {
            scripts: self.scripts.as_deref_mut(),
            marks: self.marks.as_deref_mut(),
            fleet_filter: self.fleet_filter.as_deref_mut(),
            editor: self.editor.as_deref_mut(),
            import: self.import.as_deref_mut(),
            form_save: self.form_save.as_deref_mut(),
            loadouts: self.loadouts.as_deref_mut(),
            map: self.map.as_deref_mut(),
            quester: self.quester.as_deref_mut(),
            walk_globals: self.walk_globals.as_deref_mut(),
            draw: self.draw.as_deref_mut(),
        }
    }

    /// Lend the script coordinator.
    pub fn with_scripts(mut self, scripts: &'a mut Scripts) -> Self {
        self.scripts = Some(scripts);
        self
    }

    /// Lend the fleet marks.
    pub fn with_marks(mut self, marks: &'a mut MarkedSelection) -> Self {
        self.marks = Some(marks);
        self
    }

    /// Lend the fleet filter text and order.
    pub fn with_fleet_filter(mut self, filter: &'a mut FleetFilter) -> Self {
        self.fleet_filter = Some(filter);
        self
    }

    /// Lend the profile form.
    pub fn with_editor(mut self, editor: &'a mut ProfileEditor) -> Self {
        self.editor = Some(editor);
        self
    }

    /// Lend the rs2b0t profile import.
    pub fn with_import(mut self, import: &'a mut ProfileImport) -> Self {
        self.import = Some(import);
        self
    }

    /// Lend the TUI settings popup's save feedback.
    pub fn with_form_save(mut self, form_save: &'a mut ProfileFormSave) -> Self {
        self.form_save = Some(form_save);
        self
    }

    /// Lend the loadout form.
    pub fn with_loadouts(mut self, loadouts: &'a mut LoadoutForm) -> Self {
        self.loadouts = Some(loadouts);
        self
    }

    /// Lend the map desk (walk destination, map demand, bake gate).
    pub fn with_map(mut self, map: &'a mut MapDesk) -> Self {
        self.map = Some(map);
        self
    }

    /// Lend the Quester paths controller.
    pub fn with_quester(mut self, quester: &'a mut QuesterPathsController) -> Self {
        self.quester = Some(quester);
        self
    }

    /// Lend the walk-permission view.
    pub fn with_walk_globals(mut self, view: &'a mut WalkGlobalsView) -> Self {
        self.walk_globals = Some(view);
        self
    }

    /// Lend the draw policy.
    pub fn with_draw(mut self, draw: &'a mut DrawPolicy) -> Self {
        self.draw = Some(draw);
        self
    }
}
