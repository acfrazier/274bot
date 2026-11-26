//! Shared frame mutex. This is the host mutex, including when Cargo enables
//! `host/lock-order` through another dependency.

pub use host::lock_order::Mutex;
