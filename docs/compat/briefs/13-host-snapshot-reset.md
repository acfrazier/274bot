# Step 4B: snapshot publication and session reset

Use `sol` profile defaults, then same-card `reviewer`. Read applicable
AGENTS/execution once, plan architecture and step 4, and
`docs/compat/02-host-boundary-design.md`. Host branch is
`codex/rs2b0t-multirevision`, inspected base `db9b741a`; accepted client pin
`2be1697060e4d2b8b709ad4d5e54d12513b38333`.

Implement design task 2: independently test actual decoder-to-host snapshot
publication for both revisions, including 289 g2 inventory counts/gsmart
slots, bank/interface updates, actor identities, scene/region generations,
PLAYER_INFO tick publication and logout/reconnect reset. Audit and fix stale
queued wire/cheat/script work crossing a disconnected session. Preserve
existing shared generation ownership, no deep world copies, lifecycle command
ordering, timeouts, Pause/Stop and guardian behavior. No invented tick opcode.
Do not complete unrelated disabled features or step-5 content assumptions.

Allowed product files: `crates/api/src/snapshot.rs`, relevant
`crates/host/src/{lib,slot}.rs`, and queue/drain handling in
`crates/host-play/src/lib.rs`, plus focused associated tests. Root owns a new
host-play live test harness, do not create or modify it. Do not edit client
sources without reporting a concrete missing generic seam to root first.
Concurrent action worker owns api/prot.rs, api/interact.rs, nav/traveller.rs
and outbound tests. It adds Driver::revision with R274 default for recorders;
the real Client returns its bound revision. Concurrent public profile work
owns host-play/profile.rs, main.rs, frontends and session_profile.rs tests.

Do not assume every logged-out synthetic Client test represents a live session.
If gating snapshot families on ingame, distinguish production reset semantics
from test fixture setup and add meaningful lifecycle regression coverage. Old
NPC/player/inventory/bank state must not be presented as a current live session
after logout or reused on reconnect. Stale queued work must not be flushed
after reconnect. Handle command production and consumption ordering, not just
a single empty-queue assertion. Preserve legitimate queued Login/Start/Stop
control semantics and current 274 incomplete stubs. Production 289 slot,
script, Play cheat/wire and frontend gates remain in force.

Use pinned primary decoder fixtures / existing client tests for independent
frames; feed the production decoder and inspect host views/tick publication.
Existing no-clone/generation tests remain relevant. No live sessions or broad
test reruns without a failure/change justifying them. Root runs controlled
local 274/289 proof after source review. Use
CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target. Retain failures and
final focused logs under `docs/compat/evidence/host-boundary/snapshot/`; write
`docs/compat/02b-host-snapshot-reset.md`. Commit scoped files, request same-card
review from `reviewer`, then stop. Root owns STATE, remotes and client gitlinks.
