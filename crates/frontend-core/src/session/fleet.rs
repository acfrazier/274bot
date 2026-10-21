//! Fleet-view rows of the command seam: the filter text and order, the
//! marks, the saved Load order and MultiBox.

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
    Reorder,
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
            Self::Reorder => OperatorCommand::ReorderProfiles,
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

    /// Stage the reorder the next [`OperatorCommand::ReorderProfiles`]
    /// applies to the saved Load order.
    pub fn stage_reorder(&mut self, op: crate::profile_order::ReorderOp) {
        self.commands.stage_reorder(op);
    }

    /// The name of the profile `identity` names now: its vault row, or a
    /// loaded slot without one.
    fn profile_name_of(&self, identity: ProfileIdentity) -> Option<String> {
        self.profiles()
            .find(|profile| ProfileIdentity::uid(profile.uid()) == identity)
            .map(|profile| profile.username().to_string())
            .or_else(|| {
                self.members()
                    .iter()
                    .find(|name| self.profile_identity_or_synthetic(name) == identity)
                    .cloned()
            })
    }

    /// Apply the staged reorder to the saved Load order (the lent fleet
    /// model must show Load order) and write it. Nothing moved: `Done`
    /// with no write and no audit line.
    fn reorder_profiles<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        surface: &mut S,
    ) -> Applied {
        let Some(op) = self.commands.take_reorder() else {
            return Applied::Rejected(Rejection::NotStaged, Some("reorder: nothing staged".into()));
        };
        let mut models = surface.models();
        let Some(fleet) = models.fleet.as_deref_mut() else {
            return no_model("fleet order");
        };
        if fleet.order() != FleetOrder::LoadOrder {
            return Applied::Rejected(
                Rejection::NotApplicable,
                Some(format!(
                    "reorder: the fleet is sorted by {}; choose Load order to reorder",
                    fleet.order().label()
                )),
            );
        }
        let before = self.profile_order.snapshot();
        let changed = match op {
            crate::profile_order::ReorderOp::Move { moving, to } => {
                let (target, after) = match to {
                    crate::profile_order::Placement::Before(target) => (target, false),
                    crate::profile_order::Placement::After(target) => (target, true),
                };
                let (Some(moving), Some(target)) =
                    (self.profile_name_of(moving), self.profile_name_of(target))
                else {
                    return Applied::Rejected(
                        Rejection::UnknownProfile,
                        Some("reorder: the profile no longer exists".into()),
                    );
                };
                self.profile_order.move_to(&moving, &target, after)
            }
            crate::profile_order::ReorderOp::MarkedToTop => {
                let marks = fleet.marks();
                let marked: Vec<String> = self
                    .profile_order
                    .names()
                    .iter()
                    .filter(|name| marks.contains(self.profile_identity_or_synthetic(name)))
                    .cloned()
                    .collect();
                self.profile_order
                    .move_to_top(|name| marked.iter().any(|m| m == name))
            }
        };
        if !changed {
            return Applied::Done;
        }
        self.order_saved = true;
        self.profiles_generation = self.profiles_generation.wrapping_add(1);
        self.order_reconciled_at = Some(self.profiles_generation);
        self.sort_members_by_order();
        self.write_profile_order(request, &before)
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
            FleetRow::Reorder => self.reorder_profiles(request, surface),
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
