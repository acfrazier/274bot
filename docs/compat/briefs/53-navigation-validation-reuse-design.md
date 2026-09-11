# Reuse validated navigation data during startup

Operator direction at 2026-09-11 01:22-01:26 UTC: seeing navigation checks four
times is confusing. They propose a content hash because a revision's world is
fixed once the nav-pack iteration is settled. Root confirmed the current check
already hashes, but hashes each pack/flags file at bind, template load, and
pre-Play validation. The user authorizes consolidating repeated validation
around the immutable loaded resource. This new bounded scope supersedes the
older startup brief's requirement to keep all three navigation rehash passes.
It does not authorize trusting a revision number, pathname or mtime as content.

Architecture review only using grok46 defaults. Read AGENTS.md, docs/execution.md,
this brief, and relevant code at an exact committed host plus gitlink. No source
edits or LIVE. Own only docs/compat/06h-navigation-validation-reuse-design.md.
Keep the report actionable and bounded (roughly 150 lines), not a re-audit of
all profile, protocol, gameplay or older campaign reports. Existing integration
review t_dde220ef continues against its own frozen source; this new change must
receive its own same-card source review and final branch review later.

Inspect exact bytes/ownership and all runtime navigation consumers:
crates/host-play/src/{profile,lib,progress}.rs,
crates/nav/src/{world,pack,manifest}.rs,
crates/panel/src/{picker,session,app}.rs and focused existing profile tests.
NavWorld::load_pack currently reads bytes again after hashing; its legacy grid
fallback rereads by path. SharedClientTemplate holds Arc<NavWorld>; slots and
picker share it. Raw 261 MB flags are paint-only and loaded lazily in picker,
then dropped when both collision toggles go off. Do not retain that whole file
or eagerly decode it to save a hash. Existing startup races are not solved by
assuming filesystem bytes cannot change between hash and decode.

Recommend the smallest correct implementation that:
- Computes the pack content identity from the exact bytes it decodes into the
  one immutable shared NavWorld, then reuses that identity/world across load,
  Play and every slot. Preserve cache/revision/nav-manifest binding, legacy274
  availability, all pack versions/errors, and missing navigation semantics.
- Eliminates repeated startup navigation reads. Assess keeping one streamed
  flags validation at bind vs validating the optional debug sidecar only on
  demand against the selected manifest. Prefer the simpler bounded design;
  explicitly state changed on-disk-after-load semantics. A ready process may
  continue using its original verified in-memory world; future processes or
  explicit reload must validate any rebuilt pack's new content identity.
- Keeps raw flags lazy and validates the same bytes that are decoded for paint;
  matching dimensions alone must not accept a foreign sidecar. Preserve weak
  existing fallback when no optional flags are available, with honest status.
- Keeps cache/client assets and protocol/startup/generation ownership unchanged
  unless one tiny supporting seam is necessary. Do not introduce global hash
  caches keyed only by path/mtime, a new resource manager, mmap dependencies,
  persistent artifact caches, relaxed stale-session checks, or per-bot copies.
- Makes progress understandable: one monotonic overall preparation display or
  clearly distinct stages, no identical per-file resets presented as a restart.
  Keep actual-work counters and ACCENT/#FFB000. No timer-based percentage.

Name concrete implementation files/seams, invariants and focused tests. Required
proof: pre-load wrong revision/hash/corruption rejected; file mutation/replacement
cannot make decoded bytes differ from the validated content; after-load file
mutation does not change the existing immutable world; new template validates
changed content; lazy debug flags reject the wrong hash; N=2 shares one world.
Root will run native progress/correctness checks on selected real packs after
source review. Record hashes/read counts if straightforward; do not claim
latency/RSS savings without a controlled comparison. Preserve historical native
changed-file failures as evidence of the previous contract, not a new gate
requiring redundant I/O forever. No alteration of product timeouts or catalog
proof. Commit only your report and complete the design card. No subdelegation.
