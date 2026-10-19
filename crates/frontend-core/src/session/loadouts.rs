//! Loadout-form rows of the command seam.
//!
//! Every row acts on the [`LoadoutForm`] the front end lends
//! ([`crate::Models::with_loadouts`]) and settles its one member at
//! dispatch from what the form reports: `Done`, a typed `Skipped` (no
//! loadout selected, no character in game), a `Rejected` input (no draft, a
//! draft that cannot be saved) or `Failed(WriteFailed)` when a write was
//! rolled back. A front end that lends no form gets `Rejected(NoModel)`.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{FailureKind, LoadoutField, OperatorCommand, Rejection, SkipReason};
use crate::scripts::{LoadoutForm, Refused};
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
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        match row {
            LoadoutRow::Select => {
                self.settle_on_form(request, command, surface, submitted, LoadoutForm::select);
            }
            LoadoutRow::Edit(_) => {
                self.settle_on_form(request, command, surface, submitted, LoadoutForm::edited);
            }
            LoadoutRow::CopyEquipment => {
                for name in self.admitted_names(request) {
                    let applied = self.copy_equipment_into_draft(&name, surface);
                    self.settle_member(request, command, &name, applied, submitted);
                }
            }
            LoadoutRow::Create => {
                self.settle_on_form(request, command, surface, submitted, LoadoutForm::create);
            }
            LoadoutRow::Duplicate => {
                self.settle_on_form(request, command, surface, submitted, LoadoutForm::duplicate);
            }
            LoadoutRow::Save => {
                self.settle_on_form(
                    request,
                    command,
                    surface,
                    submitted,
                    LoadoutForm::save_draft,
                );
            }
            LoadoutRow::Delete => {
                self.settle_on_form(request, command, surface, submitted, LoadoutForm::delete);
            }
        }
    }

    /// Run `act` on the lent form and settle the application member with
    /// what it reports.
    fn settle_on_form<S: SlotSurface<Io = Io>>(
        &mut self,
        request: RequestId,
        command: OperatorCommand,
        surface: &mut S,
        submitted: &mut Submitted,
        act: fn(&mut LoadoutForm) -> Result<(), Refused>,
    ) {
        if !self.app_admitted(request) {
            return;
        }
        let mut models = surface.models();
        let applied = match models.loadouts.as_deref_mut() {
            Some(form) => form_applied(act(form), form, command),
            None => no_model(),
        };
        self.settle_app(request, command, applied, submitted);
    }

    /// `CopyEquipmentToLoadout` for member `name`: its worn gear becomes the
    /// draft's, the draft's supplies kept.
    fn copy_equipment_into_draft<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> Applied {
        let mut models = surface.models();
        let Some(form) = models.loadouts.as_deref_mut() else {
            return no_model();
        };
        let Some(items) = self.observed_equipment(name) else {
            form.note_no_equipment();
            return Applied::Skipped(SkipReason::NotLoggedIn);
        };
        let data = self.game_data();
        form_applied(
            form.copy_equipment(&items, data.as_deref()),
            form,
            OperatorCommand::CopyEquipmentToLoadout,
        )
    }

    /// Whether `name` is in game with a published snapshot, so
    /// [`Self::observed_equipment`] has equipment to read.
    pub fn can_observe_equipment(&self, name: &str) -> bool {
        self.status(name).is_some_and(|status| status.ingame)
            && self.with_slot_snapshot(name, |_| ()).is_some()
    }

    /// The equipment `name`'s latest snapshot shows, as item names (trimmed,
    /// blank ones left out) with their ids; `None` unless the member is in
    /// game.
    pub fn observed_equipment(&self, name: &str) -> Option<Vec<(String, i32)>> {
        if !self.status(name).is_some_and(|status| status.ingame) {
            return None;
        }
        self.with_slot_snapshot(name, |snapshot| {
            snapshot
                .equipment()
                .iter()
                .filter_map(|item| {
                    let name = item.def.name.as_deref()?.trim();
                    (!name.is_empty()).then(|| (name.to_string(), item.def.id))
                })
                .collect()
        })
    }
}

/// The member terminal of what a loadout row did to `form`.
fn form_applied(
    result: Result<(), Refused>,
    form: &LoadoutForm,
    command: OperatorCommand,
) -> Applied {
    match result {
        Ok(()) => Applied::Done,
        Err(Refused::NoDraft) if command == OperatorCommand::CopyEquipmentToLoadout => {
            Applied::Skipped(SkipReason::NothingSelected)
        }
        Err(Refused::NoDraft) => {
            Applied::Rejected(Rejection::NotStaged, Some(form.status().to_string()))
        }
        Err(Refused::NothingSelected) => Applied::Skipped(SkipReason::NothingSelected),
        Err(Refused::Invalid) => {
            Applied::Rejected(Rejection::NotApplicable, Some(form.status().to_string()))
        }
        Err(Refused::WriteFailed(error)) => {
            Applied::Failed(FailureKind::WriteFailed, Some(error.to_string()))
        }
    }
}

fn no_model() -> Applied {
    Applied::Rejected(
        Rejection::NoModel,
        Some("loadout form: this front end holds none".into()),
    )
}

#[cfg(test)]
#[path = "loadouts_tests.rs"]
mod tests;
