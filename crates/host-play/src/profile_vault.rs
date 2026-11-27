//! The vault split (S8, OA:10): unlock, first conversion, joined profile
//! writes, reset and recovery over the two stores.
//!
//! The credential-only vault holds each account's exact `username` and
//! `password` ([`vault::Credential`]); the state database holds its `uid`
//! and every setting, keyed by (canonical server profile, canonical account)
//! (`local_state::profiles`). A session is published only after both stores
//! agree ([`unlock`]). Every write that touches both stores records what it
//! expects in the database first, so a stop between the two writes is
//! settled at the next unlock without losing a setting:
//!
//! - **First conversion** ([`unlock`] of a legacy vault): authenticate,
//!   repair account keys and resolve secret spills inside credential
//!   custody, stage the rows in bounded transactions, then one FULL
//!   transaction binds the server, activates the rows and records the seal
//!   with the source and replacement digests. The prepared credential-only
//!   image replaces the source in place (private same-directory rename,
//!   file and directory synced), is authenticated again, and the record is
//!   settled. A wrong passphrase never starts it; a refused commit never
//!   replaces the vault.
//! - **Rename** ([`ProfileStore::commit_rename`]): the renamed row is
//!   prepared with both vault digests, the vault is replaced once, and one
//!   transaction finalizes. The writer holds the vault from the snapshot to
//!   the finalization.
//! - **Reset** ([`reset`]): a reset record with the confirmed file digest
//!   blocks joins, the confirmed file is deleted, and one transaction
//!   retires the rows and cancels pending writes while the binding stays.
//!
//! No password, passphrase or setting value is ever written to a log or an
//! audit line: audit lines carry the action, the server and counts only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::host_log;
use api::hostlog::{Category, Level};
use api::local_state::AccountKey;
use serde_json::{Map, Value};
use vault::{
    Credential, FileDigest, Opened, Profile, ProfileSettings, Vault, VaultBinding, VaultChange,
    VaultError, VaultKey, VaultStore,
};

use crate::local_state::{
    diff_parts, import_pages, stage_batches, stage_items, stage_pieces, LocalStateService,
    NewProfileRow, PartChange, PendingKind, ProfileSqlError, ProfileState, StoredProfile,
    STAGE_BATCH_BYTES,
};

/// A test's named stop: the child process aborts at the point `S8_CRASH_AT`
/// names, so the next unlock meets exactly that interrupted state.
#[cfg(test)]
pub(crate) fn crash_point(name: &str) {
    if std::env::var("S8_CRASH_AT").as_deref() == Ok(name) {
        api::hostlog::abort_process();
    }
}

#[cfg(not(test))]
#[inline]
pub(crate) fn crash_point(_name: &str) {}

/// The vault one launch opens: its file and the binding it must carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultTarget {
    binding: VaultBinding,
    path: PathBuf,
}

impl VaultTarget {
    /// The vault at `path` for the canonical server profile `server` (the
    /// alias-resolved roster name; compared lowercase). The component is
    /// the file name.
    pub fn new(server: &str, path: &Path) -> Result<Self, VaultError> {
        let component = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(VaultError::InvalidBinding("vault component"))?;
        Ok(Self {
            binding: VaultBinding::new(server, component)?,
            path: path.to_path_buf(),
        })
    }

    /// The bound vault of a resolved server profile.
    pub fn for_profile(profile: &crate::profile::ServerProfile) -> Result<Self, VaultError> {
        Self::new(profile.name(), profile.vault_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn binding(&self) -> &VaultBinding {
        &self.binding
    }

    pub fn server(&self) -> &str {
        self.binding.server()
    }

    pub fn component(&self) -> &str {
        self.binding.component()
    }
}

/// What the operator chose for an unlock's repair and spill questions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnlockDecisions {
    /// For an account-key collision (canonical account), the exact username
    /// whose credential is kept.
    pub keep: BTreeMap<String, String>,
    /// The collision's other usernames whose removal (credential and
    /// settings) the operator confirmed. A keep choice takes effect only
    /// when every other username of its group is confirmed here.
    pub remove_collided: BTreeSet<String>,
    /// Usernames that cannot form a login account whose removal the
    /// operator confirmed.
    pub remove_invalid: BTreeSet<String>,
    /// Secret-spill matches by id: clear the affected setting, or confirm it
    /// is not secret.
    pub spill: BTreeMap<String, SpillAction>,
}

/// The operator's answer to one secret-spill match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpillAction {
    /// Remove the setting entry (or reset a field that cannot be absent).
    Clear,
    /// The occurrence is not secret: it may go to the database.
    Confirm,
}

/// One question an unlock needs answered. Names and reasons only: never a
/// password, a setting value or a matched key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairItem {
    /// Usernames that form one canonical account: keep one credential.
    Collision {
        account: String,
        usernames: Vec<String>,
    },
    /// A username that cannot form a login account: confirm its removal.
    Invalid { username: String, reason: String },
    /// A moved setting equals a stored game password.
    Spill {
        id: String,
        username: String,
        category: &'static str,
        /// What a Clear removes or resets.
        clears: &'static str,
    },
}

impl std::fmt::Display for RepairItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Collision { account, usernames } => write!(
                f,
                "usernames {} are one login account ({account}): choose the credential to \
                 keep and confirm removing the others with their settings",
                usernames
                    .iter()
                    .map(|name| format!("{name:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Invalid { username, reason } => write!(
                f,
                "username {username:?} cannot form a login account ({reason}): confirm removing it"
            ),
            Self::Spill {
                username,
                category,
                clears,
                ..
            } => write!(
                f,
                "{username}: a {category} equals a stored game password: clear it ({clears}) or \
                 confirm it is not secret"
            ),
        }
    }
}

/// The step of an unlock or vault write that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPhase {
    /// The server, component or file binding does not match.
    Binding,
    /// Preparing the conversion projection.
    Prepare,
    /// Writing the profile rows.
    Commit,
    /// Publishing the vault replacement.
    Publish,
    /// Checking the published vault.
    Verify,
    /// Joining the credentials to their rows.
    Join,
    /// A forgotten-passphrase reset.
    Reset,
    /// Creating a new vault.
    Create,
}

impl JoinPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Binding => "binding check",
            Self::Prepare => "conversion preparation",
            Self::Commit => "profile row commit",
            Self::Publish => "vault replacement",
            Self::Verify => "vault verification",
            Self::Join => "credential join",
            Self::Reset => "vault reset",
            Self::Create => "vault create",
        }
    }
}

/// Why an unlock, create or reset did not publish a session.
#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    /// Opening or authenticating the vault file failed.
    #[error(transparent)]
    Vault(#[from] VaultError),
    /// A step refused; `detail` names the remedy. Retry repeats the step.
    #[error("{} failed: {detail}", phase.label())]
    Refused { phase: JoinPhase, detail: String },
    /// The first conversion needs the operator's answers.
    #[error("the vault conversion needs {} decision(s) before unlock", .0.len())]
    NeedsDecisions(Vec<RepairItem>),
    /// The same questions on a start that cannot ask them.
    #[error(
        "the vault conversion needs {} decision(s). Unlock interactively (panel or TUI) to \
         choose the credential to keep and confirm removing the others, confirm removal of an \
         invalid name, and clear or confirm each setting that matches a stored password. \
         Nothing was changed.",
        .0.len()
    )]
    NonInteractive(Vec<RepairItem>),
}

impl JoinError {
    fn refused(phase: JoinPhase, detail: impl Into<String>) -> Self {
        Self::Refused {
            phase,
            detail: detail.into(),
        }
    }

    fn sql(phase: JoinPhase, error: ProfileSqlError) -> Self {
        Self::refused(phase, error.to_string())
    }

    /// The failed phase, when one is known.
    pub fn phase(&self) -> Option<JoinPhase> {
        match self {
            Self::Refused { phase, .. } => Some(*phase),
            _ => None,
        }
    }

