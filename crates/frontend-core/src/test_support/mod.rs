//! Fixtures for front-end tests (`test-support`). Panel and TUI tests build
//! their stores, vaults and script/live fixtures here, so they never name
//! the crates below the operator seam. Nothing here is compiled into a
//! production build.

#[cfg(feature = "live-harness")]
pub mod live;
pub mod paint;
pub mod scripts;
pub mod session;
pub mod tui;

use std::path::Path;
#[cfg(feature = "js")]
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Wait for an existing completion predicate after WorkAdmission publications.
///
/// The caller supplies a timeout as a hang guard. The function returns false
/// if the shared service becomes unavailable or the hang guard expires.
pub fn wait_for_work_admission_publication(
    timeout: Duration,
    mut completed: impl FnMut() -> bool,
) -> bool {
    use script::work_admission::{PublicationWaitOutcome, WorkService};

    let started = Instant::now();
    if completed() {
        return true;
    }
    let Ok(service) = WorkService::shared() else {
        return false;
    };

    loop {
        if completed() {
            return true;
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return false;
        }

        let observed = service.lock().publication_epoch();
        if completed() {
            return true;
        }
        match WorkService::wait_for_publication_change(&service, observed, remaining) {
            PublicationWaitOutcome::Changed => {}
            PublicationWaitOutcome::TimedOut | PublicationWaitOutcome::Unavailable => {
                return false;
            }
        }
    }
}

/// A scratch `$HOME` (and cleared `$RS2B0T`) pinned to this thread while it
/// lives; dropping it restores the previous pin, even on panic. The process
/// environment is never changed.
pub struct IsolatedStores {
    env: Box<script::IsolatedEnv>,
}

impl IsolatedStores {
    /// Pin a fresh scratch home for this thread, named after `label`.
    pub fn enter(label: &str) -> Self {
        Self {
            env: Box::new(script::IsolatedEnv::enter(label)),
        }
    }

    /// Pin this thread's stores away from the operator home for the rest of
    /// the thread (test threads are reused), unless already pinned.
    pub fn ensure_thread() {
        script::IsolatedEnv::ensure_thread();
    }

    /// The scratch directory (parent of [`Self::home`]).
    pub fn dir(&self) -> &Path {
        &self.env.dir
    }

    /// The scratch home every store path resolves under.
    pub fn home(&self) -> &Path {
        &self.env.home
    }

    /// Select `root` as this thread's script catalogue (`$RS2B0T`).
    #[cfg(feature = "js")]
    pub fn set_catalog_root(&self, root: &Path) {
        self.env.set_rs2b0t(root);
    }

    /// Clear this thread's script catalogue selection.
    #[cfg(feature = "js")]
    pub fn clear_catalog_root(&self) {
        self.env.clear_rs2b0t();
    }
}

/// A `Play` over `options` with no saved profiles, no slot threads and no
/// per-frame client work: a test attaches its own arms and status rows. The
/// (empty) profile list stays inside frontend-core.
pub fn empty_play(options: &host_play::PlayOptions) -> host_play::Play {
    host_play::run_with_io(options, Vec::new(), |_| (None, None), |_, _, _| {})
}

/// The thread's script catalogue selection (`$RS2B0T`, or the isolated
/// pin), read before [`IsolatedStores::enter`] clears it.
#[cfg(feature = "js")]
pub fn catalog_root() -> Option<PathBuf> {
    script::rs2b0t_env()
}

/// An encrypted vault file a test writes before a session opens it.
pub struct VaultFixture {
    pub(crate) vault: vault::Vault,
}

impl VaultFixture {
    /// Create a new vault at `path` (panics on failure: a fixture).
    pub fn create(path: &Path, passphrase: &str) -> Self {
        Self {
            vault: vault::Vault::create(path, passphrase).expect("create fixture vault"),
        }
    }

    /// Open an existing vault at `path` (panics on failure: a fixture).
    pub fn unlock(path: &Path, passphrase: &str) -> Self {
        Self {
            vault: vault::Vault::unlock(path, passphrase).expect("unlock fixture vault"),
        }
    }

    /// Insert or replace `profile` and rewrite the file.
    pub fn put(&mut self, profile: FixtureProfile) -> &mut Self {
        self.vault
            .upsert(profile.profile)
            .expect("write fixture profile");
        self
    }

    /// Remove `username`; whether it existed.
    pub fn remove(&mut self, username: &str) -> bool {
        self.vault.remove(username).expect("rewrite fixture vault")
    }

    /// The stored profile `username`, as the forms show it.
    pub fn profile(&self, username: &str) -> Option<crate::ProfileView<'_>> {
        self.vault.get(username).map(crate::ProfileView::new)
    }

    /// Adopt this vault and `play` into `core`, as an unlock does (status
    /// rows seeded, no slot spawned).
    pub fn start_session<Io>(self, core: &mut crate::OperatorSession<Io>, play: host_play::Play) {
        core.start(self.vault, play);
    }

    /// Every stored username, in vault order.
    pub fn usernames(&self) -> Vec<String> {
        self.vault
            .profiles()
            .map(|profile| profile.username.clone())
            .collect()
    }
}

/// One vault row a test stores with [`VaultFixture::put`].
pub struct FixtureProfile {
    pub(crate) profile: vault::Profile,
}

impl FixtureProfile {
    /// A row with default settings and uid 0.
    pub fn new(username: &str, password: &str) -> Self {
        Self {
            profile: vault::Profile {
                username: username.to_string(),
                password: password.into(),
                uid: 0,
                settings: vault::ProfileSettings::default(),
            },
        }
    }

    /// The account uid.
    pub fn uid(mut self, uid: i32) -> Self {
        self.profile.uid = uid;
        self
    }

    /// Whether the bot logs in when loaded.
    pub fn auto_login(mut self, auto_login: bool) -> Self {
        self.profile.settings.auto_login = auto_login;
        self
    }

    /// The preferred world.
    pub fn world(mut self, world: Option<u16>) -> Self {
        self.profile.settings.world = world;
        self
    }

    /// Low-memory mode.
    pub fn lowmem(mut self, lowmem: bool) -> Self {
        self.profile.settings.lowmem = lowmem;
        self
    }

    /// The renderer mode.
    pub fn raster(mut self, raster: crate::command::RasterMode) -> Self {
        self.profile.settings.raster = raster.to_vault();
        self
    }

    /// The settings of `draft` (its username and password are ignored).
    pub fn settings(mut self, draft: &crate::SettingsDraft) -> Self {
        self.profile.settings = draft.settings().clone();
        self
    }
}
