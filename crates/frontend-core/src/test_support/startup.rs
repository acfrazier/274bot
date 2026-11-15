//! Startup fixtures for front-end tests (see the parent module): a legacy
//! preferences source the state database cannot import, and the proof that
//! the stores beside the refused preferences keep working (T5.3 plan §5.1:
//! a source problem refuses the preferences alone, with its path, cause and
//! remedy, and blocks neither the start nor any other store).

use std::path::Path;
use std::time::{Duration, Instant};

use api::identity::{DurableTimestamp, Settlement};
use api::local_state::{
    AccountKey, BankHint, ContentScope, DomainEdit, DomainIdentity, ErrorCode, LocalStateDomain,
    LocalStatePort, SubmitReceipt,
};
use host_play::local_state::RequestState;

use crate::OperatorSession;

/// What a test puts at the legacy `panel-ui.json` so that no read of it can
/// succeed: the two kinds the startup probe used to stop the whole start for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadPreferencesSource {
    /// A directory where the file belongs.
    Directory,
    /// A file this process may not read (mode 000).
    Unreadable,
}

impl BadPreferencesSource {
    /// Every kind.
    pub const ALL: [Self; 2] = [Self::Directory, Self::Unreadable];

    /// A short name for assertion messages and scratch directories.
    pub fn label(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Unreadable => "unreadable",
        }
    }

    /// Put this kind of bad source at `path`, making its parents. `false`
    /// when this process reads the path anyway (root reads a mode-000 file,
    /// and a platform without modes has none): that kind cannot be made
    /// here, and the test skips it.
    pub fn make(self, path: &Path) -> bool {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the state root is made");
        }
        match self {
            Self::Directory => {
                std::fs::create_dir(path).expect("the directory is made");
                true
            }
            Self::Unreadable => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::write(path, br#"{"last_focus":"alice"}"#).expect("the file is made");
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))
                        .expect("the file is made unreadable");
                    std::fs::File::open(path).is_err()
                }
                #[cfg(not(unix))]
                {
                    false
                }
            }
        }
    }

    /// The cause a refusal of this kind names.
    pub fn cause(self) -> &'static str {
        match self {
            Self::Directory => "not a regular file",
            Self::Unreadable => "Permission denied",
        }
    }

    /// Give the path back to the operator, as the remedy says: a directory
    /// removed, a file made readable again.
    pub fn repair(self, path: &Path) {
        match self {
            Self::Directory => std::fs::remove_dir(path).expect("the directory is removed"),
            Self::Unreadable => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                        .expect("the file is made readable");
                }
            }
        }
    }

    /// Assert that `text` is the preferences refusal for this bad source at
    /// `source`: the exact path, this kind's cause, that nothing was saved,
    /// and the remedy.
    pub fn assert_refusal(self, text: &str, source: &Path) {
        let path = source.display().to_string();
        for needle in [
            path.as_str(),
            self.cause(),
            "No source ledger or defaults were saved.",
            "Restore readable private preferences JSON",
            "Retry after fixing this cause.",
        ] {
            assert!(
                text.contains(needle),
                "{}: {needle:?} is missing from the refusal: {text}",
                self.label()
            );
        }
    }
}

/// Save one bank hint for `account` through `core`'s process service and
/// wait for its terminal: the stores beside a refused preferences store keep
/// working. `true` when the hint became durable.
pub fn bank_hint_saves<Io>(core: &OperatorSession<Io>, account: &str) -> bool {
    let service = core
        .local_state_for_test()
        .expect("the session's local state is bound");
    let identity = DomainIdentity::new(
        LocalStateDomain::BankHints,
        AccountKey::try_new("local-289", account).expect("a login name"),
        ContentScope::try_new(api::selected::ClientRevision::R289, "test-content")
            .expect("a content scope"),
    );
    let hint = BankHint::try_new(
        identity,
        None,
        vec![(995, 10)],
        DurableTimestamp::from_unix_seconds(1_760_000_001),
    )
    .expect("a bank hint");
    let deadline = Instant::now() + Duration::from_secs(60);
    let request = loop {
        match service.submit(DomainEdit::Bank(hint.clone())) {
            SubmitReceipt::Accepted { request } => break request,
            // The bank store hydrates on its first use.
            SubmitReceipt::Refused(ErrorCode::NotReady { .. }) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            SubmitReceipt::Refused(_) => return false,
        }
    };
    matches!(
        service.wait_request(request, Duration::from_secs(60)),
        RequestState::Settled(Settlement::Completed(_))
    )
}
