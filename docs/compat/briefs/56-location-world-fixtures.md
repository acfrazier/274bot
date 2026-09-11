# Location and world catalog fixtures

Implement with profile grok46 defaults after t_3317197c same-card review.
Read AGENTS.md, docs/execution.md, active plan steps 7-8, the operator ownership
clarifications at the top of docs/compat/STATE.md, this brief, and the relevant
DoorOpener/FlaxPicker/GnomeCourse fixture-design sections. This is bounded
shared-harness work in the existing catalog campaign. Root owns LIVE and ledger.

Own only crates/scenario/src/{lib,proof}.rs as needed,
crates/host-play/tests/catalog_boundary_live.rs, and report
05e-location-world-fixtures.md. No runtime, client, foreign catalog, navpack,
engine, UI, or other workers' source changes. Preserve existing cases and
harness/production deadlines. Reuse the registry, Start and independent witness
machinery; do not add a second runner. Use immutable source exports for checks
if concurrent runtime changes would interfere.

Add five cases, both revisions and both frozen catalogs:
- door_opener: configurable adjacent stand, default door/gate names, an actually
  shut door confirmed before Start. door_opener_gate: explicit gate filter and
  an adjacent stand by an actually shut gate.
- gnome_course: default complete obstacle order/radius.
  gnome_course_radius: searchRadius=8, same complete obstacle order.
- flax_picker: default Seers picking and full bank/return loop.

Read actual frozen scripts and selected cache/server loc scripts before fixing
seeds or witness thresholds. Fixture reports are design leads, not authority to
guess coordinates, IDs, level requirements, or accept a single click as a loop.
DoorOpener only clicks when the nearest shut candidate is within one tile of
the player; use its supported stand setting to meet that documented behavior.
Record the exact selected door/gate identity and a before-Start shut state.
Any Close used to prepare the fixture must complete before Start. After Start,
require the selected loc to become open through an observed same-session world
change. Script counters/logs or a queued interaction alone fail. Do not close
or otherwise manipulate the door after Start. Missing native snapshot facts
must be reported for root, not invented through script logs.

GnomeCourse must show a complete natural course sequence and further progress
at the start of a second lap. Derive observable ordered plane/tile/XP milestones
from selected server content, including final ground-level return; do not
accept repeated XP from the first obstacle as a full lap or rely only on the
script's lap log. Keep source defaults and all course steps. An unworkable
configured radius or source re-sync error is a concrete foreign-script/option
finding; do not rewrite the host or script to rescue it. If the full observation
cannot fit existing harness timing, report the measured/source reason for root
rather than silently weakening the witness or increasing product timeouts.

FlaxPicker starts empty at the selected default field. Require script-created
flax 1779, a full first pack, deposit into a fresh loaded Seers bank generation,
return to the field, and further picked flax. No seeded flax output, prefilled
pack or post-Start mutation. Keep witnesses exact-ID and scene/session-bound.
Avoid copying the entire world into every observation: retain only bounded
facts needed for these named cases.

Use meaningful proof rejection tests: seed-only, queued-only door, unrelated
loc change, stale generation, repeated first obstacle, incomplete lap, absent
second-lap progress, flax first-pick-only, missing deposit/return, and stale
bank facts. Run focused scenario/harness tests, formatting and strict affected
Clippy. Do not run LIVE. Keep the report concise with exact settings/source
identity and explicit missing facts/foreign defects where discovered. Commit
only owned paths using commit --only, request review on THIS card with profile
reviewer and exact source/checks, then STOP. Never self-complete. Root runs the
twenty catalog/revision cells after actual Grok 4.5 approval.
