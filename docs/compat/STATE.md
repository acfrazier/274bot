# Compatibility campaign state

Updated 2026-09-10 20:54 UTC. The full implementation plan remains authorized:
`docs/superpowers/plans/2026-09-10-rs2b0t-multirevision-finish.md`.
Campaign checkout: `/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision`.
Host branch: `codex/rs2b0t-multirevision`; client: `codex/bothost-274-289`.
Read `docs/execution.md` for dispatch/review rules. Reports below preserve dated
results and failed cells; this state identifies current work.

## Accepted milestones

1. Frozen inputs and ledger: both immutable RS2B0T roots, 45 enabled cards each,
   274 and 289 profiles, 180 card/source/revision rows before option branches.
   `00-inputs-and-scope.md`, `support-matrix.json`, and `fixture-inputs.json`.
   Loader evidence is not live gameplay acceptance.
2. Bothost client reconciliation and independent whole-client Grok 4.6 review
   completed. The later accepted publication/session seam is client commit
   `6cb5a0b17aeef74da6b57b205e916681daee4f76`. Banking client edits are still WIP.
   Client provenance and preservation results live in its integration report.
3. Immutable server/revision/resource binding reaches actual shared templates
   and both frontends. Local 274/289 remain distinct. Public configuration uses
   revision 289 at w1.rs2b2t.com; public 274 is rejected. Script imports and Start
   are revision agnostic by operator direction; users judge suitability and
   unavailable operations refuse at host/client boundaries. See
   `01-session-profile.md` and `evidence/script-loading-policy/`.
4. Host outbound and snapshot/session source passed same-card Grok 4.5 review
   and combined Grok 4.6 review. Controlled Mac live host boundary passed for
   both revisions using frozen host `4f43800a` and client `6cb5a0b1`: preparation
   logout/relogin, natural courtyard walking/Hans dialogue, exterior door
   interaction/world change and actual IF logout clearing observations.
   Exact commands, hashes and retained failed cells: `02-host-boundary.md`,
   `02a-host-outbound.md`, `02b-host-snapshot-reset.md` and their raw evidence.
   This is bounded host proof; it does not qualify every catalog row or frontend.

## Current world qualification

Step-5 source/bake commit `80720784` passed same-card Grok 4.5 review. Both fresh
packs, raw flags and shared manifests are bound to their audited source/cache
identity under `.superpowers/world-capabilities/{274,289}`. Report:
`03-world-capabilities.md`. Controls, food and current Guardian inputs were
checked against both selected caches; no new solver expansion is authorized.

World harness corrections and panel/TUI ScenarioRunner world sharing passed
same-card `t_1f03d5a7` review, actual Grok 4.5 run 1128/session
`20260910_163425_67c166`. Frozen host `367af78f`, client `6cb5a0b1`, binary SHA
`03c964a1f0fb014238b0fe419f2c92119359e505b6c96a452fee562fb6bb57b1`.
Root reverified all 486 exported source files after build. Both nav_full cells
passed actual cross-square arrival at (3220,3264,0), with engine-speed override
cleared. Both final door and lamp Guardian cells passed at frozen 43a57c36/client
6cb5a0b1 after diagnosed fixture corrections. All six scoped world cells are
recorded in 03-world-capabilities.md. Bank return remains pending banking
completion; step 5 as a whole is not yet accepted. No ordinary
289 operation-gate removal has happened.

## Active and queued implementation

- `t_e52e0a03`, Sol: first banking family (brief 18). Fresh full-container and
  modal observations, Bank readiness, named booth options, host-owned bounded
  Withdraw-X, posted inventory settlement. Preserve 3000 ms dialog / 4000 ms
  settlement deadlines, Pause freeze, Stop/session abort. Candidate 8a60eef7/client 56d8027 needs correction after Grok 4.5 run 1131
  and root findings (named open, rejected/noted/zero Withdraw-X). Actual Sol
  corrective run 1132 is active; 04-capabilities-banking.md records evidence.
  Worker owns API, script and host-play source; root audited the exact gitlink.
  Require actual same-card Grok 4.5 approval and composed script-to-host-result
  tests before accepting. No bank live proof yet.
- `t_41b2f50e`, Sol, queued after banking: common-loot matching and withdrawLoad
  (brief 22), Rust matching and fresh observed bank settlement, thin mappings.
- `t_14db340e`, Sol: controlled real-library catalog harness (brief 23), new
  test/report only. Actual run 1129/session `20260910_163525_589385` verified.
  Root owns live runs after review and required capability/world qualification.
- `t_9f1baf44`, Grok 4.6: remaining combat/casting/shop/trade/production design
  completed in 04-combat-production-design.md (brief 24), verified run 1125.
- `t_887b29b5`, Sol run 1130/session `20260910_164126_9a71eb`: panel/TUI session snapshots and
  external armed-route cleanup (brief 25). Root found direct snapshot rebuild
  bypasses and retained external WalkArms/tick latches. Fix logout/reconnect and
  same-name slot recreation while retaining Guardian/ordinary-scene behavior.
  Banking owns host-play/src/lib.rs; serialize any needed callback seam first.

Provisioning/recovery design is complete in
`04-provisioning-recovery-design.md`, actual Grok 4.6 run 1118. Loadout
quantities/slots, common loot, PeriodicBank and DeathRecovery are scoped by real
in-scope callers. Design is guidance, not implemented or live acceptance.

## Platforms and fixtures

Mac 274 remains the pre-existing shared local fixture. Root owns the isolated
Mac 289 fixture and the Windows, Hyper-V and Concord campaign fixtures. The
reviewed local-only moderator givebank addition is 289 engine `cc359656`,
actual same-card Grok 4.5 approval. Root refreshed all four 289 runtimes and
verified their unchanged cache archives and all 1,213 runtime files. See
`platform-preparation.md` and `evidence/platform-preparation/bank-fixture/`.

All 289 fixture startup/HTTP/cache observations passed; remote host gameplay
has not run yet. Windows native panel/client, Linux and Mac frontend acceptance
remain open. Compile Linux on the Hyper-V builder; Concord is TUI-only and its
old 274 system unit was stopped by the operator. Run revisions sequentially on
Concord. Prior held SSH lifetime failures are retained; old recorded PIDs are
not current authority. Verify process identity before stopping an owned process.

## Remaining finish line

Complete world/Guardian/bank-return qualification, required capability families,
every supported card and option branch, both frontends and lifecycle controls,
N=2 isolation, qualified N=32 Thiever on both revisions, actual native CPU/GPU
preservation, and final whole-branch Grok 4.6 review. Then integrate the reviewed
client into acfrazier/FR-client-bothost r274-bh-modular and the host exact gitlink,
with recursive fresh-checkout verification. No main merge, push, release tag,
package or announcement has occurred in this campaign.

The target remains host alpha 2 / 0.1.7; release planning follows acceptance.
Full 377, mixed fleets, tutorial/audio parity and the named quest/clue/gatherer/
MarketMaker rewrites remain deferred. Do not shrink the enabled ledger or add
script revision allowlists to make acceptance pass. Do not restart the completed
memory optimization campaign or turn elapsed diagnostic time into savings claims.
