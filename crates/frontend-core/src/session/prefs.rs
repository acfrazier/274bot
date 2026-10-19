//! Preference and notice rows of the command seam.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{DrawSwitch, OperatorCommand, Rejection};
use crate::map_bake::MapBakeChoice;
use crate::nav_prefs::NavPreference;
use crate::surface::SlotSurface;
use crate::walk_permissions::DangerLevel;
use crate::OperatorSession;

/// One preference row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefRow {
    Draw(DrawSwitch, bool),
    Nav(NavPreference, bool),
    Danger(DangerLevel),
    SessionLog(bool),
    MapBake(MapBakeChoice),
    QuesterPaths,
    ReloadQuesterPaths,
    AcknowledgeNotice,
    SaveLog,
}

impl PrefRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Draw(switch, on) => OperatorCommand::SetDraw(switch, on),
            Self::Nav(pref, on) => OperatorCommand::SetNavPreference(pref, on),
            Self::Danger(level) => OperatorCommand::SetDangerLevel(level),
            Self::SessionLog(on) => OperatorCommand::SetSessionLog(on),
            Self::MapBake(choice) => OperatorCommand::SetMapBakeChoice(choice),
            Self::QuesterPaths => OperatorCommand::SetQuesterPaths,
            Self::ReloadQuesterPaths => OperatorCommand::ReloadQuesterPaths,
            Self::AcknowledgeNotice => OperatorCommand::AcknowledgeNotice,
            Self::SaveLog => OperatorCommand::SaveLog,
        }
    }
}

/// The panel's draw policy (placeholder until the prefs family lands).
#[derive(Debug, Default)]
pub struct DrawPolicy {}

impl<Io> OperatorSession<Io> {
    pub(super) fn pref_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: PrefRow,
        request: RequestId,
        admitted: bool,
        _surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let applied = if admitted {
            Applied::Rejected(Rejection::NoModel, Some("preference rows: in progress".into()))
        } else {
            Applied::NotAdmitted
        };
        self.settle_app(request, row.command(), applied, submitted);
    }
}
