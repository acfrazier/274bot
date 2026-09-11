# Refresh native static-loc observations without invalidating render caches

Use grok46 profile defaults. Host branch codex/rs2b0t-multirevision, client
branch codex/bothost-274-289 in vendor/fr-client-rust. Read applicable host
instructions, docs/execution.md and fail-closed-dispatch. No agents or LIVE.
Root owns Git integration/gitlink and all remotes.

Root source audit after exact isolated 3be68eaf FlaxPicker LIVE:
- r274 old catalog collected seven flax in the existing 150-tick window,
  repeatedly dispatching Pick to removed flax at 2737,3440. The selected 274
  pickables.rs2 always loc_del(25) after one flax; 289 randomly removes it.
- api Snapshot::rebuild_loc dirties from scene generation plus an aggregate
  of World tile_model_stamp. Client core/world.rs add_sprite explicitly does
  not bump those render stamps because doing so recreated unlit/black walls
  when moving players/NPCs. del_sprite also does not bump them.
- LOC_DEL reaches loc_change_unchecked -> world.del_loc -> del_sprite; static
  scenery is deleted, but that path need not dirty the API loc cache. Static
  add/respawn may have the same gap. This is a source-supported hypothesis,
  not yet a confirmed complete explanation of the Flax LIVE failure.

First reproduce missing refresh in a focused test: publish a static scenery
loc, delete it through the native seam without scene rebuild, rebuild the API
loc family and require disappearance; re-add and require return. Then repair
native static-loc observation invalidation with the smallest coherent seam.
A dedicated static-scenery mutation generation in World consumed by Snapshot
is a candidate. Inspect all static add/remove/reset paths, multi-tile bounds,
failed/no-op operations, level and scene changes, and counter semantics.
Keep walls/decor's existing invalidation and all dynamic sprite reuse intact.
DO NOT bump render model stamps on dynamic or static scene sprites, re-decode
walls, dirty the whole world per frame, clone the world, add a JS cache/policy,
or introduce a per-read scenery sweep to conceal this invalidation gap.
Rendering ownership and the last-FBO freeze remain unchanged. No measured
performance claim; verify no-op/dynamic churn does not invalidate loc reads.

Own only client crates/client/src/core/world.rs, focused unique client tests,
minimal API snapshot.rs invalidation wiring and unique API integration test,
plus 04g-static-loc-observation.md and evidence/static-loc-observation/.
Root is reserving snapshot.rs; DeathRecovery owns script/shared callbacks and
must not edit this file concurrently. No script, scenario, catalog fixture,
engine, foreign source, profile, frontend or navigation-policy changes.

Verify source exports with exact client bytes and owned overlay in new empty
targets. Run meaningful static delete/add/no-op/dynamic-churn host regressions,
client integration tests separately including relevant world/render regression
coverage, formatting and strict affected Clippy. Existing render-cache tests
must retain their assertions. No broad unrelated test churn. If the proposed
seam grows beyond this bounded fix or evidence contradicts it, stop and report.
Commit owned client source on the named client branch, then owned host source
and report. Never stage the gitlink, merge, push or reset. Record both exact
commits and source manifest; same-card reviewer must inspect the explicit
client commit/export because root alone updates the host gitlink after review.
Request SAME-card reviewer with those identities and focused test evidence,
then STOP. Root owns LIVE requalification after actual Grok 4.5 approval.
