# Proposed 289 client campaign dispatch

Prepared 2026-09-07 for operator review. This is a proposed task, not a record
of authorization or dispatch. The 274bot memory campaign remains active.

Objective: establish and implement a source-grounded revision-289 Rust client
in its own client checkout, preserving the working 274 line. Begin with a
reviewed source/fixture milestone, then execute bounded client implementation
stages. Do not assume unchanged TypeScript declarations prove compatibility.

Orchestration: the verified Hermes `orch` profile is gpt-6-astra/openai-codex.
Use existing 274bot board, explicitly scoped client workspace and task prefix.
Use profile defaults: implementer grok-composer-2.5-fast, bounded tooling luna,
per-task reviewer grok-4.5, final branchreviewer grok-4.6. Follow the existing
same-card review flow and verify actual review runs. Root repo hygiene remains
inline. A separate Codex task is an alternative, but only the host project is
currently registered in the app; the client must be given its own workspace.

Repository: acfrazier/FR-client-bothost. Before dispatch, prepare a dedicated
client checkout/branch with an explicitly pinned starting revision and its own
target directory. Decide whether the desired deliverable is a standalone
client or a bothost-capable client before choosing the modular starting branch.
Do not mutate the active 274bot client checkout/gitlink, public branch, or
measurement binaries. Client changes can be developed independently; host
integration is a separate later task. Avoid builds or tests on machines while
they are collecting clean measurements.

Primary evidence:
- /Users/acfrazier/experiments/FR-vault/research/deob/289,
  RuneWiki/openrs2-nonfree 0c00ef249546fada67b1f6eb8bbe01ea7c250c95.
- /Users/acfrazier/experiments/FR-vault/docs/research/deob-289-source-inventory.md
  and sibling deob-289-source-provenance.json.
- Existing vendor/client-java-289 at 6834c7255f559db5f1702b8b0e5e7286a7d61244
  is corroborating evidence; resolve disagreements explicitly.
- Host research: revision-289-host-script-delta.md in this directory. Its
  PARTIAL/UNKNOWN labels are still unresolved requirements, not implemented
  capabilities. rs2b0t diff 100adccc..8e7d965 supplies corroborating changes.

First milestone and deliverables:
1. Freeze source and cache inventories with hashes. Distinguish deob/client
   jars from actual game-cache archives. Identify the intended 289 server and
   its compatible cache contract; record missing artifacts explicitly.
2. Extract complete inbound/outbound IDs, lengths, payload encodings and
   login/RSA/ISAAC/reset sequencing from primary 289 Java. Map the affected
   Rust modules and every relevant delta from 274 with source anchors.
3. Add source-grounded offline golden tests for variable packet framing,
   inventory g2 counts/gsmart slots, actor update masks, region/scene updates,
   widgets and logout/reset. Check exact consumed lengths and rejection paths.
   Fixtures must have an independent oracle, not mirror newly written Rust.
4. Produce an implementation plan with dependency order and narrow acceptance
   gates. Obtain actual Grok review before treating it as the implementation
   contract. First executable milestone: verified login, player/world updates,
   scene readiness and clean logout against the identified fixture/server.

Subsequent stages: cache/config loaders, protocol and lifecycle, actor/world/
scene handling, rendering and basic interactions, then native proof and final
whole-branch review. Preserve the existing renderer ownership and 274 behavior.
Do not expand into host JavaScript compatibility, script policy, banking
routers, or tutorial/random-event policy as an incidental client-port task.

Acceptance: offline bytes/state transitions and client integration tests first;
actual 289 login/scene/action/reset proof when the server/cache prerequisites
exist. Record PASS, FAIL, and missing prerequisites separately. Do not claim a
complete 289 port from compilation or an unchanged script ABI.

Usage: no banked-reset redemption. Any expected automatic 9 PM reset is an
operator-reported future event, not a verified available budget. Check usage
at milestones and preserve a resumable checkpoint if the allowance is reached.