    /// The questions an interactive unlock asks, if any.
    pub fn repair_items(&self) -> Option<&[RepairItem]> {
        match self {
            Self::NeedsDecisions(items) | Self::NonInteractive(items) => Some(items),
            _ => None,
        }
    }
}

fn audit(server: &str, action: &str, detail: std::fmt::Arguments<'_>) {
    host_log!(
        Category::Lifecycle,
        Level::Info,
        "audit domain=profiles server={server} action={action} {detail}"
    );
}

/// The binding refusal for `state` and `target`, if any.
fn binding_refusal(state: &ProfileState, target: &VaultTarget) -> Option<JoinError> {
    if let Some(binding) = &state.binding {
        if binding.component != target.component() {
            return Some(JoinError::refused(
                JoinPhase::Binding,
                crate::local_state::binding_mismatch(
                    target.server(),
                    target.component(),
                    &binding.component,
                ),
            ));
        }
    }
    match &state.component_server {
        Some(owner) if owner != target.server() => Some(JoinError::refused(
            JoinPhase::Binding,
            crate::local_state::component_taken(target.server(), target.component(), owner),
        )),
        _ => None,
    }
}

fn read_state(
    service: &LocalStateService,
    target: &VaultTarget,
) -> Result<ProfileState, JoinError> {
    let state = service
        .profile_state(target.server(), target.component())
        .map_err(|error| JoinError::sql(JoinPhase::Binding, error))?;
    if let Some(refusal) = binding_refusal(&state, target) {
        return Err(refusal);
    }
    Ok(state)
}

/// Settle a pending reset found at unlock or create: finish it when the file
/// is gone, cancel it when the confirmed file is intact, refuse a different
/// file. `true` when the reset finished (the vault is now absent).
fn settle_reset(
    service: &LocalStateService,
    target: &VaultTarget,
    state: &ProfileState,
) -> Result<bool, JoinError> {
    let Some(record) = state.pending(PendingKind::Reset) else {
        return Ok(false);
    };
    match (vault::current_digest(target.path())?, record.old_digest) {
        (None, _) => {
            service
                .finish_reset(target.server(), target.component())
                .map_err(|error| JoinError::sql(JoinPhase::Reset, error))?;
            audit(
                target.server(),
                "vault-reset-settled",
                format_args!("file=absent"),
            );
            Ok(true)
        }
        (Some(found), Some(confirmed)) if found == confirmed => {
            service
                .cancel_reset(target.server())
                .map_err(|error| JoinError::sql(JoinPhase::Reset, error))?;
            audit(
                target.server(),
                "vault-reset-cancelled",
                format_args!("file=intact"),
            );
            Ok(false)
        }
        (Some(_), _) => Err(JoinError::refused(
            JoinPhase::Reset,
            format!(
                "A vault reset of {} is pending, and a different file appeared at {}. It is \
                 neither deleted nor adopted. Move that file away and unlock again to finish \
                 the reset.",
                target.server(),
                target.path().display()
            ),
        )),
    }
}

/// Unlock the vault of `target`: authenticate it, complete a first
/// conversion or a pending two-write step, and publish the joined session
/// only when both stores agree. A missing file is
/// [`VaultError::NotFound`]; the caller decides whether to [`create`].
/// `interactive` says whether repair and spill questions can be asked.
pub fn unlock(
    service: &LocalStateService,
    target: &VaultTarget,
    passphrase: &str,
    decisions: &UnlockDecisions,
    interactive: bool,
) -> Result<Vault, JoinError> {
    let mut state = read_state(service, target)?;
    if state.pending(PendingKind::Reset).is_some() {
        if settle_reset(service, target, &state)? {
            return Err(VaultError::NotFound(target.path().to_path_buf()).into());
        }
        state = read_state(service, target)?;
    }
    let (bytes, digest) = vault::read_file(target.path())?;
    // A wrong passphrase stops here, before any write.
    let (key, opened) = vault::authenticate(&bytes, passphrase)?;
    drop(bytes);
    match opened {
        Opened::Legacy(profiles) => match &state.binding {
            None => first_conversion(
                service,
                target,
                &state,
                key,
                profiles,
                digest,
                decisions,
                interactive,
            ),
            Some(_) => match state.pending(PendingKind::Seal) {
                Some(seal) if seal.old_digest == Some(digest) => {
                    resume_seal(service, target, &state, key, &profiles, digest)
                }
                Some(_) => Err(JoinError::refused(
                    JoinPhase::Publish,
                    format!(
                        "The conversion of {} is pending, but the legacy vault at {} is not the \
                         one it converted. Restore the matching pending source or a matching \
                         paired backup of the vault and its saved state. Nothing was changed.",
                        target.server(),
                        target.path().display()
                    ),
                )),
                None => Err(JoinError::refused(
                    JoinPhase::Binding,
                    format!(
                        "The vault of {} was already converted; the old-format vault at {} cannot \
                         replace it and is not imported. Restore the converted vault or its \
                         paired backup. Nothing was changed.",
                        target.server(),
                        target.path().display()
                    ),
                )),
            },
        },
        Opened::Sealed {
            binding,
            credentials,
        } => sealed_join(service, target, state, key, binding, credentials, digest),
    }
}

/// [`unlock`], or [`create`] when no file is at the target.
pub fn open_or_create(
    service: &LocalStateService,
    target: &VaultTarget,
    passphrase: &str,
    decisions: &UnlockDecisions,
    interactive: bool,
) -> Result<Vault, JoinError> {
    match unlock(service, target, passphrase, decisions, interactive) {
        Err(JoinError::Vault(VaultError::NotFound(_))) => create(service, target, passphrase),
        other => other,
    }
}

/// Create an empty credential-only vault at `target`: binding first, then
/// the file, so an interrupted create is completed by the next create. A
/// server whose bound file is missing (not reset) is refused. Profiles
/// created later get fresh uids; nothing of a retired row returns.
pub fn create(
    service: &LocalStateService,
    target: &VaultTarget,
    passphrase: &str,
) -> Result<Vault, JoinError> {
    vault::check_new_passphrase(passphrase)?;
    let state = read_state(service, target)?;
    settle_reset(service, target, &state)?;
    if target.path().exists() {
        return Err(VaultError::AlreadyExists(target.path().to_path_buf()).into());
    }
    if let Some(parent) = target.path().parent() {
        if !parent.as_os_str().is_empty() {
            vault::create_private_dir(parent).map_err(VaultError::Io)?;
        }
    }
    service
        .bind_created(target.server(), target.component())
        .map_err(|error| JoinError::sql(JoinPhase::Create, error))?
        .map_err(|refusal| JoinError::refused(JoinPhase::Create, refusal))?;
    let key = VaultKey::create(passphrase)?;
    let blob = vault::seal(&key, target.binding(), &BTreeMap::new())?;
    vault::create_new(target.path(), &blob)?;
    service
        .mark_vault_present(target.server())
        .map_err(|error| JoinError::sql(JoinPhase::Create, error))?;
    audit(
        target.server(),
        "vault-create",
        format_args!("credentials=0"),
    );
    Ok(Vault::joined(
        target.path().to_path_buf(),
        key,
        target.binding().clone(),
        BTreeMap::new(),
    ))
}

/// What a confirmed reset did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetOutcome {
    /// The confirmed file was deleted and the rows retired.
    Deleted,
    /// No file was there; the rows were retired.
    AlreadyAbsent,
}

