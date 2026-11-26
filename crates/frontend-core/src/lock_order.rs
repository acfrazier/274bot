//! Lock-order feature alias for front-end code.
//!
//! Off by default, and in every release build that does not pass
//! `--features lock-order`: [`Mutex`] is [`std::sync::Mutex`].

#[cfg(feature = "lock-order")]
pub use host::lock_order::TrackingMutex as Mutex;

#[cfg(not(feature = "lock-order"))]
pub use std::sync::Mutex;
