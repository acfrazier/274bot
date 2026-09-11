# Shared headed catalog-core witness

## Scope

The catalog-boundary full-core proof now has one implementation in `host_play::catalog_core`. The existing `catalog_boundary_live` consumer imports that module, and the dedicated visible `panel/examples/catalog_watch` path opts into the same witness. Ordinary panel, host-play, slot, and scenario execution leaves the witness disabled.

No gameplay action API, second driver, scenario preparation, client, engine, or fixture behavior changed. No LIVE run was made; visible LIVE proof remains root-owned.

## Producer and Start ordering

The production slot publisher calls `CoreWatch::observe_snapshot` from the already-built per-slot `GameSnapshot`, immediately after `host::publish_snapshot` and before the panel slot-frame callback. The callback's deferred catalog start calls `begin_start` synchronously before `ScriptStartHandle::start_load`. Therefore the witness freezes the most recent prepared scene-2 observation before the isolate Start/first action can occur. There is no second baseline capture.

A pre-Start login/session boundary clears the candidate observation and keeps the watcher ready. `begin_start` then refuses until a fresh qualified observation from the current session arrives. Any boundary after Start is terminal failure, so snapshots from different sessions cannot be combined.

The watcher is configured only by the catalog-watch opt-in, requires one driven slot, and validates the configured fresh account and every existing case-specific start predicate.

## Terminal decision and evidence

Per-frame UI polling evaluates only `CoreWitness::qualified`; it does not construct witness JSON. The first qualification caches one bounded immutable `Arc<Value>` receipt in a terminal watcher state, and later status/receipt reads reuse it. This terminal state also makes the status/qualify handoff atomic with respect to producer updates. Timeout handles either a concurrently reached qualified receipt or the current failure normally; it has no stale-status assertion or panic path.

Existing scenario evidence output remains unchanged. On catalog-watch PASS or FAIL, compact core evidence is emitted as a separate additive `CATALOG_CORE:` record only after terminal-shot drain. Existing shot/snapshot capture ownership and serialization were not broadened because moving the full capture path safely would exceed this proof-wiring scope; the scenario receipt and its screenshot/full-snapshot behavior remain intact.

## Predicate preservation

`docs/compat/evidence/shared-headed-core/normalization.json` compares complete function blocks from `HEAD:crates/host-play/tests/catalog_boundary_live.rs` with the extracted module after stripping visibility modifiers and whitespace. It finds 128 exact normalized shared bodies. The unmatched extracted blocks are the deliberate snapshot adapter, qualification predicate/receipt split, and watcher lifecycle bridge. The existing catalog suite now executes against the shared types and passes all 49 non-LIVE tests, including every case-specific cycle observer and receipt regression.

## Verification

Source identity before commit:

- base HEAD: `4a06347ce0f52de5c6612838514b08e1d90c83fd`
- `crates/host-play/src/catalog_core.rs`: `3d282870d153b1a4d5d09f7106c0104394219a5c`
- `crates/host-play/tests/catalog_core_watch.rs`: `5048ba0970faf99fcafb3c5088ca1dd7657e8424`
- `crates/host-play/tests/catalog_boundary_live.rs`: `857c189816413d19b7c4d5286a94ac9206cc78c3`
- `crates/panel/src/app.rs`: `e7087f7d8ab05b0cf2176595990aa4c9f2e41082`
- `crates/panel/src/session.rs`: `b27ac38caab5797b92e172fb410236d10a1cbc75`
- `crates/panel/examples/catalog_watch.rs`: `dd5f3fc251e8afe2d574fcea776014656c1da8c8`

Commands and outcomes:

- `cargo test --locked --offline -p host-play --features memory-profile --test catalog_boundary_live -- --test-threads=1`: 49 passed, 1 LIVE test ignored.
- `cargo test --locked --offline -p host-play --test catalog_core_watch -- --test-threads=1`: 3 passed.
- `cargo test --locked --offline -p host-play --lib -- --test-threads=1`: 146 passed.
- `cargo test --locked --offline -p panel --lib -- --test-threads=1`: 401 passed.
- `cargo clippy --locked --offline -p host-play -p panel --all-targets --features memory-profile -- -D warnings`: passed.
- `git diff --check`: passed.

Focused bridge coverage rejects a wrong account, invalidates a candidate across initial login, requires a fresh current-session preparation observation, freezes it before the actual Start closure, rejects scenario-only PASS, reports timeout as FAIL, requires post-Start core progress, makes post-Start boundaries terminal, and accepts/caches a real full-core receipt.

## Follow-on seam

`CoreWatch` is cloneable and the producer hook is already per slot, but this card deliberately enforces one driven catalog-watch slot. A future paired proof can compose one watcher/qualifier per driven slot (or a bounded multi-slot coordinator) without duplicating `Observation`, `CoreCase`, or cycle predicates. Paired acceptance is not claimed here.