/// Forgotten-passphrase reset of `target` after the operator confirmed the
/// server, component and file. The confirmed file (by digest) is deleted
/// only after a reset record blocks joins; then one transaction retires the
/// rows and cancels any pending seal or rename. The binding and the closed
/// first import stay: a later legacy vault is not imported.
pub fn reset(service: &LocalStateService, target: &VaultTarget) -> Result<ResetOutcome, JoinError> {
    let state = read_state(service, target)?;
    if state.pending(PendingKind::Reset).is_some() && settle_reset(service, target, &state)? {
        return Ok(ResetOutcome::AlreadyAbsent);
    }
    let digest = vault::current_digest(target.path())?;
    service
        .prepare_reset(target.server(), target.component(), digest)
        .map_err(|error| JoinError::sql(JoinPhase::Reset, error))?
        .map_err(|refusal| JoinError::refused(JoinPhase::Reset, refusal))?;
    crash_point("reset-after-prepare");
    if let Some(digest) = digest {
        match vault::remove_checked(target.path(), &digest) {
            Ok(()) | Err(VaultError::NotFound(_)) => {}
            Err(VaultError::Changed(_)) => {
                return Err(JoinError::refused(
                    JoinPhase::Reset,
                    format!(
                        "The vault file at {} changed after the reset was confirmed; it was \
                         neither deleted nor adopted. Confirm the reset again.",
                        target.path().display()
                    ),
                ))
            }
            Err(error) => return Err(error.into()),
        }
    }
    crash_point("reset-after-delete");
    service
        .finish_reset(target.server(), target.component())
        .map_err(|error| JoinError::sql(JoinPhase::Reset, error))?;
    audit(
        target.server(),
        "vault-reset",
        format_args!(
            "file={}",
            if digest.is_some() {
                "deleted"
            } else {
                "absent"
            }
        ),
    );
    Ok(if digest.is_some() {
        ResetOutcome::Deleted
    } else {
        ResetOutcome::AlreadyAbsent
    })
}

