//! Fleet-view rows of the command seam: the filter text and order, the
//! marks and MultiBox.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{FleetOrder, OperatorCommand, Rejection, SkipReason};
use crate::selection::ProfileIdentity;
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
    /// Stage the rows the next [`OperatorCommand::MarkAll`] marks (the
    /// rows the front end shows, or the eligible Walk-send rows).
    pub fn stage_mark_all(&mut self, identities: impl IntoIterator<Item = ProfileIdentity>) {
        let staged = self.commands.mark_all_staging();
        staged.clear();
        staged.extend(identities);
    }

    pub(super) fn fleet_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: FleetRow,
        request: RequestId,
        admitted: bool,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        if !admitted && command.subject() == crate::command::SubjectKind::Application {
            self.settle_app(request, command, Applied::NotAdmitted, submitted);
            return;
        }
        let applied = match row {
            FleetRow::Mark(on) => {
                let names = self.admitted_names(request);
                for name in &names {
                    let identity = self.profile_identity_or_synthetic(name);
                    let applied = match surface.models().fleet.as_deref_mut() {
                        Some(fleet) => {
                            fleet.set_mark(identity, on);
                            Applied::Done
                        }
                        None => no_model("marks"),
                    };
                    self.settle_member(request, command, name, applied, submitted);
                }
                self.release_names(names);
                return;
            }
            FleetRow::Filter => match surface.models().fleet.as_deref_mut() {
                Some(fleet) => {
                    fleet.commit_filter();
                    Applied::Done
                }
                None => no_model("fleet filter"),
            },
            FleetRow::Sort(order) => match surface.models().fleet.as_deref_mut() {
                Some(fleet) => {
                    fleet.set_order(order);
                    Applied::Done
                }
                None => no_model("fleet order"),
            },
            FleetRow::MarkAll => {
                let staged = std::mem::take(self.commands.mark_all_staging());
                let applied = match surface.models().fleet.as_deref_mut() {
                    Some(fleet) => {
                        fleet.mark_all(staged.iter().copied());
                        Applied::Done
                    }
                    None => no_model("marks"),
                };
                *self.commands.mark_all_staging() = staged;
                applied
            }
            FleetRow::ClearMarks => match surface.models().fleet.as_deref_mut() {
                Some(fleet) => {
                    fleet.clear_marks();
                    Applied::Done
                }
                None => no_model("marks"),
            },
            FleetRow::MultiBox(true) => {
                self.seed_running();
                Applied::Done
            }
            FleetRow::MultiBox(false) => self.multibox_off(surface),
        };
        self.settle_app(request, command, applied, submitted);
    }

    /// MultiBox off closes Profiles and its form: while the lent form's
    /// save has not settled, the Discard / Keep editing decision is staged
    /// instead and MultiBox stays on.
    fn multibox_off<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) -> Applied {
        let mut models = surface.models();
        match models.editor.as_deref_mut() {
            Some(editor) if editor.saving(self) => {
                editor.stage_leave(self, crate::FormLeave::MultiBoxOff, "", "turn MultiBox off");
                Applied::Skipped(SkipReason::DecisionStaged)
            }
            _ => Applied::Done,
        }
    }
}

fn no_model(what: &str) -> Applied {
    Applied::Rejected(
        Rejection::NoModel,
        Some(format!("{what}: this front end holds none")),
    )
}
