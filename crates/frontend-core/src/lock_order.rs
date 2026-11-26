//! Shared frame mutex. Wraps `host::lock_order::Mutex` so Cargo's additive
//! `host/lock-order` feature applies uniformly without a cfg-split public
//! re-export.

use std::fmt;
use std::sync::{PoisonError, TryLockError};

#[repr(transparent)]
pub struct Mutex<T>(host::lock_order::Mutex<T>);

impl<T> Mutex<T> {
    pub fn new(value: T) -> Self {
        Self(host::lock_order::Mutex::new(value))
    }

    pub fn lock(
        &self,
    ) -> Result<host::lock_order::MutexGuard<'_, T>, PoisonError<host::lock_order::MutexGuard<'_, T>>>
    {
        self.0.lock()
    }

    pub fn try_lock(
        &self,
    ) -> Result<
        host::lock_order::MutexGuard<'_, T>,
        TryLockError<host::lock_order::MutexGuard<'_, T>>,
    > {
        self.0.try_lock()
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mutex").finish_non_exhaustive()
    }
}
