//! Vault rows of the command seam: `UnlockVault`, `CreateVault`,
//! `ResetVault`. The front end stages the vault file, the typed passphrase
//! and the play launch ([`OperatorSession::stage_vault_unlock`],
//! [`OperatorSession::stage_vault_create`]) or the file to reset
//! ([`OperatorSession::stage_vault_reset`]); the row opens, creates or
//! deletes the vault and starts the play session with the submitting
//! surface's per-frame client work ([`SlotSurface::frame_hook`]).

use std::path::PathBuf;
use std::sync::Arc;

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{FailureKind, OperatorCommand, Rejection};
use crate::launch::{Passphrase, PlayLaunch, UnlockStage};
use crate::surface::SlotSurface;
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

/// What the next vault row commits.
pub(crate) enum StagedVault {
    /// Open (Unlock) or first-run create (Create) the vault at `path` with
    /// `passphrase`, then start the play `launch` describes with the
    /// surface's [`SlotSurface::frame_hook`] as the per-frame client work.
    Open {
        path: PathBuf,
        passphrase: Passphrase,
        launch: PlayLaunch,
        create: bool,
    },
    /// Delete the locked vault file at `path`.
    Reset { path: PathBuf },
}

impl<Io> OperatorSession<Io> {
    /// Stage the next [`OperatorCommand::UnlockVault`]: open the existing
    /// vault at `path` with `passphrase` (copied into a zeroed-on-drop
    /// buffer) and start `launch` with the submitting surface's frame hook.
    pub fn stage_vault_unlock(&mut self, path: PathBuf, passphrase: &str, launch: PlayLaunch) {
        self.commands.stage_vault(StagedVault::Open {
            path,
            passphrase: Passphrase::from(passphrase),
            launch,
            create: false,
        });
    }

    /// Stage the next [`OperatorCommand::CreateVault`]: create the vault at
    /// `path` with `passphrase` and start `launch` with the submitting
    /// surface's frame hook.
    pub fn stage_vault_create(&mut self, path: PathBuf, passphrase: &str, launch: PlayLaunch) {
        self.commands.stage_vault(StagedVault::Open {
            path,
            passphrase: Passphrase::from(passphrase),
            launch,
            create: true,
        });
    }

    /// Stage the next [`OperatorCommand::ResetVault`]: delete the locked
    /// vault file at `path` (forgotten-passphrase recovery).
    pub fn stage_vault_reset(&mut self, path: PathBuf) {
        self.commands.stage_vault(StagedVault::Reset { path });
    }

    pub(super) fn vault_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: VaultRow,
        request: RequestId,
        admitted: bool,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        let staged = self.commands.take_vault();
        let applied = if admitted {
            self.dispatch_vault(row, staged, surface)
        } else {
            Applied::NotAdmitted
        };
        self.settle_app(request, command, applied, submitted);
    }

    fn dispatch_vault<S: SlotSurface<Io = Io>>(
        &mut self,
        row: VaultRow,
        staged: Option<StagedVault>,
        surface: &mut S,
    ) -> Applied {
        match (row, staged) {
            (
                VaultRow::Unlock | VaultRow::Create,
                Some(StagedVault::Open {
                    path,
                    passphrase,
                    launch,
                    create,
                }),
            ) => {
                if create != (row == VaultRow::Create) {
                    return Applied::Rejected(
                        Rejection::NotStaged,
                        Some("vault: the staged request is for the other row".into()),
                    );
                }
                if self.vault.is_some() {
                    return Applied::Rejected(
                        Rejection::NotApplicable,
                        Some("vault: already unlocked".into()),
                    );
                }
                match (create, path.is_file()) {
                    (false, false) => {
                        return Applied::Rejected(
                            Rejection::VaultRefused,
                            Some(format!("vault: no vault file at {}", path.display())),
                        )
                    }
                    (true, true) => {
                        return Applied::Rejected(
                            Rejection::VaultRefused,
                            Some(format!("vault: {} already exists", path.display())),
                        )
                    }
                    _ => {}
                }
                let hook = surface
                    .frame_hook()
                    .unwrap_or_else(|| Arc::new(crate::frames::NoClientWork));
                match self.unlock_vault(&path, passphrase.as_str(), launch, hook) {
                    Ok(()) => Applied::Done,
                    Err(error) => match error.stage() {
                        UnlockStage::Vault => {
                            Applied::Rejected(Rejection::VaultRefused, Some(error.to_string()))
                        }
                        UnlockStage::Launch => {
                            Applied::Failed(FailureKind::JobFailed, Some(error.to_string()))
                        }
                    },
                }
            }
            (VaultRow::Reset, Some(StagedVault::Reset { path })) => {
                if self.vault.is_some() {
                    return Applied::Rejected(
                        Rejection::NotApplicable,
                        Some("reset vault: unlock / close the session first".into()),
                    );
                }
                match crate::launch::reset_vault_file(&path) {
                    Ok(()) => Applied::Done,
                    Err(error) => Applied::Failed(
                        FailureKind::WriteFailed,
                        Some(format!("reset vault: {error}")),
                    ),
                }
            }
            (VaultRow::Unlock | VaultRow::Create | VaultRow::Reset, _) => Applied::Rejected(
                Rejection::NotStaged,
                Some("vault: nothing staged for this row".into()),
            ),
        }
    }
}
