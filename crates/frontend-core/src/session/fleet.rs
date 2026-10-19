//! Fleet-view rows of the command seam: the filter text and order, the
//! marks and MultiBox.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{FleetOrder, OperatorCommand, Rejection};
use crate::surface::SlotSurface;
use crate::OperatorSession;

/// One fleet-view row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FleetRow {
    Filter,
    Sort(FleetOrder),
    Mark(bool),
    MarkAll,
    ClearMarks,
    MultiBox(bool),
}

impl FleetRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Filter => OperatorCommand::SetFleetFilter,
            Self::Sort(order) => OperatorCommand::SetFleetSort(order),
            Self::Mark(on) => OperatorCommand::SetMark(on),
            Self::MarkAll => OperatorCommand::MarkAll,
            Self::ClearMarks => OperatorCommand::ClearMarks,
            Self::MultiBox(on) => OperatorCommand::SetMultiBox(on),
        }
    }
}

impl<Io> OperatorSession<Io> {
    pub(super) fn fleet_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: FleetRow,
        request: RequestId,
        admitted: bool,
        _surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        let applied = || Applied::Rejected(Rejection::NoModel, Some("fleet rows: in progress".into()));
        if matches!(row, FleetRow::Mark(_)) {
            for name in self.admitted_names(request) {
                self.settle_member(request, command, &name, applied(), submitted);
            }
        } else if admitted {
            self.settle_app(request, command, applied(), submitted);
        }
    }
}