/// The live rows of `state` by exact username.
fn rows_by_username(state: &ProfileState) -> BTreeMap<&str, &StoredProfile> {
    state
        .rows
        .iter()
        .map(|row| (row.username.as_str(), row))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn sealed_join(
    service: &LocalStateService,
    target: &VaultTarget,
    mut state: ProfileState,
    key: VaultKey,
    file_binding: VaultBinding,
    credentials: BTreeMap<String, Credential>,
    digest: FileDigest,
) -> Result<Vault, JoinError> {
    if &file_binding != target.binding() {
        return Err(JoinError::refused(
            JoinPhase::Binding,
            format!(
                "The vault at {} was sealed for server {} (vault {:?}), not {} (vault {:?}). It \
                 does not join, even where its usernames match. Restore the configured vault \
                 file. Nothing was opened or changed.",
                target.path().display(),
                file_binding.server(),
                file_binding.component(),
                target.server(),
                target.component()
            ),
        ));
    }
    let Some(binding) = state.binding.clone() else {
        return Err(JoinError::refused(
            JoinPhase::Join,
            format!(
                "The vault at {} is converted, but the state database has no binding or saved \
                 settings for {}. Restore the paired state backup, or run 'state vault recover \
                 --defaults --profile {}' to create default settings. Nothing was changed.",
                target.path().display(),
                target.server(),
                target.server()
            ),
        ));
    };
    if binding.absent_ok {
        service
            .mark_vault_present(target.server())
            .map_err(|error| JoinError::sql(JoinPhase::Join, error))?;
    }
    let mut settled = false;
    if let Some(seal) = state.pending(PendingKind::Seal) {
        if seal.new_digest != Some(digest) {
            return Err(JoinError::refused(
                JoinPhase::Verify,
                format!(
                    "The conversion of {} is pending, and the vault at {} is neither its source \
                     nor its replacement. Restore the matching pending source or a matching \
                     paired backup. Nothing was changed.",
                    target.server(),
                    target.path().display()
                ),
            ));
        }
        service
            .settle_seal(target.server())
            .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
        audit(
            target.server(),
            "vault-seal-settled",
            format_args!("resumed=1"),
        );
        settled = true;
    }
    if let Some(rename) = state.pending(PendingKind::Rename) {
        if rename.old_digest == Some(digest) {
            service
                .discard_rename(target.server())
                .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
            audit(
                target.server(),
                "profile-rename-discarded",
                format_args!("rows=1"),
            );
        } else if rename.new_digest == Some(digest) {
            service
                .finalize_rename(target.server())
                .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
            audit(
                target.server(),
                "profile-rename",
                format_args!("rows=1 resumed=1"),
            );
        } else {
            return Err(JoinError::refused(
                JoinPhase::Join,
                format!(
                    "A profile rename of {} is pending, and the vault at {} is neither the image \
                     before it nor after it. Restore the matching vault or a paired backup. \
                     Nothing was changed.",
                    target.server(),
                    target.path().display()
                ),
            ));
        }
        settled = true;
    }
    if settled {
        state = read_state(service, target)?;
    }
    let rows = rows_by_username(&state);
    let missing: Vec<&str> = credentials
        .keys()
        .map(String::as_str)
        .filter(|name| !rows.contains_key(name))
        .collect();
    if !missing.is_empty() {
        return Err(JoinError::refused(
            JoinPhase::Join,
            format!(
                "{} credential(s) of {} have no saved settings row ({}). Restore the paired \
                 backup of the vault and its saved state; if no backup is usable, run 'state \
                 vault recover --defaults --profile {}' (the original settings and uids of \
                 those rows are lost). Nothing was changed.",
                missing.len(),
                target.server(),
                missing.join(", "),
                target.server()
            ),
        ));
    }
    // A row whose credential is gone was an interrupted create (row first)
    // or delete (credential hidden first): settle it before any reuse.
    let orphans: Vec<i64> = state
        .rows
        .iter()
        .filter(|row| !credentials.contains_key(&row.username))
        .map(|row| row.id)
        .collect();
    if !orphans.is_empty() {
        let count = orphans.len();
        service
            .retire_profiles(target.server(), orphans)
            .map_err(|error| JoinError::sql(JoinPhase::Join, error))?;
        audit(
            target.server(),
            "profile-retire",
            format_args!("rows={count} reason=no-credential"),
        );
    }
    while service
        .collect_retired_profiles(target.server())
        .map_err(|error| JoinError::sql(JoinPhase::Join, error))?
    {}
    let profiles = join(&state, credentials);
    audit(
        target.server(),
        "vault-join",
        format_args!("profiles={}", profiles.len()),
    );
    Ok(Vault::joined(
        target.path().to_path_buf(),
        key,
        target.binding().clone(),
        profiles,
    ))
}

/// The joined profiles: every credential with its row.
fn join(
    state: &ProfileState,
    mut credentials: BTreeMap<String, Credential>,
) -> BTreeMap<String, Profile> {
    state
        .rows
        .iter()
        .filter_map(|row| {
            let credential = credentials.remove(&row.username)?;
            Some((
                row.username.clone(),
                Profile {
                    username: credential.username,
                    password: credential.password,
                    uid: row.uid,
                    settings: row.settings.clone(),
                },
            ))
        })
        .collect()
}

/// The first conversion's projection: the surviving accounts and what the
/// operator's answers removed or cleared.
struct Projection {
    rows: Vec<NewProfileRow>,
    credentials: BTreeMap<String, Credential>,
    removed: usize,
    cleared: usize,
    confirmed: usize,
}

#[allow(clippy::too_many_arguments)]
fn first_conversion(
    service: &LocalStateService,
    target: &VaultTarget,
    state: &ProfileState,
    key: VaultKey,
    legacy: BTreeMap<String, Profile>,
    source: FileDigest,
    decisions: &UnlockDecisions,
    interactive: bool,
) -> Result<Vault, JoinError> {
    if let Some((name, _)) = legacy
        .iter()
        .find(|(name, profile)| **name != profile.username)
    {
        return Err(JoinError::refused(
            JoinPhase::Prepare,
            format!(
                "The legacy vault at {} stores a profile under a key that is not its username \
                 (near {name:?}). It is not converted. Restore a good copy of the vault. Nothing \
                 was changed.",
                target.path().display()
            ),
        ));
    }
    let projection = project(target.server(), legacy, decisions, interactive)?;
    let blob = vault::seal(&key, target.binding(), &projection.credentials)?;
    let replacement = vault::file_digest(&blob);
    let count = projection.rows.len();
    // The projection validates (every row encodes) before any write.
    let mut items = Vec::new();
    for (index, row) in projection.rows.into_iter().enumerate() {
        stage_items(index, row, &mut items)
            .map_err(|error| JoinError::refused(JoinPhase::Prepare, error))?;
    }
    // An interrupted staging (invisible to every join) goes first, so its
    // pages do not count against this import.
    if state.unbound_rows {
        while service
            .clear_unbound_profiles(target.server())
            .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?
        {}
        audit(
            target.server(),
            "profile-staging-cleared",
            format_args!("interrupted=1"),
        );
    }
    service
        .admit_profile_import(import_pages(&items))
        .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?
        .map_err(|refusal| JoinError::refused(JoinPhase::Commit, refusal))?;
    let generation = service
        .next_profile_generation(target.server())
        .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
    let mut ids = HashMap::new();
    for (index, batch) in stage_batches(items).into_iter().enumerate() {
        ids = service
            .stage_profiles(target.server(), generation, batch, ids)
            .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
        audit(
            target.server(),
            "profile-stage",
            format_args!("batch={}", index + 1),
        );
    }
    crash_point("before-activation");
    service
        .activate_seal(
            target.server(),
            target.component(),
            generation,
            count,
            source,
            replacement,
        )
        .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
    audit(
        target.server(),
        "profile-import",
        format_args!(
            "rows={count} removed={} spill_cleared={} spill_confirmed={}",
            projection.removed, projection.cleared, projection.confirmed
        ),
    );
    crash_point("after-sql-commit");
    publish_seal(service, target, &key, &blob, source, replacement)?;
    let state = read_state(service, target)?;
    let profiles = join(&state, projection.credentials);
    if profiles.len() != count {
        return Err(JoinError::refused(
            JoinPhase::Join,
            "The converted rows and credentials do not agree. Unlock again.",
        ));
    }
    audit(
        target.server(),
        "vault-join",
        format_args!("profiles={count}"),
    );
    Ok(Vault::joined(
        target.path().to_path_buf(),
        key,
        target.binding().clone(),
        profiles,
    ))
}

/// Publish a prepared seal over the unchanged source, authenticate it, and
/// settle its record.
fn publish_seal(
    service: &LocalStateService,
    target: &VaultTarget,
    key: &VaultKey,
    blob: &[u8],
    source: FileDigest,
    replacement: FileDigest,
) -> Result<(), JoinError> {
    vault::replace_checked(target.path(), blob, &source).map_err(|error| match error {
        VaultError::Changed(_) => JoinError::refused(
            JoinPhase::Publish,
            format!(
                "The legacy vault at {} changed after it was read; the conversion was not \
                 published. Unlock again to finish it.",
                target.path().display()
            ),
        ),
        other => JoinError::refused(JoinPhase::Publish, other.to_string()),
    })?;
    audit(target.server(), "vault-seal", format_args!("published=1"));
    crash_point("after-vault-replace");
    let (bytes, digest) = vault::read_file(target.path())?;
    let verified = digest == replacement
        && matches!(
            vault::authenticate_with_key(&bytes, key),
            Ok(Opened::Sealed { ref binding, .. }) if binding == target.binding()
        );
    if !verified {
        return Err(JoinError::refused(
            JoinPhase::Verify,
            format!(
                "The converted vault at {} did not read back as published. Unlock again.",
                target.path().display()
            ),
        ));
    }
    service
        .settle_seal(target.server())
        .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
    audit(
        target.server(),
        "vault-seal-settled",
        format_args!("resumed=0"),
    );
    Ok(())
}

/// Finish a conversion whose rows committed but whose vault was not yet
/// replaced: prepare the credential image again from the unchanged source
/// for exactly the committed rows (no row is copied again) and publish it.
fn resume_seal(
    service: &LocalStateService,
    target: &VaultTarget,
    state: &ProfileState,
    key: VaultKey,
    legacy: &BTreeMap<String, Profile>,
    source: FileDigest,
) -> Result<Vault, JoinError> {
    let mut credentials = BTreeMap::new();
    for row in &state.rows {
        let Some(profile) = legacy.get(&row.username) else {
            return Err(JoinError::refused(
                JoinPhase::Publish,
                format!(
                    "The pending conversion of {} has a row the legacy vault lacks. Restore the \
                     matching pending source. Nothing was changed.",
                    target.server()
                ),
            ));
        };
        credentials.insert(
            row.username.clone(),
            Credential {
                username: profile.username.clone(),
                password: profile.password.clone(),
            },
        );
    }
    let blob = vault::seal(&key, target.binding(), &credentials)?;
    let replacement = vault::file_digest(&blob);
    service
        .reprepare_seal(target.server(), replacement)
        .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?;
    publish_seal(service, target, &key, &blob, source, replacement)?;
    let profiles = join(state, credentials);
    audit(
        target.server(),
        "vault-join",
        format_args!("profiles={} resumed=1", profiles.len()),
    );
    Ok(Vault::joined(
        target.path().to_path_buf(),
        key,
        target.binding().clone(),
        profiles,
    ))
}

/// Why `username` cannot form a login account under `server`.
fn account_of(server: &str, username: &str) -> Result<String, String> {
    AccountKey::try_new(server, username)
        .map(|key| key.account().to_string())
        .map_err(|error| error.to_string())
}

/// Apply the repair and spill answers inside credential custody and build
/// the projection. Unanswered questions stop the conversion before any
/// write.
fn project(
    server: &str,
    legacy: BTreeMap<String, Profile>,
    decisions: &UnlockDecisions,
    interactive: bool,
) -> Result<Projection, JoinError> {
    let mut items = Vec::new();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut invalid = Vec::new();
    for name in legacy.keys() {
        match account_of(server, name) {
            Ok(account) => groups.entry(account).or_default().push(name.clone()),
            Err(reason) => invalid.push((name.clone(), reason)),
        }
    }
    let mut removed: BTreeSet<String> = BTreeSet::new();
    for (account, usernames) in &groups {
        if usernames.len() < 2 {
            continue;
        }
        match decisions.keep.get(account) {
            // The keep choice counts only with every other credential's
            // removal confirmed by name; otherwise the question stays.
            Some(keep)
                if usernames.contains(keep)
                    && usernames
                        .iter()
                        .filter(|name| *name != keep)
                        .all(|name| decisions.remove_collided.contains(name)) =>
            {
                removed.extend(usernames.iter().filter(|name| *name != keep).cloned());
            }
            _ => items.push(RepairItem::Collision {
                account: account.clone(),
                usernames: usernames.clone(),
            }),
        }
    }
    for (username, reason) in invalid {
        if decisions.remove_invalid.contains(&username) {
            removed.insert(username);
        } else {
            items.push(RepairItem::Invalid { username, reason });
        }
    }
    // Every nonempty stored game password of the vault, removed rows too.
    let passwords: BTreeSet<&str> = legacy
        .values()
        .map(|profile| profile.password.as_str())
        .filter(|password| !password.is_empty())
        .collect();
    let mut spills = Vec::new();
    for profile in legacy.values().filter(|p| !removed.contains(&p.username)) {
        scan_spills(
            &profile.username,
            &profile.settings,
            &passwords,
            &mut spills,
        );
    }
    let mut cleared = 0;
    let mut confirmed = 0;
    // A nested match whose containing card key or envelope key is cleared
    // goes with it; otherwise it needs its own answer, whatever its
    // parent's answer is.
    let cleared_above = |spill: &Spill| {
        let mut parent = spill.parent.as_deref();
        while let Some(id) = parent {
            if decisions.spill.get(id) == Some(&SpillAction::Clear) {
                return true;
            }
            parent = spills
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.parent.as_deref());
        }
        false
    };
    for spill in &spills {
        match decisions.spill.get(&spill.id) {
            Some(SpillAction::Clear) => cleared += 1,
            Some(SpillAction::Confirm) => confirmed += 1,
            None if cleared_above(spill) => {}
            None => items.push(RepairItem::Spill {
                id: spill.id.clone(),
                username: spill.username.clone(),
                category: spill.target.category(),
                clears: spill.target.clears(),
            }),
        }
    }
    drop(passwords);
    if !items.is_empty() {
        return Err(if interactive {
            JoinError::NeedsDecisions(items)
        } else {
            JoinError::NonInteractive(items)
        });
    }
    let mut rows = Vec::new();
    let mut credentials = BTreeMap::new();
    let removed_count = removed.len();
    for (name, mut profile) in legacy {
        if removed.contains(&name) {
            continue;
        }
        for spill in spills.iter().filter(|spill| spill.username == name) {
            if decisions.spill.get(&spill.id) == Some(&SpillAction::Clear) {
                spill.target.clear(&mut profile.settings);
            }
        }
        convert_compiled(&mut profile.settings);
        let account = account_of(server, &name)
            .map_err(|reason| JoinError::refused(JoinPhase::Prepare, reason))?;
        credentials.insert(
            name.clone(),
            Credential {
                username: profile.username.clone(),
                password: profile.password.clone(),
            },
        );
        rows.push(NewProfileRow {
            account,
            username: name,
            uid: profile.uid,
            settings: std::mem::take(&mut profile.settings),
        });
    }
    if removed_count > 0 || cleared + confirmed > 0 {
        audit(
            server,
            "vault-repair",
            format_args!(
                "removed={removed_count} spill_cleared={cleared} spill_confirmed={confirmed}"
            ),
        );
    }
    Ok(Projection {
        rows,
        credentials,
        removed: removed_count,
        cleared,
        confirmed,
    })
}

