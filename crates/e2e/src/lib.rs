//! `e2e` — the native end-to-end harness crate.
//!
//! The crate exposes one reusable library ([`suite`]) and a live-test surface under
//! `tests/`:
//!
//!   * [`suite`] is the tracked native suite entrypoint (`e2e-suite`): a frozen manifest
//!     of reference metadata plus the native adapter map, level/`--only`/`smart`
//!     selection, an identity-bound resumable ledger, terminal-receipt and capture
//!     validation, and process ownership for the launched native executables.
//!   * `tests/**` are the `LIVE=1`-gated gameplay tests (unchanged by the suite work).
//!
//! The suite never re-implements a scenario engine, never runs the foreign JavaScript
//! runtime, and never claims qualification: a green child is a recorded observation that
//! a human still has to read back.

pub mod suite;
