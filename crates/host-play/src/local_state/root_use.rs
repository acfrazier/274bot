//! The root-use lease (T5.3 §4.3): every new-version process that uses a
//! state root's database holds `<root>/instance.lock` **shared** for its
//! lifetime; an explicit maintenance command (same-root export, recovery)
//! takes it **exclusive** without waiting, so it never runs beside a user
//! of the same root.
//!
//! A frontend whose instance permit already holds the lock
//! ([`crate::InstanceLock`]) hands the same open file to the service, so a
//! process keeps one descriptor for it. A composition without the permit
//! (a harness, Continue anyway, an explicit root) takes its own shared
//! lease when the database first opens.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::sync::Arc;

use super::files::StateRoot;

/// One holder of a root's `instance.lock`, shared or exclusive. Clones of
/// the instance permit's file share one descriptor; the lock is released
/// when the last holder drops.
#[derive(Debug)]
pub struct RootUse {
    file: Arc<File>,
}

impl RootUse {
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
