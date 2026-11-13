//! Build-time repository identities shared by the frontends and live harness.
//!
//! Git metadata, not tracked source files, invalidates this stamp. Unstaged edits
//! can leave the dirty flag stale until another watched input changes.

use serde::Serialize;

/// Source identity captured while compiling one repository.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SourceIdentity {
    /// Full commit identifier, or `"unknown"` when Git identity is unavailable.
    pub commit: &'static str,
    /// Whether the repository had a dirty worktree when the stamp was built.
    pub worktree_dirty: bool,
}

/// Identity of the 274bot host repository at build time.
pub const HOST: SourceIdentity = SourceIdentity {
    commit: env!("STAMP_HOST_COMMIT"),
    worktree_dirty: matches!(env!("STAMP_HOST_DIRTY").as_bytes(), b"1"),
};

/// Identity of the checked-out `vendor/fr-client-rust` repository at build time.
pub const CLIENT: SourceIdentity = SourceIdentity {
    commit: env!("STAMP_CLIENT_COMMIT"),
    worktree_dirty: matches!(env!("STAMP_CLIENT_DIRTY").as_bytes(), b"1"),
};

/// Full host commit identifier used by the panel's existing build label.
pub const COMMIT: &str = HOST.commit;
/// Seven-character host commit identifier, or `"unknown"`.
pub const SHORT: &str = env!("STAMP_HOST_SHORT");
/// Whether the host worktree was dirty when the stamp was built.
pub const DIRTY: bool = HOST.worktree_dirty;
/// ISO-8601 UTC timestamp when the stamp was built, possibly empty.
pub const BUILT_AT: &str = env!("BUILD_TIME");
