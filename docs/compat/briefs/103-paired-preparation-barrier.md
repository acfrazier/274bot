# Repair paired Air preparation and one shared post-preparation Start barrier

Use grok46 defaults. Own ONLY paired_catalog_live.rs and support/paired_catalog.rs
under crates/host-play/tests, docs/compat/05s-paired-preparation-barrier.md and
unique evidence/paired-preparation-barrier/. No shared fixtures/runtime/shim/
nav/client/foreign edits, no LIVE/cache cleanup.7c2eef6eb was reviewed by actual
Grok4.5 and root exact-source checks passed; first root LIVE exposed a fixture
bug. Retain earlier artifacts/report; this is a new concrete correction.

Raw docs/compat/evidence/paired-catalog-fixtures/live/r{274,289}-air-100adccc-
7c2eef6e.log: preparation180s timeout a_prep=Ready b_prep=WaitAck,
a_started=true b_started=false. Root read code: runner seeded and teleported
straight to Air ruins, then AckBank calls open_nearest_booth there. No booth
within the loaded scene, so real givebank200 is never acknowledged. Master
starts alone in Prep::Ready while runner is still preparing. Neither cell is
paired script acceptance. Fix actual preparation, do not waive loaded bank
acknowledgement or count seed as script restock.

Prepare runner bank200 at actual native Falador East booth (correct selected
world stand3013,3355), observe bank open+loaded with correct count, close and
observe close/new generation, then prepare/reposition final seeded runner
firstload25unnoted essence at Air ruins. Master talisman but noessence. Keep
preparation separate from script baseline. Use existing native controls and
seed APIs; do not invoke trade/craft inside harness. Correct refused bank sends
must remain refusals and report concrete target absence if relevant.

Require BOTH actors current ingame+scene2, prepared final positions/inventory,
modalclosed, names/partners validated, with post-preparation observations,
BEFORE either Start. Start each exactlyonce at a shared barrier. Gold clock
begins only after bothstarts; no start while otherWaitAck and no counting
preparation progression. Audit actual relog admission so stale pre-logout
scene2 cannot satisfy preparation. Use established local preparation patterns
where needed; no arbitrary sleep/timeouts changed. Explicit prep phase logs
must reflect acknowledgement, not merely a queued mutation.

Keep existing 180s prep/gold constants, identity/conservation/full-vs-partial
predicates. Duel first runs also FAIL BotHost.addTickListener; that is separate
native mapping work, do not patch imported Duel or replace observer. Common
Startbarrier correction applies to Duel as well. No change to public receipts
that turns earlierFAILintoPASS.

Meaningful focused tests for mismatched readiness/missing acknowledgement/
stale relog and onebarrier Start; actual source review of sequence. Exact
committed source+onlyownedoverlay, task-exclusive cache (previouspairedcache
was root-cleaned afterreview; may recreate a single cache), focused existing
paired tests and strict Clippy. Scoped commit, same-card reviewer then STOP.
Root owns bothrevision/cat LIVE reruns after review.
