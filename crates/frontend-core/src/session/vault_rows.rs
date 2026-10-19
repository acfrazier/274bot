//! Vault rows of the command seam: `UnlockVault`, `CreateVault`,
//! `ResetVault`.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{OperatorCommand, Rejection};
use crate::OperatorSession;

/// One vault row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultRow {
    Unlock,
    Create,
    Reset,
}

impl VaultRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Unlock => OperatorCommand::UnlockVault,
            Self::Create => OperatorCommand::CreateVault,
            Self::Reset => OperatorCommand::ResetVault,
        }
    }
}

impl<Io> OperatorSession<Io> {
    pub(super) fn vault_row(
        &mut self,
        row: VaultRow,
        request: RequestId,
        admitted: bool,
        submitted: &mut Submitted,
    ) {
        let applied = if admitted {
            Applied::Rejected(Rejection::NotStaged, Some("vault rows: in progress".into()))
        } else {
            Applied::NotAdmitted
        };
        self.settle_app(request, row.command(), applied, submitted);
    }
}