/// The setting a spill match lives in, by position (never by its text).
#[derive(Debug, Clone, PartialEq, Eq)]
enum SpillTarget {
    LampSkill,
    Policy(String),
    Assignment,
    Bag(String),
    /// A member key of a compiled card's settings envelope
    /// (`schema_version` or `values`).
    EnvelopeKey(String),
    BagEntry {
        bag: String,
        entry: String,
        envelope: bool,
    },
}

impl SpillTarget {
    fn category(&self) -> &'static str {
        match self {
            Self::LampSkill => "lamp skill",
            Self::Policy(_) => "random event policy key",
            Self::Assignment => "script assignment",
            Self::Bag(_) => "script settings card key",
            Self::EnvelopeKey(_) => "compiled card settings envelope key",
            Self::BagEntry { .. } => "script setting",
        }
    }

    fn clears(&self) -> &'static str {
        match self {
            Self::LampSkill => "the lamp skill returns to its default, strength",
            Self::Policy(_) => "removes that random event's policy; it flees",
            Self::Assignment => "removes the whole script assignment",
            Self::Bag(_) | Self::EnvelopeKey(_) => "removes every saved setting of that card",
            Self::BagEntry { .. } => "removes the saved setting that contains it",
        }
    }

    fn clear(&self, settings: &mut ProfileSettings) {
        match self {
            Self::LampSkill => settings.lamp_skill = ProfileSettings::default().lamp_skill,
            Self::Policy(key) => {
                settings.random_event_policies.remove(key);
            }
            Self::Assignment => settings.script_assignment = None,
            Self::Bag(key) | Self::EnvelopeKey(key) => {
                settings.script_settings.remove(key);
            }
            Self::BagEntry {
                bag,
                entry,
                envelope,
            } => {
                if let Some(bag) = settings.script_settings.get_mut(bag) {
                    let members = if *envelope {
                        bag.get_mut("values").and_then(Value::as_object_mut)
                    } else {
                        Some(bag)
                    };
                    if let Some(members) = members {
                        members.remove(entry);
                    }
                }
            }
        }
    }
}

/// One spill match: a stable id built from the row and the setting's
/// position, never its content. `parent` is the id of the match on the
/// containing card key or envelope key, if any: clearing that parent
/// removes this setting too, so it needs no answer of its own then.
struct Spill {
    id: String,
    username: String,
    target: SpillTarget,
    parent: Option<String>,
}

/// Whole-value and whole-key matches of moved strings against `passwords`.
/// Usernames and `clue_duel_partner` are account identifiers, never
/// compared; the vault passphrase is never compared. A match on a card key
/// or an envelope key does not end the scan: every nested match is its own
/// question.
fn scan_spills(
    username: &str,
    settings: &ProfileSettings,
    passwords: &BTreeSet<&str>,
    out: &mut Vec<Spill>,
) {
    let hit = |text: &str| passwords.contains(text);
    let mut push = |id: String, target: SpillTarget, parent: Option<String>| {
        if !out.iter().any(|spill| spill.id == id) {
            out.push(Spill {
                id,
                username: username.to_string(),
                target,
                parent,
            });
        }
    };
    if hit(&settings.lamp_skill) {
        push(format!("{username}#lamp"), SpillTarget::LampSkill, None);
    }
    for (index, key) in settings.random_event_policies.keys().enumerate() {
        if hit(key) {
            push(
                format!("{username}#policy{index}"),
                SpillTarget::Policy(key.clone()),
                None,
            );
        }
    }
    if let Some(assignment) = &settings.script_assignment {
        if [
            assignment.source_kind.as_str(),
            assignment.identity.as_str(),
            assignment.display_name.as_str(),
        ]
        .into_iter()
        .chain(assignment.unavailable.as_deref())
        .any(hit)
        {
            push(
                format!("{username}#assignment"),
                SpillTarget::Assignment,
                None,
            );
        }
    }
    for (bag_index, (bag_key, bag)) in settings.script_settings.iter().enumerate() {
        let mut parent = None;
        if hit(bag_key) {
            let id = format!("{username}#bag{bag_index}");
            push(id.clone(), SpillTarget::Bag(bag_key.clone()), None);
            parent = Some(id);
        }
        // Only a compiled card's bag is a settings envelope. Its member keys
        // move to the database like every other key, so they are checked
        // too (an empty compiled entry is stored as the schema-1 envelope,
        // so it gains them); any other bag is a plain map whatever its shape.
        let compiled = script::compiled_cards()
            .iter()
            .any(|card| script::compiled_identity_key(card.id) == *bag_key);
        let envelope =
            compiled && vault::CompiledSettingsRecord::view(bag).is_ok() && !bag.is_empty();
        if (envelope || (compiled && bag.is_empty()))
            && (["schema_version", "values"].into_iter().any(hit) || bag.keys().any(|key| hit(key)))
        {
            let id = format!("{username}#bag{bag_index}.envelope");
            push(
                id.clone(),
                SpillTarget::EnvelopeKey(bag_key.clone()),
                parent.clone(),
            );
            parent = Some(id);
        }
        let members = if envelope {
            bag.get("values").and_then(Value::as_object)
        } else {
            Some(bag)
        };
        let Some(members) = members else {
            continue;
        };
        for (entry_index, (entry, value)) in members.iter().enumerate() {
            if hit(entry) || value_spills(value, &hit) {
                push(
                    format!("{username}#bag{bag_index}.{entry_index}"),
                    SpillTarget::BagEntry {
                        bag: bag_key.clone(),
                        entry: entry.clone(),
                        envelope,
                    },
                    parent.clone(),
                );
            }
        }
    }
}

/// Whether a nested key or string value equals a stored password.
fn value_spills(value: &Value, hit: &impl Fn(&str) -> bool) -> bool {
    match value {
        Value::String(text) => hit(text),
        Value::Array(items) => items.iter().any(|item| value_spills(item, hit)),
        Value::Object(members) => members
            .iter()
            .any(|(key, value)| hit(key) || value_spills(value, hit)),
        _ => false,
    }
}

