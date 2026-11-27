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
use crate::launch::{Passphrase, PlayLaunch, UnlockDecisions, UnlockError, UnlockStage};
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
        /// The operator's answers to a first conversion's questions.
        decisions: UnlockDecisions,
        /// Whether the submitting surface can ask those questions.
        interactive: bool,
    },
    /// Delete the locked vault file at `path`, bound to `server`, after the
    /// operator confirmed the server, the file and the loss.
    Reset { path: PathBuf, server: String },
}

impl<Io> OperatorSession<Io> {
    /// Stage the next [`OperatorCommand::UnlockVault`]: open the existing
    /// vault at `path` with `passphrase` (copied into a zeroed-on-drop
    /// buffer) and start `launch` with the submitting surface's frame hook.
    /// A first conversion that needs the operator's answers is refused with
    /// the questions (a start that cannot ask them).
    pub fn stage_vault_unlock(&mut self, path: PathBuf, passphrase: &str, launch: PlayLaunch) {
        self.stage_vault_open(
            path,
            passphrase,
            launch,
            false,
            UnlockDecisions::default(),
            false,
        );
    }

    /// [`Self::stage_vault_unlock`] from a surface that can ask a first
    /// conversion's questions: an unanswered question refuses with the
    /// questions ([`Self::take_vault_questions`]); `decisions` answers them.
    pub fn stage_vault_unlock_interactive(
        &mut self,
        path: PathBuf,
        passphrase: &str,
        launch: PlayLaunch,
        decisions: UnlockDecisions,
    ) {
        self.stage_vault_open(path, passphrase, launch, false, decisions, true);
    }

    /// Stage the next [`OperatorCommand::CreateVault`]: create the vault at
    /// `path` with `passphrase` and start `launch` with the submitting
    /// surface's frame hook.
    pub fn stage_vault_create(&mut self, path: PathBuf, passphrase: &str, launch: PlayLaunch) {
        self.stage_vault_open(
            path,
            passphrase,
            launch,
            true,
            UnlockDecisions::default(),
            false,
        );
    }

    fn stage_vault_open(
        &mut self,
        path: PathBuf,
        passphrase: &str,
        launch: PlayLaunch,
        create: bool,
        decisions: UnlockDecisions,
        interactive: bool,
    ) {
        self.commands.stage_vault(StagedVault::Open {
            path,
            passphrase: Passphrase::from(passphrase),
            launch,
            create,
            decisions,
            interactive,
        });
    }

    /// Stage the next [`OperatorCommand::ResetVault`]: delete the locked
    /// vault file at `path` of server profile `server` (forgotten-passphrase
    /// recovery) and retire its saved profile rows.
    pub fn stage_vault_reset(&mut self, path: PathBuf, server: &str) {
        self.commands.stage_vault(StagedVault::Reset {
            path,
            server: server.to_string(),
        });
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
            self.dispatch_vault(row, staged, request, surface)
        } else {
            Applied::NotAdmitted
        };
        self.settle_app(request, command, applied, submitted);
    }

    fn dispatch_vault<S: SlotSurface<Io = Io>>(
        &mut self,
        row: VaultRow,
        staged: Option<StagedVault>,
        request: RequestId,
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
                    decisions,
                    interactive,
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
                match self.unlock_vault(
                    &path,
                    passphrase.as_str(),
                    launch,
                    hook,
                    &decisions,
                    interactive,
                ) {
                    Ok(()) => Applied::Done,
                    Err(error) => match error.stage() {
                        UnlockStage::Vault => {
                            remember_vault_refusal(self, request, &error);
                            Applied::Rejected(Rejection::VaultRefused, Some(error.to_string()))
                        }
                        UnlockStage::Launch => {
                            Applied::Failed(FailureKind::JobFailed, Some(error.to_string()))
                        }
                    },
                }
            }
            (VaultRow::Reset, Some(StagedVault::Reset { path, server })) => {
                if self.vault.is_some() {
                    return Applied::Rejected(
                        Rejection::NotApplicable,
                        Some("reset vault: unlock / close the session first".into()),
                    );
                }
                let Some(service) = self.local_state.clone() else {
                    return Applied::Failed(
                        FailureKind::WriteFailed,
                        Some("reset vault: the saved state is not open".into()),
                    );
                };
                let target = match host_play::profile_vault::VaultTarget::new(&server, &path) {
                    Ok(target) => target,
                    Err(error) => {
                        return Applied::Rejected(
                            Rejection::VaultRefused,
                            Some(format!("reset vault: {error}")),
                        )
                    }
                };
                match crate::launch::reset_vault(&service, &target) {
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

fn remember_vault_refusal<Io>(
    session: &mut OperatorSession<Io>,
    request: RequestId,
    error: &UnlockError,
) {
    if let Some(refusal) = error
        .vault_error()
        .and_then(host_play::saved_data::SavedDataRefusal::from_vault_error)
    {
        session.vault_refusals.insert(request, refusal);
    }
    if let Some(items) = error.repair_items() {
        session.vault_questions.insert(request, items.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_old_vault_refusal_keeps_its_path_on_the_unlock_request() {
        let path = PathBuf::from("old.vault");
        let error = UnlockError::from_vault(vault::VaultError::UnsupportedSavedFormat {
            path: path.clone(),
        });
        let request = RequestId::isolate(1).unwrap();
        let mut session = OperatorSession::<()>::open(crate::launch::InstancePermit::skip());

        remember_vault_refusal(&mut session, request, &error);

        let refusal = session
            .take_vault_refusal(request)
            .expect("the typed refusal belongs to this unlock");
        assert_eq!(refusal.paths(), &[path]);
        assert!(
            session.take_vault_refusal(request).is_none(),
            "taking the refusal settles it once"
        );
    }

    #[test]
    fn unrelated_vault_errors_do_not_create_saved_format_refusals() {
        let request = RequestId::isolate(2).unwrap();
        let mut session = OperatorSession::<()>::open(crate::launch::InstancePermit::skip());
        let error = UnlockError::from_vault(vault::VaultError::WrongPassphrase);

        remember_vault_refusal(&mut session, request, &error);

        assert!(session.take_vault_refusal(request).is_none());
    }
}
