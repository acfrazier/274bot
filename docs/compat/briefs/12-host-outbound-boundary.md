# Step 4A: revision-correct host outbound boundary

Use `sol` profile defaults, then the same-card `reviewer` handoff. Read applicable
AGENTS/execution once, plan architecture and step 4, and
`docs/compat/02-host-boundary-design.md`. Host branch is
`codex/rs2b0t-multirevision`, inspected base `db9b741a`; accepted client pin
`2be1697060e4d2b8b709ad4d5e54d12513b38333`. Root owns Git remotes/gitlinks.

Implement design task 1 only: add the bound revision to `api::Driver`, select
named client protocol rows for every direct host writer, make legal-send
coverage revision-aware, and independently prove payloads through actual
host interactions/Client driver. Allowed product files are
`crates/api/src/{prot,interact}.rs`, the necessary call site in
`crates/nav/src/traveller.rs`, related Driver stubs only if required, and
focused new/existing API tests. Do not edit client sources without reporting a
concrete missing generic client seam to root first. No second opcode table.

Root correction to the design report: its blanket absence claim for old numeric
ids is wrong. R289 legitimately uses 51 for OPPLAYER2 and 224 for
EVENT_MOUSE_CLICK. Validate each **named** row against independent primary
fixtures, including its payload length; never ban numbers merely because they
were different packets on R274. Current ClientRevision has only R274/R289;
use exhaustive matches, do not invent an R377 variant for a refusal test.

Keep existing R274 public APIs/bytes/return semantics, including legacy
recorders and exposed Send behavior. Add a checked revision-aware write path
where necessary: a forged/unsupported typed Send on R289 must fail before
out.pos/ISAAC/menu/route mutation, without catching a mapping panic. Typed
payload identity must remain the original named packet, not the remapped id.
Known named constants can safely use the existing client mapper; do not build
a duplicated host mapping to cope with a hypothetical enum variant.
Direct IF_BUTTON/CLOSE/COUNT/WalkStep and cheat payload ordering remains as in
the design. Keep `Driver::do_action` acceptance semantics intact. Public target
cheat refusal must still happen with no bytes regardless of revision.

Use pinned primary write sequences and existing client revision-289 fixtures
as independent expected bytes, not mapper-derived expectations. Cover direct
writers and named NPC/loc/ground/item/player/widget host paths, including a
refused action with no send. Inspect all production raw writes to ensure no
missed caller. No live sessions, no broad 289 bot-operation gate lift, no nav
or guardian/content implementation, no unrelated tests or formatter churn.
Root will run both local engines through a separate controlled proof harness
after same-card source reviews complete.

Concurrent public-profile card edits profile.rs and frontend selectors/help,
not these action files. Another step-4 card owns snapshot.rs and host/host-play
queue reset code; do not edit those files. Root owns the live harness and STATE.
Use CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target for focused tests.
Retain meaningful failures and final logs under
`docs/compat/evidence/host-boundary/outbound/`; write concise source/test findings
to `docs/compat/02a-host-outbound.md`. Commit only scoped files, call
kanban_request_review with reviewer `reviewer` on this same card, then stop.
