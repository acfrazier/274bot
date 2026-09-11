# Repair paired Air preparation and one shared Start barrier

`crates/host-play/tests/paired_catalog_live.rs` and
`crates/host-play/tests/support/paired_catalog.rs` remain a standalone
ignored LIVE pair harness. This card corrects the first root7c Air
bothrev fixture bug without changing public receipts, imported Duel, or
shared runtime.

Root `r274-air-100adccc-7c2eef6e` timed out at 180s with
`a_prep=Ready b_prep=WaitAck a_started=true b_started=false`. The runner
seeded and teleported to Air ruins, then `AckBank` called
`open_nearest_booth` there. No Use-quickly booth is in that scene, so
`givebank blankrune 200` was never observed open+loaded. Master captured
baseline and started alone. Neither cell is paired script acceptance.
05r and the raw 7c2eef6e logs are retained.

## Preparation sequence

Runner bank seed is at the native Falador East stand `(3013,3355,0)`.
The harness opens an in-scene Falador East `Use-quickly` booth with
`open_booth_at`, not nearest-at-ruins. `SendResult::Refused` stays a
refusal and logs concrete target absence. Acknowledgement requires
ingame scene2, bank open+loaded, unnoted 1436 count >= 200, and the
Falador East stand. Close is observed with `bank_session_generation`.
Only then is the firstload 25 unnoted essence given and the runner
repositioned to Air ruins. Master still seeds talisman and no essence
at the ruins. Queued `givebank` is logged as `seed-queued`, not as
acknowledgement or script restock.

Relog admission requires an observed logout (`ingame` false or
`scene_state != 2`) before a later scene2+inventory-tab frame can
satisfy preparation. Stale pre-logout scene2 is not admission.

## Shared Start barrier

`Prep::Ready` captures the post-preparation baseline and waits. Start
is exactly once per actor, only from one shared barrier, and only when
both actors are current ingame scene2, at final positions/inventory,
modals closed, names/partners validated. Gold clock 180s begins only
after both Start. Start while the other is `WaitAck` is rejected. The
same barrier applies to Duel. `SCRIPT_GOLD_DEADLINE` 180s,
`SCRIPT_GOLD_WATCH_TICKS` 150, `PREP_DEADLINE` 180s, and
identity/conservation/full-vs-partial predicates are unchanged.
Duel `BotHost.addTickListener` remains separate native mapping work.

## Verification

Exact Git regular blobs of host
`fc5c9a49d9a4792251fd1c7bfdb5be4cc730c2aa` plus client
`9d090ed04957e4efc254f073cda97bc5510ca72b` plus owned overlay:
`.superpowers/review-exports/paired-preparation-t_371cf6e0`. Isolated
empty target `.superpowers/review-exports/paired-preparation-t_371cf6e0-target`
(`isolated_build=true`). Shared campaign target was not used. Overlay
sha256: live `8dd02367e6cec5df0519df32712444c6dd9c7733d6a12cb7f664ebb9b04c8e37`,
support `40f7be9277308172a40058f7ced1c1da7da464ff1c3fcee67b5ab183a260ccee`.

- `rustfmt --check --edition 2021 -- crates/host-play/tests/paired_catalog_live.rs crates/host-play/tests/support/paired_catalog.rs` — pass.
- `cargo test -p host-play --test paired_catalog_live` — 25 passed, 2 ignored
  (existing identity/conservation/full-vs-partial witnesses plus mismatched
  readiness, missing Falador East acknowledgement, stale relog, and
  one-barrier Start).
- `cargo clippy -p host-play --test paired_catalog_live --no-deps -- -D warnings` — pass.
- `cargo test -p host-play --test paired_catalog_live paired_catalog_air_live -- --exact` — 0 passed, 1 ignored.

No LIVE client or engine was launched. These checks are fixture/build
evidence, not gameplay acceptance. Root owns both-revision/catalog LIVE
reruns after review.
