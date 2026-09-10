# Step-5 world/navigation source and fixture preparation

Use `sol` profile defaults, then hand this same card to `reviewer` and stop.
Verify host branch `codex/rs2b0t-multirevision`; source base `b9cacc6e`.
Client `6cb5a0b17aeef74da6b57b205e916681daee4f76` on
`codex/bothost-274-289` is root-owned. Read applicable AGENTS/execution once,
step 5 of the active plan, `world-source-audit.md` and
`evidence/world-capabilities/source-audit.json`. Do not read the whole campaign
history. Step-4 source review/live proof is being completed independently by
root; this task may prepare step-5 source/assets, but may not claim live
acceptance or remove the temporary 289 slot gate.

Implement the smallest coherent step-5 source changes needed to bind the baked
world and existing host content/guardian behavior to the selected revision.
The v8 nav format already suffices; preserve its collision/transport/stand
representation, route worker generations, pending request bounds, failed-search
route retention and retry behavior. Existing host-play NavManifest binds
revision, cache identity, nav SHA and optional flags SHA; move/reuse that schema
as necessary for the nav bake CLI without a dependency cycle or duplicate
production format. Keep the compatible unmanifested legacy 274 path. A 289 bake
must explicitly select its revision, pinned content/maps, and
`engine/data/pack/client/config`; the 274 default is `engine/data/pack/config`.
Generate manifests from the actual inputs and outputs, reject mismatches and
ambiguous/wrong revision inputs before overwriting an artifact. Don't infer
revision solely from a filename or accept a claimed revision with unrelated
cache bytes. Reuse the checked cache identity format/known fingerprints or an
explicit verified manifest. No fresh nav format version unless demonstrated
necessary.

Bake and inspect fresh 274 and 289 packs under gitignored
`.superpowers/world-capabilities/` using the pinned fixtures in
`fixture-inputs.json`. Do not overwrite ~/.274bot packs. Retain hashes, exact
commands, counts and scoped verification under `evidence/world-capabilities/`.
This task may run local source/cache/nav offline checks, not client gameplay or
server restart. Do not mutate the external engine/content trees, remote hosts,
credentials, existing databases or accounts. Root owns any necessary local
fixture givebank addition and live proof.

Audit actual selected cache/world data for existing bank and control discovery,
food/spells/teleports/transport endpoints and the current guardian supported
solver inputs. The source audit found selected genie/mime/box/shop/trade IDs and
stunned_thieving=245 unchanged; verify actual cache shape before asserting
runtime equivalence. Bank capacity grows from 8x30 to 8x36. Preserve existing
incomplete features, errors and unknown-event hold policy. No new guardian
solver campaign or foreign runtime/router/planner cloning. Fix real mismatches
in Rust only; unchanged definitions do not need speculative abstraction.

Allowed product ownership: crates/nav; crates/api/src/content.rs; targeted
snapshot control discovery if a verified selected-cache mismatch requires it;
crates/host/src/random.rs; nav manifest/default-loading parts of
crates/host-play/src/profile.rs; focused tests. Do not edit host-play/lib.rs,
script isolate transport, client sources, panel/TUI, root live harness,
STATE or the active plan. Report other required changes to root with exact
source evidence. Root may finish cross-crate wiring after this task review.

Operator clarification: scripts may load/start irrespective of revision.
Users determine script suitability; unsupported operations refuse at host/client
boundaries. Do not add catalog/revision allowlists or restore removed catalog
hash gates. Data resource binding does not authorize a script compatibility gate.

Run proportionate affected nav/content/guardian tests with needed features;
strict focused Clippy/format checks. Existing protocol/lifecycle tests elsewhere
need no duplicate campaign. Never stash/reset/restore/checkout/alter other WIP or
move the shared index for isolation; root owns Git hygiene. Commit only scoped
files, write `03-world-capabilities.md` distinguishing source/bake checks from
pending live door/region/bank-return/guardian proof, request review on this same
card from profile `reviewer`, then stop. Root performs accepted live qualification
and any operation-gate removal after prerequisite reviews.
