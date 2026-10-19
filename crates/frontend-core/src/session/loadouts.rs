//! Loadout-form rows of the command seam.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{LoadoutField, OperatorCommand, Rejection, SubjectKind};
use crate::surface::SlotSurface;
use crate::OperatorSession;

/// One loadout row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadoutRow {
    Select,
    Edit(LoadoutField),
    CopyEquipment,
    Create,
    Duplicate,
    Save,
    Delete,
}

impl LoadoutRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Select => OperatorCommand::SelectLoadout,
            Self::Edit(field) => OperatorCommand::EditLoadoutDraft(field),
            Self::CopyEquipment => OperatorCommand::CopyEquipmentToLoadout,
            Self::Create => OperatorCommand::CreateLoadout,
            Self::Duplicate => OperatorCommand::DuplicateLoadout,
            Self::Save => OperatorCommand::SaveLoadout,
            Self::Delete => OperatorCommand::DeleteLoadout,
        }
    }
}

impl<Io> OperatorSession<Io> {
    pub(super) fn loadout_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: LoadoutRow,
        request: RequestId,
        _surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        let applied = || Applied::Rejected(Rejection::NoModel, Some("loadout rows: in progress".into()));
        if command.subject() == SubjectKind::Application {
            if self.app_admitted(request) {
                self.settle_app(request, command, applied(), submitted);
            }
        } else {
            for name in self.admitted_names(request) {
                self.settle_member(request, command, &name, applied(), submitted);
            }
        }
    }
}
