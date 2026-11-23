//! The mandatory startup permit over the local-state root's exclusive lock.

use crate::local_state::{RootUse, StateRoot, StateRootLockError};

/// The process's exclusive root lock, or an unbound library-test session.
#[derive(Debug)]
pub enum InstancePermit {
    Locked(RootUse),
    #[cfg(any(test, feature = "test-support"))]
    SkipLock,
}

impl InstancePermit {
    /// Construct an unbound library-test session, not a product startup.
    #[cfg(any(test, feature = "test-support"))]
    pub fn skip() -> Self {
        Self::SkipLock
    }

    /// Share the locked descriptor with this process's state service.
    pub fn root_use(&self) -> Option<RootUse> {
        match self {
            Self::Locked(root_use) => Some(root_use.clone()),
            #[cfg(any(test, feature = "test-support"))]
            Self::SkipLock => None,
        }
    }
}

/// Acquire the mandatory startup lock before opening any saved state.
pub fn resolve_instance_permit() -> Result<InstancePermit, StateRootLockError> {
    let root = StateRoot::new(script::bot_home().join(".274bot"));
    RootUse::acquire(&root).map(InstancePermit::Locked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_permit_holds_an_exclusive_lock() {
        let _iso = script::IsolatedEnv::enter("instance-permit");
        let permit = resolve_instance_permit().unwrap();
        assert!(matches!(permit, InstancePermit::Locked(_)));
        let root = StateRoot::new(script::bot_home().join(".274bot"));
        assert!(RootUse::shared(&root).unwrap().is_none());
        assert!(matches!(
            resolve_instance_permit(),
            Err(StateRootLockError::InUse { .. })
        ));
    }

    #[test]
    fn lock_is_reacquired_after_the_holder_drops() {
        let _iso = script::IsolatedEnv::enter("instance-lock-drop");
        let first = resolve_instance_permit().unwrap();
        drop(first);
        let _second = resolve_instance_permit().expect("the first permit released the lock");
    }

    #[test]
    fn service_lease_keeps_the_lock_after_the_permit_drops() {
        let _iso = script::IsolatedEnv::enter("instance-lock-service");
        let permit = resolve_instance_permit().unwrap();
        let lease = permit
            .root_use()
            .expect("the startup permit holds the lock");
        drop(permit);
        assert!(matches!(
            resolve_instance_permit(),
            Err(StateRootLockError::InUse { .. })
        ));
        drop(lease);
        let _next = resolve_instance_permit().expect("the last lease released the lock");
    }

    #[cfg(unix)]
    #[test]
    fn startup_lock_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let _iso = script::IsolatedEnv::enter("instance-lock-mode");
        let _permit = resolve_instance_permit().unwrap();
        let root = StateRoot::new(script::bot_home().join(".274bot"));
        let permissions = std::fs::metadata(root.instance_lock_file())
            .unwrap()
            .permissions();
        assert_eq!(permissions.mode() & 0o077, 0);
    }
}