/// The compiled-card conversions of the first import (SD1 §5): an empty
/// `compiled:<card>` entry is stored as the explicit schema-1 envelope with
/// an empty `values` object, never promoted; an envelope at a version the
/// card reads as its own (Gatherer 4) is stored at the card's current
/// version with the same overrides. Every other entry, including an
/// unsupported or malformed one, keeps its original representation and its
/// per-card refusal at use.
pub fn convert_compiled(settings: &mut ProfileSettings) {
    for (key, entry) in settings.script_settings.iter_mut() {
        let Some(card) = script::compiled_cards()
            .iter()
            .find(|card| script::compiled_identity_key(card.id) == *key)
        else {
            continue;
        };
        if entry.is_empty() {
            *entry = vault::CompiledSettingsRecord {
                schema_version: 1,
                values: Map::new(),
            }
            .into_entry();
            continue;
        }
        let promote = match vault::CompiledSettingsRecord::view(entry) {
            Ok((version, _)) => {
                version != card.schema_version && card.reads_schema_versions.contains(&version)
            }
            Err(_) => false,
        };
        if promote {
            if let Some(version) = entry.get_mut("schema_version") {
                *version = Value::from(card.schema_version);
            }
        }
    }
}

/// The durable joined store one profile writer owns: the credential vault
/// (`vault`) and the server's rows. It serializes every write to that vault
/// file; a rename holds it from its ciphertext snapshot through the SQL
/// finalization.
pub struct ProfileStore {
    vault: VaultStore,
    backend: Backend,
}

enum Backend {
    Joined {
        service: Arc<LocalStateService>,
        /// Live row ids by exact username, read at the first commit.
        ids: Option<HashMap<String, i64>>,
        /// The vault image the last write published, read at first commit.
        digest: Option<FileDigest>,
        /// Rows whose credential was hidden but whose retire did not commit.
        retire: Vec<i64>,
        /// A two-write step that must settle at the next unlock first.
        blocked: Option<String>,
    },
    /// A legacy-format test fixture: writes go to its file in the old
    /// format, as the fixture's own writer would.
    #[cfg(any(test, feature = "test-support"))]
    Legacy,
}

impl ProfileStore {
    /// The store for a joined vault over `service`.
    pub fn joined(vault: VaultStore, service: Arc<LocalStateService>) -> Self {
        Self {
            vault,
            backend: Backend::Joined {
                service,
                ids: None,
                digest: None,
                retire: Vec::new(),
                blocked: None,
            },
        }
    }

    /// The store of a legacy-format test fixture vault.
    #[cfg(any(test, feature = "test-support"))]
    pub fn legacy(vault: VaultStore) -> Self {
        Self {
            vault,
            backend: Backend::Legacy,
        }
    }

    /// The store for `vault`: joined over `service` when the vault is
    /// bound, else (test builds only) its legacy fixture writer.
    pub fn for_vault(
        vault: VaultStore,
        service: Option<Arc<LocalStateService>>,
    ) -> Result<Self, String> {
        match (vault.binding().is_some(), service) {
            (true, Some(service)) => Ok(Self::joined(vault, service)),
            (true, None) => Err("vault writes need the state service".into()),
            #[cfg(any(test, feature = "test-support"))]
            (false, _) => Ok(Self::legacy(vault)),
            #[cfg(not(any(test, feature = "test-support")))]
            (false, _) => Err("an unbound vault cannot be written".into()),
        }
    }

    /// The durable value of one profile.
    pub fn get(&self, username: &str) -> Option<&Profile> {
        self.vault.get(username)
    }

    /// Apply `changes` (no rename among them) to both stores: settings-only
    /// edits commit in SQL; credential-only edits replace the vault and keep
    /// its binding; a create prepares its row before its credential; a
    /// delete hides its credential before retiring its row. On error the
    /// durable copy is unchanged.
    pub fn commit(&mut self, changes: &[VaultChange]) -> Result<(), String> {
        #[cfg(any(test, feature = "test-support"))]
        if matches!(self.backend, Backend::Legacy) {
            return self.vault.commit_legacy(changes).map_err(|e| e.to_string());
        }
        let next = self.vault.applied(changes);
        // A delete hides its credential before its row retires, and a create
        // prepares its row first; a batch with both (a delete of `Alice`
        // then a create of `alice`) settles the deletes as their own write.
        let deleted: Vec<String> = changes
            .iter()
            .map(VaultChange::username)
            .filter(|name| self.vault.get(name).is_some() && !next.contains_key(*name))
            .map(str::to_string)
            .collect();
        let creating = changes
            .iter()
            .map(VaultChange::username)
            .any(|name| self.vault.get(name).is_none() && next.contains_key(name));
        if !deleted.is_empty() && creating {
            let removes: Vec<VaultChange> = deleted.into_iter().map(VaultChange::Remove).collect();
            self.commit(&removes)?;
            return self.commit(changes);
        }
        let Self { vault, backend } = self;
        let (service, ids, digest, retire, blocked) = match backend {
            Backend::Joined {
                service,
                ids,
                digest,
                retire,
                blocked,
            } => (service, ids, digest, retire, blocked),
            #[cfg(any(test, feature = "test-support"))]
            Backend::Legacy => unreachable!("legacy handled above"),
        };
        if let Some(reason) = blocked {
            return Err(reason.clone());
        }
        let binding = vault.binding().expect("joined store is bound").clone();
        let server = binding.server().to_string();
        load_joined(service, vault, &binding, ids, digest)?;
        let ids = ids.as_mut().expect("loaded");
        let durable = vault.profiles();
        let mut creates = Vec::new();
        let mut updates = Vec::new();
        let mut deletes = Vec::new();
        let mut credentials_changed = false;
        for name in changes
            .iter()
            .map(VaultChange::username)
            .collect::<BTreeSet<_>>()
        {
            match (durable.get(name), next.get(name)) {
                (None, Some(new)) => {
                    let account = account_of(&server, name)
                        .map_err(|reason| format!("profile {name}: {reason}"))?;
                    creates.push(NewProfileRow {
                        account,
                        username: name.to_string(),
                        uid: new.uid,
                        settings: new.settings.clone(),
                    });
                    credentials_changed = true;
                }
                (Some(_), None) => {
                    deletes.push(name.to_string());
                    credentials_changed = true;
                }
                (Some(old), Some(new)) => {
                    if old.password != new.password {
                        credentials_changed = true;
                    }
                    if old.uid != new.uid {
                        return Err(format!("profile {name}: a uid never changes"));
                    }
                    let parts = diff_parts(&old.settings, &new.settings)?;
                    if !parts.is_empty() {
                        let id = *ids
                            .get(name)
                            .ok_or_else(|| format!("profile {name} has no saved row"))?;
                        updates.push((name.to_string(), id, parts, new));
                    }
                }
                (None, None) => {}
            }
        }
        let mut created_ids = Vec::new();
        if !creates.is_empty() {
            let count = creates.len();
            let usernames: Vec<String> = creates.iter().map(|row| row.username.clone()).collect();
            let created = match service.create_profiles(&server, creates) {
                Ok(Ok(created)) => created,
                // A refused or failed create left only retired or staged
                // rows: collect them now.
                Ok(Err(refusal)) => {
                    collect_obsolete(service, &server);
                    return Err(refusal);
                }
                Err(error) => {
                    collect_obsolete(service, &server);
                    return Err(error.to_string());
                }
            };
            for (name, id) in usernames.into_iter().zip(created) {
                ids.insert(name.clone(), id);
                created_ids.push((name, id));
            }
            audit(&server, "profile-create", format_args!("rows={count}"));
            crash_point("create-after-row");
        }
        let mut obsolete = false;
        if !updates.is_empty() {
            let count = updates.len();
            match save_settings(service, &server, ids, updates) {
                Ok(copied) => obsolete = copied,
                Err(error) => {
                    // Nothing of the settings batch was saved; the rows this
                    // commit created have no credential and retire now (or
                    // with the next write; the next unlock settles them too).
                    let rows: Vec<i64> = created_ids.iter().map(|(_, id)| *id).collect();
                    for (name, _) in &created_ids {
                        ids.remove(name);
                    }
                    if !rows.is_empty() {
                        if service.retire_profiles(&server, rows.clone()).is_err() {
                            retire.extend(rows);
                        } else {
                            collect_obsolete(service, &server);
                        }
                    }
                    return Err(error);
                }
            }
            audit(&server, "profile-settings", format_args!("rows={count}"));
        }
        if credentials_changed {
            let blob = vault::seal(vault.key(), &binding, &vault::credentials_of(&next))
                .map_err(|error| error.to_string())?;
            let expected = digest.expect("loaded");
            match vault::replace_checked(vault.path(), &blob, &expected) {
                Ok(()) => {
                    *digest = Some(vault::file_digest(&blob));
                    audit(
                        &server,
                        "vault-credentials",
                        format_args!("credentials={}", next.len()),
                    );
                }
                Err(error @ VaultError::Unsynced { .. }) => {
                    // The new image is in place but may not survive a crash:
                    // keep every row (created and deleted ones alike) and let
                    // the next unlock settle them against the image the file
                    // holds (rows without a credential retire there).
                    *blocked = Some(format!(
                        "a vault write of {server} may not be durable; unlock again to settle it"
                    ));
                    return Err(error.to_string());
                }
                Err(error) => {
                    // Nothing was published.
                    if let VaultError::Changed(_) = error {
                        *blocked = Some(format!(
                            "the vault file of {server} changed outside this session; unlock \
                             again"
                        ));
                    }
                    // The created rows have no credential: retire them now,
                    // or with the next write (the next unlock settles them too).
                    let rows: Vec<i64> = created_ids.iter().map(|(_, id)| *id).collect();
                    for (name, _) in &created_ids {
                        ids.remove(name);
                    }
                    if !rows.is_empty() {
                        if service.retire_profiles(&server, rows.clone()).is_err() {
                            retire.extend(rows);
                        } else {
                            collect_obsolete(service, &server);
                        }
                    }
                    return Err(error.to_string());
                }
            }
        }
        crash_point("delete-after-vault");
        retire.extend(deletes.iter().filter_map(|name| ids.get(name).copied()));
        if !retire.is_empty() {
            let rows = std::mem::take(retire);
            let count = rows.len();
            match service.retire_profiles(&server, rows.clone()) {
                Ok(()) => {
                    audit(&server, "profile-retire", format_args!("rows={count}"));
                    obsolete = true;
                }
                Err(error) => {
                    // The credentials are gone already, so the delete stands;
                    // the rows retire with the next write or at the next unlock.
                    host_log!(
                        Category::Lifecycle,
                        Level::Warn,
                        "profiles: {count} deleted row(s) of {server} retire later: {error}"
                    );
                    retire.extend(rows);
                }
            }
        }
        for name in &deletes {
            ids.remove(name);
        }
        if obsolete {
            collect_obsolete(service, &server);
        }
        vault.adopt(next);
        Ok(())
    }

