# Step 4 host action and snapshot boundary design review

Use configured `grok46` defaults. Read AGENTS/docs/execution once and plan
architecture plus step 4. This is a bounded pre-implementation design/audit;
do not edit product code or launch live sessions. Host branch is
`codex/rs2b0t-multirevision`, base `db9b741a`; client pin `2be16970`.
Concurrent card `t_96dc78da` only corrects public profile resolution/help/tests
and current profile documentation; it does not touch action/snapshot code.
Operator confirms public is 289 at w1.rs2b2t.com:443; public 274 is unavailable.
All live step-4 proofs use the isolated local engines, not the public server.

Inspect actual writers and observation/reset paths, then approve or correct
the following proposed ownership shape with concrete implementation seams:

1. Keep named protocol definitions and revision mapping in the client. Host
   `api::Driver` exposes the bound revision (R274 default for legacy recorders;
   real Client reports `revision()`). Typed direct sends, legal-send rows,
   cardinal door steps and local cheat packets select named client rows using
   that revision. Preserve 274 byte order, lengths, ISAAC, send ordering and
   old APIs where their additive default can remain safe. Do not duplicate a
   foreign runtime or add a bot action API inside the client.
2. Audit every production raw outbound writer, including outside api. Current
   known sites are api/prot.rs Send and WalkStep, interact.rs cheat, and nav
   traveller call sites. MiniMenuAction/Client::doAction and tryMove already
   have revision-selected packet paths: verify their actual host use and
   return/error semantics rather than assuming opcode mapping proves payloads.
   Legal tables must handle revision-specific lengths and valid 289 additions
   without silently allowing a 274 opcode. Independent byte fixtures must use
   the pinned primary source/engine write/read sequences, not only the mapping
   implementation under test.
3. Audit real snapshot publication and script/host consumers for g2 counts,
   gsmart slots, bank/interface updates, actors, scene/region generation,
   PLAYER_INFO tick publication, reconnect/logout reset and old queued work.
   Keep existing shared ownership and no deep world copies. Preserve 274
   incomplete stubs/errors, timeouts and routing safeguards. Make a missing
   capability explicit; only finish capabilities authorized by step 4.
4. Preserve the broad 289 bot-operation gate until the host boundary and world
   guardian/nav assumptions are qualified. Step 4 needs a narrow controlled
   local proof via the actual host driver/shared client constructor while
   step-5 world/guardian binding remains unqualified. Propose the smallest
   honest seam for that proof without exposing ordinary 289 slots/scripts
   prematurely or turning off existing guardian policy in production.

Primary artifacts: client `docs/revision-289/source-contract.md`,
`protocol-289.json`, `crates/client/src/io/{revision,client_prot,client_prot_289}.rs`,
existing client `tests/revision_289_{outbound,actors,stage1,stage2}.rs` and fixture
provenance. Local engines are inventoried in docs/compat/fixture-inputs.*.
No network research or archive instructions are needed for this review.

Deliver `docs/compat/02-host-boundary-design.md`: explicit verdict, short
base-to-current risk inventory with concrete files/call sites, bounded coherent
implementation tasks, meaningful byte/state tests, and local login -> scene2
-> walk -> NPC/loc interaction -> logout proof plan. Distinguish design approval
from source/live acceptance. Scope is step 4; step 5 world capabilities and
step 7 all-catalog gameplay are later. Root owns machine/engine preparation.
Complete this review card with actual outcome; do not request routine review
on a review-only card. Do not commit, use remotes, move gitlinks or spawn agents.
