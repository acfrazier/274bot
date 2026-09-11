# Preserve selected loc identity through compatibility dispatch

Use grok46 defaults after autocast source/review releases shared runtime files.
Current host branch codex/rs2b0t-multirevision; root owns repository integration.
Use fail-closed-dispatch and current STATE operator ownership/stub rules.

Confirmed B1/client9d defect: foreign FlaxPicker selects ground Flax2646 at
2737,3440,0. The same snapshot also contains a nameless Wall980 at that tile.
`crates/host-play/src/lib.rs` InteractReq::Loc arm currently finds the FIRST
row matching x/z/level, then asks it for action Pick. It therefore selects Wall980
and refuses instead of dispatching the selected flax. Both co-located rows are
in the native full snapshot
`docs/compat/evidence/catalog-headed/r274-flax-picker-100adccc-b1cff8a7-gpu-diagnostic/shots/2026-09-11T06-21-11_29226/2026-09-11T06-22-46_flax_picker.json`.
The paired internal PNG was read by root. Both headless and native274 stall at6
flax; the exact source manifest/binaries are under catalog-headed/*b1cff8a7.json.
Static-loc deletion and scalar distance fixes are already source-reviewed and
integrated client9d; preserve them. This new identity loss is OUR bridge bug.
Audit card t_26092c52 is read-only and may provide additional evidence.

Trace the exact loc selected by both immutable catalogs through existing loc
wrappers, Rust request serialization and host dispatch. Preserve selected ID
and enough native identity/layer facts to avoid selecting a different co-located
loc. Validate against the CURRENT native snapshot at dispatch; refuse stale,
changed/missing identity or absent action. Do not select another loc merely
because it shares coordinates/name/action. Keep the bridge thin and preserve
supported existing native/legacy calls and operation order. Do not patch foreign
catalogs, introduce JS loc policy/pathfinding, change native timeouts, widen
foreign import gates, or clone the world. Do not hide the bug by dimming Flax.
A raw forged identity must not bypass current-snapshot or action validation.

Own only minimal script shim/request files, selected-loc host dispatch in
host-play/src/lib.rs, meaningful focused regressions in unique/scoped tests,
plus docs/compat/04i-loc-selected-identity.md and unique evidence/loc-selected-identity/.
No API/client/scenario/catalog harness/engine/frontend/nav changes unless a
strictly necessary public type needs coordinated root approval; first report
that boundary instead of editing another owner's files. Autocast source is
released only after its parent completes review. Hostile/duel controls will be
dependency-gated behind this card by root; preserve all prior committed work.

Meaningful tests must cover a wall and actionable ground loc sharing one tile
in either row order, exact selected-ID targeting, stale replacement/missing
selected loc refusal, invalid action refusal and valid current legacy behavior.
Test actual shim/request/host dispatch composition, not just fabricated JSON
shapes or an implementation-mirroring helper. Run affected script/host-play tests
with required features and strict affected Clippy. Freeze exact committed host
plus its explicit client bytes and owned overlay using the regular Git-blob
helper in evidence/build-isolation/export_git_files.py, with a fresh EMPTY
Cargo target. Do not read concurrent target artifacts or extract tar archives.
Commit only owned source/report/evidence; verify the actual committed diff.
Request SAME-card profile reviewer (actual Grok4.5) with exact commits and
validation, then STOP. Root owns LIVE requalification and ledger acceptance.
