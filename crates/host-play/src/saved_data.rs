//! One startup probe and move-aside decision for saved files with a known
//! shape older than the 0.2.8 format. The probe deliberately recognizes
//! shapes, not writer releases or values that merely resemble an older
//! default. A shape 0.2.8 read without asking the user is in the 0.2.8
//! format, whichever release wrote it, and is never refused here.

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use vault::{read_private_file, VaultError};

pub use crate::local_state::StateRoot;
use crate::local_state::{hint_has_pre_0_2_8_shape, ServerRosterError, MAX_HINT_BYTES};

/// Which response the frontend got to its single saved-data question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedDataChoice {
    /// Move every recognized old-shape file aside before starting normally.
    Rename,
    /// Leave every file untouched and stop startup.
    Keep,
}

/// One complete set of files refused as older saved-data formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedDataRefusal {
    paths: Vec<PathBuf>,
}

impl SavedDataRefusal {
    /// A one-file refusal from a vault's post-decryption format check.
    pub fn from_vault_error(error: &VaultError) -> Option<Self> {
        match error {
            VaultError::UnsupportedSavedFormat { path } => Some(Self {
                paths: vec![path.clone()],
            }),
            _ => None,
        }
    }

    /// Every refused file, in stable path order.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Resolve this refusal after the frontend's one explicit choice.
    pub fn resolve(self, choice: SavedDataChoice) -> Result<(), SavedDataResolveError> {
        match choice {
            SavedDataChoice::Keep => Err(SavedDataResolveError::Declined(self)),
            SavedDataChoice::Rename => {
                for path in &self.paths {
                    if let Err(source) = move_aside(path) {
                        let path = path.clone();
                        return Err(SavedDataResolveError::Move {
                            refusal: self,
                            path,
                            source,
                        });
                    }
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for SavedDataRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = api::local_state::FailureReason::UnsupportedSavedFormat;
        for (index, path) in self.paths.iter().enumerate() {
            if index != 0 {
                f.write_str("\n")?;
            }
            write!(f, "{}: {reason}", path.display())?;
        }
        Ok(())
    }
}

/// A shallow startup snapshot. It contains only paths that must be resolved;
/// current store contents continue through their existing readers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedDataStartup {
    refusal: Option<SavedDataRefusal>,
}

impl SavedDataStartup {
    /// Check the resolved state root for legacy formats that need one
    /// startup refusal decision.
    ///
    /// Script settings use the root-wide local-state service. Its importer
    /// owns their readiness and errors. Loadouts are checked early for
    /// ordinary read failures. Bank hints have older shapes that this step
    /// can identify. With no database or server source, it can also refuse
    /// retired worlds before starting any importer. Otherwise the roster
    /// importer checks whether a committed SQL roster supersedes them.
    ///
    /// Preferences are not probed. They live in the state database, and the
    /// legacy `panel-ui.json` is only an import source that its writer reads.
    /// A source it cannot read refuses preferences on their own error
    /// surface. Startup and every other store continue.
    pub fn check(root: &StateRoot) -> Result<Self, SavedDataCheckError> {
        let mut refusal_paths = Vec::new();
        let mut first_error = None;

        if matches!(root.database_file().try_exists(), Ok(false))
            && matches!(root.path().join("servers.json").try_exists(), Ok(false))
        {
            let worlds = root.path().join("worlds.json");
            match probe_readable_file(&worlds) {
                Ok(true) => refusal_paths.push(worlds),
                Ok(false) => {}
                Err(error) => remember_error(&mut first_error, error),
            }
        }

        if let Err(error) = probe_readable_file(&root.loadouts_file()) {
            remember_error(&mut first_error, error);
        }
        scan_hints(root.path(), &mut refusal_paths, &mut first_error);

        if let Some(error) = first_error {
            return Err(error);
        }

        refusal_paths.sort();
        refusal_paths.dedup();
        Ok(Self {
            refusal: (!refusal_paths.is_empty()).then_some(SavedDataRefusal {
                paths: refusal_paths,
            }),
        })
    }

    /// Add the legacy world file refused by the roster importer, if any.
    pub fn include_server_roster_error(&mut self, error: &ServerRosterError) {
        let ServerRosterError::UnsupportedSavedFormat { path } = error else {
            return;
        };
        let paths = self
            .refusal
            .take()
            .map_or_else(Vec::new, |refusal| refusal.paths);
        let mut paths = paths;
        paths.push(path.clone());
        paths.sort();
        paths.dedup();
        self.refusal = Some(SavedDataRefusal { paths });
    }

    /// The complete old-shape refusal, if the probe found one.
    pub fn refusal(&self) -> Option<&SavedDataRefusal> {
        self.refusal.as_ref()
    }

    /// Apply the frontend's single decision, if any file was refused.
    pub fn resolve(self, choice: SavedDataChoice) -> Result<(), SavedDataResolveError> {
        match self.refusal {
            Some(refusal) => refusal.resolve(choice),
            None => Ok(()),
        }
    }
}

/// An I/O failure while probing saved data; it is not an old-format refusal.
#[derive(Debug, thiserror::Error)]
pub enum SavedDataCheckError {
    #[error("{path}: {source}", path = .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path}: not a regular file", path = .path.display())]
    NotRegularFile { path: PathBuf },
    #[error("{path}: could not enumerate saved bank hints: {source}", path = .path.display())]
    ReadDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn remember_error(first: &mut Option<SavedDataCheckError>, error: SavedDataCheckError) {
    if first.is_none() {
        *first = Some(error);
    }
}

fn probe_readable_file(path: &Path) -> Result<bool, SavedDataCheckError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(SavedDataCheckError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !file
        .metadata()
        .map_err(|source| SavedDataCheckError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .is_file()
    {
        return Err(SavedDataCheckError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    Ok(true)
}

fn scan_hints(
    root: &Path,
    refusal_paths: &mut Vec<PathBuf>,
    first_error: &mut Option<SavedDataCheckError>,
) {
    let hints_root = root.join("bank-hints");
    let profiles = match fs::read_dir(&hints_root) {
        Ok(profiles) => profiles,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(source) => {
            remember_error(
                first_error,
                SavedDataCheckError::ReadDir {
                    path: hints_root,
                    source,
                },
            );
            return;
        }
    };

    for profile in profiles {
        let profile = match profile {
            Ok(profile) => profile,
            Err(source) => {
                remember_error(
                    first_error,
                    SavedDataCheckError::ReadDir {
                        path: hints_root.clone(),
                        source,
                    },
                );
                continue;
            }
        };
        let profile_path = profile.path();
        match profile.file_type() {
            Ok(kind) if kind.is_dir() => {}
            Ok(_) => continue,
            Err(source) => {
                remember_error(
                    first_error,
                    SavedDataCheckError::Io {
                        path: profile_path,
                        source,
                    },
                );
                continue;
            }
        }

        let accounts = match fs::read_dir(&profile_path) {
            Ok(accounts) => accounts,
            Err(source) => {
                remember_error(
                    first_error,
                    SavedDataCheckError::ReadDir {
                        path: profile_path,
                        source,
                    },
                );
                continue;
            }
        };
        for account in accounts {
            let account = match account {
                Ok(account) => account,
                Err(source) => {
                    remember_error(
                        first_error,
                        SavedDataCheckError::ReadDir {
                            path: profile_path.clone(),
                            source,
                        },
                    );
                    continue;
                }
            };
            let path = account.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            match account.file_type() {
                Ok(kind) if kind.is_file() => {}
                Ok(_) => continue,
                Err(source) => {
                    remember_error(first_error, SavedDataCheckError::Io { path, source });
                    continue;
                }
            }
            match read_private_file(&path, MAX_HINT_BYTES) {
                Ok(bytes) => {
                    if hint_has_pre_0_2_8_shape(&bytes) {
                        refusal_paths.push(path);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    remember_error(first_error, SavedDataCheckError::Io { path, source })
                }
            }
        }
    }
}

/// The frontend declined the one saved-data prompt, or moving a refused file
/// aside failed after a choice to rename.
#[derive(Debug, thiserror::Error)]
pub enum SavedDataResolveError {
    #[error("{0}")]
    Declined(SavedDataRefusal),
    #[error("could not move {path} aside: {source}", path = .path.display())]
    Move {
        refusal: SavedDataRefusal,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl SavedDataResolveError {
    /// The complete refusal set associated with this result.
    pub fn refusal(&self) -> &SavedDataRefusal {
        match self {
            Self::Declined(refusal) | Self::Move { refusal, .. } => refusal,
        }
    }
}

fn move_aside(path: &Path) -> io::Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "saved-data path has no file name",
        )
    })?;
    let mut base_name = file_name.to_os_string();
    base_name.push(".pre-0.2.8");
    let mut candidate = parent.join(&base_name);
    let mut suffix = 1_u64;

    loop {
        match fs::hard_link(path, &candidate) {
            Ok(()) => {
                if let Err(error) = fs::remove_file(path) {
                    let _ = fs::remove_file(&candidate);
                    return Err(error);
                }
                api::host_log!(
                    api::hostlog::Category::Lifecycle,
                    api::hostlog::Level::Info,
                    "saved data moved aside from {} to {}",
                    path.display(),
                    candidate.display()
                );
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let mut name = base_name.clone();
                name.push(format!(".{suffix}"));
                candidate = parent.join(name);
                suffix = suffix.checked_add(1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "no saved-data backup name remains",
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::Servers;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let path = std::env::temp_dir().join(format!(
                "saved-data-startup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn root(&self) -> StateRoot {
            StateRoot::new(self.0.clone())
        }

        fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            vault::write_private_file(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn current_0_2_8_store_shapes_are_not_refused() {
        let dir = Scratch::new();
        dir.write("panel-ui.json", br#"{"capture":false}"#);
        dir.write(
            "script-settings.json",
            br#"{"catalog:ChickenKiller":{"buryBones":false}}"#,
        );
        dir.write(
            "loadouts.json",
            br#"[{"name":"current","worn":{"hat":"Helm"},"carry":[{"item":"Shark","qty":2}],"unassigned":[]}]"#,
        );
        dir.write(
            "bank-hints/alice/alice.json",
            br#"{"schema_version":1,"profile":"alice","account":"alice","revision":289,"content":"alice","observed_at_unix":1,"rows":[]}"#,
        );
        let mut two_worlds = Servers::builtins();
        two_worlds.servers[0].worlds.truncate(2);
        let servers = serde_json::to_vec(&two_worlds).unwrap();
        dir.write("servers.json", &servers);
        dir.write("worlds.json", br#"{"worlds":[]}"#);

        let startup = SavedDataStartup::check(&dir.root()).unwrap();
        assert!(startup.refusal().is_none());
    }
    #[test]
    fn script_settings_are_not_probed_at_startup() {
        let dir = Scratch::new();
        fs::create_dir(dir.0.join("script-settings.json")).unwrap();

        let startup = SavedDataStartup::check(&dir.root())
            .expect("script settings readiness belongs to its service");
        assert!(startup.refusal().is_none());
    }

    /// The legacy `panel-ui.json` is an import source of the state database,
    /// not a store this probe opens (plan §5.1, §5.5): a directory at its
    /// path, or a file the process may not read, refuses the preferences
    /// alone (the database import says so with the path, cause and remedy)
    /// and leaves the start, and every other store, going on.
    #[test]
    fn a_directory_or_unreadable_preferences_source_does_not_stop_the_start() {
        let dir = Scratch::new();
        dir.write(
            "bank-hints/alice/alice.json",
            br#"{"schema_version":1,"profile":"alice","account":"alice","revision":289,"content":"alice","observed_at_unix":1,"rows":[]}"#,
        );
        let source = dir.0.join("panel-ui.json");

        fs::create_dir(&source).unwrap();
        let startup = SavedDataStartup::check(&dir.root())
            .expect("a directory at the preferences source is not a startup failure");
        assert_eq!(startup.refusal(), None);
        fs::remove_dir(&source).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            dir.write("panel-ui.json", br#"{"last_focus":"alice"}"#);
            fs::set_permissions(&source, fs::Permissions::from_mode(0o000)).unwrap();
            // A process that may read a mode-000 file anyway (root) cannot
            // make this case; the directory case above still holds.
            if File::open(&source).is_err() {
                let startup = SavedDataStartup::check(&dir.root())
                    .expect("an unreadable preferences source is not a startup failure");
                assert_eq!(startup.refusal(), None);
            }
            fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    /// Shapes older than what 0.2.8 wrote, which 0.2.8 still read without
    /// asking the user: a schema 1 hint without a scope, and loadout rows
    /// with an array `worn` or string `carry` entries. A 0.2.8 user can
    /// still have them on disk, so the probe never offers to move them.
    #[test]
    fn older_shapes_0_2_8_read_as_its_own_are_not_refused() {
        let dir = Scratch::new();
        dir.write(
            "loadouts.json",
            br#"[{"name":"old","worn":["Helm"],"carry":["Shark"]},{"name":"current","worn":{"hat":"Helm"},"carry":[{"item":"Shark","qty":2}]}]"#,
        );
        dir.write(
            "bank-hints/local-289/alice.json",
            br#"{"schema_version":1,"profile":"local-289","account":"alice","observed_at_unix":1,"rows":[[995,3]]}"#,
        );
        dir.write(
            "bank-hints/local-274/bob.json",
            br#"{"schema_version":1,"profile":"local-274","account":"bob","observed_at_unix":1,"rows":[]}"#,
        );
        dir.write(
            "servers.json",
            &serde_json::to_vec(&Servers::builtins()).unwrap(),
        );

        let startup = SavedDataStartup::check(&dir.root()).unwrap();
        assert_eq!(startup.refusal(), None);
    }

    #[test]
    fn retired_worlds_are_reported_by_the_importer_when_the_database_exists() {
        let dir = Scratch::new();
        let worlds = dir.write("worlds.json", br#"{"worlds":[]}"#);
        let service = crate::local_state::LocalStateService::open_process(dir.root());
        let error = service
            .wait_server_roster(std::time::Duration::from_secs(5))
            .err()
            .expect("the SQL roster must refuse standalone worlds");

        let mut startup = SavedDataStartup::check(&dir.root()).unwrap();
        assert!(startup.refusal().is_none());
        startup.include_server_roster_error(&error);

        assert_eq!(
            startup.refusal().unwrap().paths(),
            std::slice::from_ref(&worlds)
        );
        startup.resolve(SavedDataChoice::Rename).unwrap();
        assert!(!worlds.exists());
    }

    #[test]
    fn keep_returns_the_complete_refusal_without_changing_any_file() {
        let dir = Scratch::new();
        let hint = dir.write(
            "bank-hints/alice/alice.json",
            br#"{"profile":"alice","account":"alice","observed_at_unix":1,"rows":[]}"#,
        );
        let worlds = dir.write("worlds.json", br#"{"worlds":[]}"#);
        let originals = [
            (hint.clone(), fs::read(&hint).unwrap()),
            (worlds.clone(), fs::read(&worlds).unwrap()),
        ];

        let startup = SavedDataStartup::check(&dir.root()).unwrap();
        let refusal = startup.refusal().unwrap();
        let mut expected = vec![hint.clone(), worlds.clone()];
        expected.sort();
        assert_eq!(refusal.paths(), expected.as_slice());
        let error = startup.resolve(SavedDataChoice::Keep).unwrap_err();
        assert!(matches!(error, SavedDataResolveError::Declined(_)));
        assert_eq!(error.refusal().paths(), expected.as_slice());
        for (path, bytes) in originals {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn rename_moves_every_refused_file_without_clobbering_collisions() {
        let dir = Scratch::new();
        let hint = dir.write(
            "bank-hints/alice/alice.json",
            br#"{"profile":"alice","account":"alice","observed_at_unix":1,"rows":[]}"#,
        );
        let worlds = dir.write("worlds.json", br#"{"worlds":[]}"#);
        let old_hint = fs::read(&hint).unwrap();
        let old_worlds = fs::read(&worlds).unwrap();
        let collision = dir.write("worlds.json.pre-0.2.8", b"keep this existing file");

        SavedDataStartup::check(&dir.root())
            .unwrap()
            .resolve(SavedDataChoice::Rename)
            .unwrap();

        assert!(!hint.exists());
        assert_eq!(
            fs::read(dir.0.join("bank-hints/alice/alice.json.pre-0.2.8")).unwrap(),
            old_hint
        );
        assert!(!worlds.exists());
        assert_eq!(fs::read(collision).unwrap(), b"keep this existing file");
        assert_eq!(
            fs::read(dir.0.join("worlds.json.pre-0.2.8.1")).unwrap(),
            old_worlds
        );
    }

    #[test]
    fn vault_refusal_uses_the_same_single_file_choice() {
        let dir = Scratch::new();
        let path = dir.0.join("vault");
        let refusal = SavedDataRefusal::from_vault_error(&VaultError::UnsupportedSavedFormat {
            path: path.clone(),
        })
        .unwrap();
        assert_eq!(refusal.paths(), std::slice::from_ref(&path));
        assert!(refusal
            .to_string()
            .contains("saved data is in a format from before 0.2.8"));
        assert!(matches!(
            refusal.resolve(SavedDataChoice::Keep),
            Err(SavedDataResolveError::Declined(_))
        ));
    }
}