    /// Rename `old` to `profile.username` (with any credential and settings
    /// edit of the same command): prepare the renamed row with both vault
    /// digests, replace the vault once, finalize in one transaction. An
    /// occupied target changes neither store.
    pub fn commit_rename(&mut self, old: &str, profile: &Profile) -> Result<(), String> {
        crash_point("rename-before-prepare");
        let changes = [
            VaultChange::Upsert(profile.clone()),
            VaultChange::Remove(old.to_string()),
        ];
        #[cfg(any(test, feature = "test-support"))]
        if matches!(self.backend, Backend::Legacy) {
            return self
                .vault
                .commit_legacy(&changes)
                .map_err(|e| e.to_string());
        }
        let next = self.vault.applied(&changes);
        let Self { vault, backend } = self;
        let (service, ids, digest, blocked) = match backend {
            Backend::Joined {
                service,
                ids,
                digest,
                blocked,
                ..
            } => (service, ids, digest, blocked),
            #[cfg(any(test, feature = "test-support"))]
            Backend::Legacy => unreachable!("legacy handled above"),
        };
        if let Some(reason) = blocked {
            return Err(reason.clone());
        }
        let binding = vault.binding().expect("joined store is bound").clone();
        let server = binding.server().to_string();
        load_joined(service, vault, &binding, ids, digest)?;
        let ids = ids.as_mut().expect("loaded");
        let old_row = *ids
            .get(old)
            .ok_or_else(|| format!("profile {old} has no saved row"))?;
        let previous = vault
            .get(old)
            .ok_or_else(|| format!("profile {old} is not saved"))?;
        if previous.uid != profile.uid {
            return Err(format!("profile {old}: a rename keeps its uid"));
        }
        let account = account_of(&server, &profile.username)
            .map_err(|reason| format!("profile {}: {reason}", profile.username))?;
        let blob = vault::seal(vault.key(), &binding, &vault::credentials_of(&next))
            .map_err(|error| error.to_string())?;
        let old_digest = digest.expect("loaded");
        let new_digest = vault::file_digest(&blob);
        let prepared = service
            .prepare_rename(
                &server,
                old_row,
                NewProfileRow {
                    account,
                    username: profile.username.clone(),
                    uid: profile.uid,
                    settings: ProfileSettings::default(),
                },
                old_digest,
                new_digest,
            )
            .map_err(|error| error.to_string())??;
        audit(&server, "profile-rename-prepare", format_args!("rows=1"));
        crash_point("rename-after-prepare");
        // The renamed row's settings follow in bounded batches; the pending
        // record keeps the row out of every join and collection meanwhile.
        let mut items = Vec::new();
        stage_pieces(0, profile.settings.clone(), &mut items)?;
        let mut staged = HashMap::from([(0, prepared)]);
        for batch in stage_batches(items) {
            match service.stage_bound_profiles(&server, batch, staged) {
                Ok(next_ids) => staged = next_ids,
                Err(error) => {
                    if service.discard_rename(&server).is_ok() {
                        collect_obsolete(service, &server);
                    } else {
                        *blocked = Some(format!(
                            "a rename of {server} is pending; unlock again to settle it"
                        ));
                    }
                    return Err(error.to_string());
                }
            }
        }
        crash_point("rename-after-stage");
        match vault::replace_checked(vault.path(), &blob, &old_digest) {
            Ok(()) => {}
            Err(error @ VaultError::Unsynced { .. }) => {
                // The renamed image is in place but may not survive a crash:
                // the pending record stays, and the next unlock finalizes or
                // discards the rename by the image the file then holds.
                *blocked = Some(format!(
                    "a rename of {server} is pending; unlock again to settle it"
                ));
                return Err(error.to_string());
            }
            Err(error) => {
                // Nothing was published: the old image stays authoritative.
                if service.discard_rename(&server).is_ok() {
                    collect_obsolete(service, &server);
                } else {
                    *blocked = Some(format!(
                        "a rename of {server} is pending; unlock again to settle it"
                    ));
                }
                return Err(error.to_string());
            }
        }
        *digest = Some(new_digest);
        audit(
            &server,
            "vault-credentials",
            format_args!("credentials={}", next.len()),
        );
        crash_point("rename-after-vault");
        if let Err(error) = service.finalize_rename(&server) {
            *blocked = Some(format!(
                "a rename of {server} is pending; unlock again to settle it ({error})"
            ));
            return Err(error.to_string());
        }
        audit(&server, "profile-rename", format_args!("rows=1"));
        crash_point("rename-after-finalize");
        collect_obsolete(service, &server);
        ids.remove(old);
        ids.insert(profile.username.clone(), prepared);
        vault.adopt(next);
        Ok(())
    }
}

/// Read the live row ids and the current vault image at a store's first
/// commit, and check they match the durable profiles.
fn load_joined(
    service: &LocalStateService,
    vault: &VaultStore,
    binding: &VaultBinding,
    ids: &mut Option<HashMap<String, i64>>,
    digest: &mut Option<FileDigest>,
) -> Result<(), String> {
    if ids.is_some() && digest.is_some() {
        return Ok(());
    }
    let state = service
        .profile_state(binding.server(), binding.component())
        .map_err(|error| error.to_string())?;
    if !state.pending.is_empty() {
        return Err("a vault write is pending; unlock again to settle it".into());
    }
    let live: HashMap<String, i64> = state
        .rows
        .iter()
        .map(|row| (row.username.clone(), row.id))
        .collect();
    if live.len() != vault.profiles().len()
        || vault.profiles().keys().any(|name| !live.contains_key(name))
    {
        return Err("the saved profile rows changed outside this session; unlock again".into());
    }
    let current = vault::current_digest(vault.path())
        .map_err(|error| error.to_string())?
        .ok_or("the vault file is missing; unlock again")?;
    *ids = Some(live);
    *digest = Some(current);
    Ok(())
}

