# Load bundled navigation using its build-time identity

Implement with profile grok46 defaults after design t_66955ad4. Read AGENTS.md,
docs/execution.md, operator clarification in brief53 and accepted design
06h-navigation-validation-reuse-design.md (09f26d0a). The approved behavior is
build/package-time SHA-256 for shipped navpacks, zero full-file startup hashes
for bundled release assets, one runtime validation for custom/external packs,
and one immutable shared decoded world per process. Do not silently fall back
to the superseded requirement to rehash nav at bind/load/pre-Play.

This is authorized source groundwork now; no shipping, signing, main merge,
remote updates, release tag or final package publication. Keep the checked-in
bundled identity table empty until actual release assets are assembled. Build
fixture coverage for the nonempty table path without claiming a released app.
A mere --release build or user-authored manifest must not activate bundle trust.

Own host-play/src/profile.rs plus a small origin/identity helper if useful,
the SharedClientTemplate loading/validation portion of host-play/src/lib.rs,
host-play/src/progress.rs and relevant profile tests; nav/src/world.rs and
minimal pack/manifest helpers/tests; panel/src/picker.rs, its focused tests,
the profile-to-picker install call in session.rs, and the loading-banner
presentation/tests in app.rs. Add bundled-nav-identities.json and minimal
supporting Cargo/build metadata only if needed. Own concise report
06i-bundled-navigation-loading.md. Do not edit other capabilities, scenarios,
source catalogs, generated game facts, rendering/backend code or client source.

Concurrent owners: t_90e850ab owns host-play interact dispatcher, script useOn
and its tests; loadout t_3737503d owns loadout UI/session fields and API/script
provisioning; t_3317197c owns scenario/proof/catalog_boundary_live. Keep your
host-play/app/session edits within the named loading and picker seams. Never
restore/stash peers' WIP for builds; export exact committed host/client source.

Apply the accepted minimal design:
1. NavWorld::from_bytes decodes the bytes provided, including the legacy grid
   fallback, without rereading a pathname. load_pack uses one read + from_bytes.
2. Explicit trusted bundled identity rows compiled into host-play bind revision,
   cache identity, format, nav hash and install-relative pack path. Resolve an
   actual install resource root across native Mac app / ordinary binary layouts;
   keep a testable resolver. CLI and environment path overrides select external
   validation. Empty table preserves development behavior. Validate row shape /
   containment and exact profile/cache/header compatibility; no user trust UI.
3. Bundle startup reads/decodes once and uses the recorded identity without
   hashing the pack. External loads hash the SAME bytes they decode once, check
   the existing nav manifest/revision/cache, then keep the resulting Arc world.
   Preserve bad-version/corrupt/missing behavior and Legacy274 where applicable.
4. ServerProfile/template share that loaded world; subsequent template creation,
   pre-Play and slots do not reread or rehash navigation. Existing cache asset
   checks and UI-thread/generation/validated-handoff ownership stay intact.
   A post-load disk edit cannot replace the running world; fresh explicit bind
   validates a new pack. Do not retain a second raw pack buffer after decode.
5. Flags remain optional, lazy and paint-only. Do not read/hash 261MB flags at
   ordinary startup. On paint-on, validate exact bytes against the selected
   identity when supplied, then decode with geometry checks; wrong/unknown
   identity cannot quietly apply foreign flags. Preserve fallback to walk-word
   paint and drop when toggles turn off, with honest diagnostic state. No eager
   raw buffer, per-bot copy or per-frame world cloning. User package plan omits
   the flags file; implementation must not create a release package now.
6. Text progress stays #FFB000/20 cells/generation-safe and based on actual work.
   Bundle reads say Loading navigation; custom validation gets one clear
   Verifying custom navigation pass. Eliminate the same per-file repeated nav
   bar; no fabricated time-based percentages. Small cache validation can remain.

Meaningful verification: trusted fixture bundle zero nav hash passes / one
read-decode and N=2 Arc identity; external known wrong hash/revision/cache or
corrupt/version bytes rejected; external override defeats bundle selection;
legacy/missing pack behaviors; exact bytes validated are those decoded;
post-load replacement preserves loaded world, fresh bind rejects or selects
new identity; lazy sidecar wrong hash/geometry refused and off drops memory;
startup generation/error/close behavior retained; no nav hashing in final Play.
Prefer instrumented loader callbacks/counters only where they prove actual
work, not tests that restate labels. Run affected suites/features and strict
focused checks. Native external + synthetic bundled fixture observations are
root-owned after review. No measured latency/RSS claim from removed calls alone.

Commit only owned paths with commit --only; do not unstage anyone else's index.
Request SAME-card profile reviewer review with exact commits and checks, then
STOP. Do not self-complete. Include changed old post-load-disk semantics in the
report so reviewers do not reimpose historical acceptance tests accidentally.
