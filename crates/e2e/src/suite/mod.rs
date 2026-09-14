//! Native e2e suite: one ordered, resumable entrypoint over the existing native
//! catalog/pair executables.
//!
//! The suite is a thin *driver*: it selects cases from a frozen, tracked manifest,
//! launches the existing native executables (`panel` `catalog_watch` / `pair_watch`),
//! validates their terminal receipts and captures, and owns the run ledger. It does
//! not implement a second scenario engine, a JavaScript runtime, or any gameplay
//! assertion of its own.
//!
//! Reading order:
//!   `manifest`  frozen reference metadata + native adapter map (tracked fixture)
//!   `select`    quick/full/smart/--only selection over that metadata
//!   `identity`  manifest/reference/host/client/binary/profile/settings identities
//!   `ledger`    run directory, attempt retention, resume refusal, summary
//!   `receipt`   terminal receipt + capture parsing/validation
//!   `child`     command construction, process ownership, budget and cleanup
//!   `cli`       `e2e-suite list|dry-run|run`

pub mod child;
pub mod cli;
pub mod identity;
pub mod ledger;
pub mod manifest;
pub mod receipt;
pub mod select;

/// The tracked manifest fixture. Embedded so a run never depends on a local
/// campaign path; `--manifest` overrides it for tests and one-off inspections.
pub const EMBEDDED_MANIFEST: &str = include_str!("../../fixtures/native-suite/suite-manifest.json");

/// Result alias used across the suite: the CLI reports these as one message.
pub type SuiteResult<T> = Result<T, String>;
