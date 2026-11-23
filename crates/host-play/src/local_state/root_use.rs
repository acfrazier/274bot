//! A product startup holds `<root>/instance.lock` exclusively before it
//! opens saved state. The OS releases the lock when the process exits.
//! The state service shares the frontend's locked descriptor.
//!
//! Shared leases remain for library compositions and contention tests.
//! Maintenance takes an independent exclusive lock and never waits.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::sync::Arc;

use super::files::StateRoot;

/// A state root cannot be opened by this process.
#[derive(Debug, thiserror::Error)]
pub enum StateRootLockError {
    #[error("Another 274bot process is using state root {}. Close that process before starting 274bot.", root.display())]
    InUse { root: std::path::PathBuf },
    #[error("Cannot lock 274bot state root {}: {source}", root.display())]
    Io {
        root: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
}

/// One holder of a root's `instance.lock`, shared or exclusive. Clones of
/// the instance permit's file share one descriptor; the lock is released
/// when the last holder drops.
#[derive(Debug)]
pub struct RootUse {
    file: Arc<File>,
}

impl RootUse {
    /// Hold the root exclusively before any startup reads or writes.
    pub fn acquire(root: &StateRoot) -> Result<Self, StateRootLockError> {
        Self::exclusive(root)
            .map_err(|source| StateRootLockError::Io {
                root: root.path().to_path_buf(),
                source,
            })?
            .ok_or_else(|| StateRootLockError::InUse {
                root: root.path().to_path_buf(),
            })
    }

    /// The lease over an already locked instance file.
    pub(crate) fn from_locked(file: Arc<File>) -> Self {
        Self { file }
    }

    /// Take `root`'s lease shared, without waiting: `Ok(None)` while a
    /// maintenance command holds it exclusive.
    pub fn shared(root: &StateRoot) -> io::Result<Option<Self>> {
        let file = open(root)?;
        match file.try_lock_shared() {
            Ok(()) => Ok(Some(Self {
                file: Arc::new(file),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// Take `root`'s lease exclusive, without waiting: `Ok(None)` while any
    /// other process (or another lease of this one) uses the root.
    pub fn exclusive(root: &StateRoot) -> io::Result<Option<Self>> {
        let file = open(root)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                file: Arc::new(file),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for RootUse {
    fn drop(&mut self) {
        // The last holder releases the lock explicitly: a child that
        // briefly inherited the descriptor must not keep the root in use.
        if Arc::strong_count(&self.file) == 1 {
            let _ = self.file.unlock();
        }
    }
}

fn open(root: &StateRoot) -> io::Result<File> {
    vault::create_private_dir(root.path())?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(root.instance_lock_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_lock_io_error_preserves_its_root_and_source() {
        let scratch = script::IsolatedEnv::enter("root-lock-io");
        let path = scratch.home.join("not-a-directory");
        std::fs::write(&path, b"keep").unwrap();
        let root = StateRoot::new(path.clone());
        let error = RootUse::acquire(&root).unwrap_err();
        assert!(matches!(
            error,
            StateRootLockError::Io { root, source } if root == path && source.kind() != io::ErrorKind::WouldBlock
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }

    #[test]
    fn shared_leases_coexist_and_exclude_maintenance_until_the_last_one_drops() {
        let scratch = script::IsolatedEnv::enter("root-use");
        let root = StateRoot::new(scratch.home.join(".274bot"));
        let first = RootUse::shared(&root).unwrap().expect("a free root");
        let second = RootUse::shared(&root)
            .unwrap()
            .expect("shared leases coexist");
        assert!(RootUse::exclusive(&root).unwrap().is_none());
        drop(first);
        assert!(RootUse::exclusive(&root).unwrap().is_none());
        drop(second);
        let maintenance = RootUse::exclusive(&root).unwrap().expect("no user left");
        assert!(RootUse::shared(&root).unwrap().is_none());
        drop(maintenance);
        assert!(RootUse::shared(&root).unwrap().is_some());
    }
}

#[cfg(test)]
#[path = "root_use_process_tests.rs"]
mod process_tests;