/// One settings edit of a live row: its username, row id, part changes and
/// the edited profile.
type SettingsEdit<'a> = (String, i64, Vec<PartChange>, &'a Profile);

/// Save one commit's settings edits atomically, within the SQL transaction
/// bound. A batch of at most [`STAGE_BATCH_BYTES`] written and removed
/// settings bytes changes its parts in place in one transaction. A larger
/// batch stages a full copy of every edited row, hidden, in bounded
/// batches, then makes every copy live and retires every replaced row in
/// one small transaction. Either every edit is saved or none is: a failure
/// while staging leaves the live rows as they were and collects the staged
/// copies. `ids` follows the copied rows. Returns whether rows were
/// retired (so the caller collects them).
fn save_settings(
    service: &LocalStateService,
    server: &str,
    ids: &mut HashMap<String, i64>,
    edits: Vec<SettingsEdit<'_>>,
) -> Result<bool, String> {
    let dirty: usize = edits
        .iter()
        .flat_map(|(_, _, parts, _)| parts.iter())
        .map(PartChange::dirty_bytes)
        .sum();
    if dirty <= STAGE_BATCH_BYTES {
        service
            .update_profiles(
                server,
                edits
                    .into_iter()
                    .map(|(_, id, parts, _)| (id, parts))
                    .collect(),
            )
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    let mut items = Vec::new();
    let mut replaced = Vec::with_capacity(edits.len());
    for (key, (name, id, _, profile)) in edits.into_iter().enumerate() {
        let account = account_of(server, &profile.username)
            .map_err(|reason| format!("profile {}: {reason}", profile.username))?;
        stage_items(
            key,
            NewProfileRow {
                account,
                username: profile.username.clone(),
                uid: profile.uid,
                settings: profile.settings.clone(),
            },
            &mut items,
        )?;
        replaced.push((name, id));
    }
    let mut staged = HashMap::with_capacity(replaced.len());
    for batch in stage_batches(items) {
        match service.stage_bound_profiles(server, batch, staged) {
            Ok(next) => staged = next,
            Err(error) => {
                collect_obsolete(service, server);
                return Err(error.to_string());
            }
        }
    }
    crash_point("copy-after-stage");
    let pairs: Vec<(i64, i64)> = replaced
        .iter()
        .enumerate()
        .map(|(key, (_, old))| {
            staged
                .get(&key)
                .map(|copy| (*old, *copy))
                .ok_or("a staged profile copy is missing")
        })
        .collect::<Result<_, _>>()?;
    if let Err(error) = service.replace_profile_rows(server, pairs.clone()) {
        collect_obsolete(service, server);
        return Err(error.to_string());
    }
    for ((name, _), (_, copy)) in replaced.into_iter().zip(pairs) {
        ids.insert(name, copy);
    }
    Ok(true)
}

/// Delete `server`'s retired rows and unnamed staged copies now, in bounded
/// batches ([`LocalStateService::collect_retired_profiles`]), so a running
/// session's edits, renames and deletes never accumulate obsolete copies
/// until the next unlock. A failure is logged: the next write or unlock
/// collects them instead.
fn collect_obsolete(service: &LocalStateService, server: &str) {
    loop {
        match service.collect_retired_profiles(server) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                host_log!(
                    Category::Lifecycle,
                    Level::Warn,
                    "profiles: obsolete rows of {server} are collected later: {error}"
                );
                return;
            }
        }
    }
}

/// Missing-row recovery (`state vault recover --defaults --profile
/// <server>`): authenticate the configured vault's binding, then create only
/// the rows its credentials lack with product defaults and fresh uids.
/// Refused for a pending conversion, rename or reset, a legacy vault, or a
/// vault bound elsewhere. `confirm` sees the count and the loss before any
/// write.
pub fn recover_defaults(
    service: &LocalStateService,
    target: &VaultTarget,
    passphrase: &str,
    confirm: &dyn Fn(&str) -> bool,
) -> Result<String, JoinError> {
    let state = read_state(service, target)?;
    if !state.pending.is_empty() {
        return Err(JoinError::refused(
            JoinPhase::Join,
            format!(
                "A vault write of {} is pending. Unlock it first; defaults are never applied to \
                 a pending split. Nothing was changed.",
                target.server()
            ),
        ));
    }
    let (bytes, _) = vault::read_file(target.path())?;
    let (_, opened) = vault::authenticate(&bytes, passphrase)?;
    let Opened::Sealed {
        binding,
        credentials,
    } = opened
    else {
        return Err(JoinError::refused(
            JoinPhase::Binding,
            "This vault is not converted yet; unlock it to convert it. Nothing was changed.",
        ));
    };
    if &binding != target.binding() {
        return Err(JoinError::refused(
            JoinPhase::Binding,
            format!(
                "The vault at {} was sealed for server {}, not {}. Defaults are never applied \
                 to a mismatched vault. Nothing was changed.",
                target.path().display(),
                binding.server(),
                target.server()
            ),
        ));
    }
    let rows = rows_by_username(&state);
    let missing: Vec<&str> = credentials
        .keys()
        .map(String::as_str)
        .filter(|name| !rows.contains_key(name))
        .collect();
    if missing.is_empty() {
        return Ok(format!(
            "Every credential of {} has its saved settings row; nothing was changed.",
            target.server()
        ));
    }
    let question = format!(
        "{} credential(s) of {} have no saved settings row. Recovery creates them with product \
         defaults and fresh uids; their original settings and uids are lost. Surviving rows \
         are kept. Type yes to create {} default row(s):",
        missing.len(),
        target.server(),
        missing.len()
    );
    if !confirm(&question) {
        return Ok("Recovery was not confirmed; nothing was changed.".into());
    }
    // Above every uid this server ever stored (the binding's high-water,
    // which a missing or deleted row never lowers) and every surviving row.
    let high = state.binding.as_ref().map_or(0, |binding| binding.uid_high);
    let surviving = state
        .rows
        .iter()
        .map(|row| i64::from(row.uid))
        .max()
        .unwrap_or(0);
    let first = high
        .max(surviving)
        .saturating_add(1)
        .max(i64::from(FIRST_UID));
    let mut next_uid = i32::try_from(first).map_err(|_| {
        JoinError::refused(
            JoinPhase::Prepare,
            "No fresh uid is left for this server. Nothing was changed.",
        )
    })?;
    let mut new_rows = Vec::with_capacity(missing.len());
    for name in &missing {
        let account = account_of(target.server(), name)
            .map_err(|reason| JoinError::refused(JoinPhase::Prepare, reason))?;
        new_rows.push(NewProfileRow {
            account,
            username: (*name).to_string(),
            uid: next_uid,
            settings: ProfileSettings::default(),
        });
        next_uid = next_uid.saturating_add(1);
    }
    let count = new_rows.len();
    service
        .recover_profiles(target.server(), target.component(), new_rows)
        .map_err(|error| JoinError::sql(JoinPhase::Commit, error))?
        .map_err(|refusal| JoinError::refused(JoinPhase::Commit, refusal))?;
    audit(
        target.server(),
        "profile-recover-defaults",
        format_args!("rows={count} version=1"),
    );
    Ok(format!(
        "Created {count} default settings row(s) for {}. Their original settings and uids \
         were not recoverable.",
        target.server()
    ))
}

/// The first profile uid (the 274M base).
pub const FIRST_UID: i32 = 274_000_001;

#[cfg(test)]
#[path = "profile_vault_tests.rs"]
mod tests;
