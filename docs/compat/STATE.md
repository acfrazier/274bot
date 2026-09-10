# Compatibility campaign state

Updated 2026-09-10 22:53 UTC. The full implementation plan remains authorized:
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
   `6cb5a0b17aeef74da6b57b205e916681daee4f76`. Banking client facts are committed at 56d8027; host corrections remain active.
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
recorded in 03-world-capabilities.md. Both fixed-transfer bank-return cells
also passed at frozen 469a50ba/client 56d8027, using the independently reviewed
freshness path while named access/Withdraw-X corrections continue. All eight
scoped world cells now pass. Ordinary 289 operations are enabled at 87084cbc after those selected host/world
proofs; ten profile tests and focused strict Clippy passed on an exact export.
Catalog and frontend acceptance remain separate.

## Active and queued implementation

- Matching/fill t_41b2f50e committed 06077fe9, 23a30524 and fa23cc60, but
  incorrectly completed without review. Root reclaimed premature dependent run
  1137 and restored a real corrective gate t_d68ee0fc. Actual reviewer run 1140,
  session 20260910_180938_b537c4, approved frozen 76d61beb/client 56d8027
  with Grok 4.5 / xai-oauth, including root's count-dialog correction. The
  actual run completed at 22:17 UTC; source approval restores the gate.
- t_90b60f12 resumed as actual Sol run 1141 after that corrective review: Rust nearest-bank routing,
  honest withdrawal results and bounded observed deposit helpers (brief 31).
  BankFletcher exposed duplicate-row amplification and a slot packet panic.
- t_14db340e catalog harness completed with actual Grok 4.5 review 1133.
  Root is strengthening the BoneBurier proof after the headed observation;
  its original first-burial PASS is insufficient. 05a-catalog-live-harness.md.
- t_887b29b5 frontend session source 5eeff8f0/c9521d2f completed actual Grok
  4.5 review 1135; six targeted lifecycle tests and host tests pass.
- t_dde220ef Grok 4.6 integration review waits for bank routing, reviewed
  Alcher option fixtures t_a384fe64 and panel startup t_779f58e5. It also
  covers root profile/harness, script Stop and frontend/world changes.
- Loadout t_3737503d waits for integration review, then targeted spell facts
  t_63138b8b, PeriodicBank t_51452e47 and DeathRecovery t_0c5ce97f follow.
  Design reports are guidance, not implemented/live acceptance.

## Current headed work

The operator requested native windowed catalog runs. First BoneBurier on
local Mac 289 buried its five seed bones and then failed to open a bank. The
run, screenshot and misleading original first-burial PASS are preserved;
no full-loop acceptance. Both the missing travel fallback and withdrawal
result are confirmed source gaps. Root will keep planned API semantics and
explain evidence before treating conditional concerns as scope changes.
The revised fixture adds bank stock only before Start and requires actual
restocking plus another burial. Corrected ChickenKiller f2b04198 showed the
Strength announcement and repeated combat/loot/bury cycles. Thiever f2b04198
showed 27 steals/810 coins without script errors, banking disabled and no food
consumption. Alcher failed after a successful noted withdrawal because the
host left the count prompt open; root correction 76d61beb passed 65 API tests
and strict Clippy, and passed source review 1140. Its ScriptRunner.stop then
produced repeated interrupted-slow-tick logs. Root now propagates terminal
isolate state into slot Stop cleanup before host continuations; focused
stopped-slot/restart and no-slow-tick regressions plus strict script/host-play
Clippy passed on a separate source export. Integration review remains pending.
BankFletcher made the initial products, then its slot panicked; the original
process exit 0 is preserved with an explicit failed qualification. No complete
catalog/options acceptance from these basic diagnostics.

The operator's startup beachball reproduced: the slot starts at +37.47 seconds,
with the UI thread spending the startup sample hashing navigation resources.
Profile bind, template load and Play start hash the 73 MB pack and 261 MB flags
three times. 06b-panel-startup-trace.md records the trace and source path.
Actual Grok 4.6 run 1142/session 20260910_183241_b9252e completed the
06c-panel-startup-design.md review. Implementation t_779f58e5 waits for bank
source review to release the shared file. Brief 42 preserves all validation
with a consuming checked handoff and UI-owned session/slot setup. No startup
fix or performance claim yet; no validation has been disabled.

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

Complete the required capability families,
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


## Newly prepared follow-on capabilities

After matching/fill and loadout source review, the serialized shared-file queue
continues with spell facts/targeted inventory cast (`t_63138b8b`, brief 27),
PeriodicBank (`t_51452e47`, brief 28), then DeathRecovery (`t_0c5ce97f`, brief 29).
They use Sol defaults and same-card Grok 4.5 review. These are scoped tasks,
not source or live acceptance. Autocast/special/teleport, hostile facts, shop,
make menus and fire remain subsequent capability work from the existing design.

## Current catalog evidence and prepared follow-ons

Native Alcher at 1947741f/client 56d8027/catalog 100adccc completed 30 casts
over 27+3 withdrawals, banked 900000 coins, and stopped cleanly on 274/289.
Post-stop logs were clean for 59.79/56.68 seconds. Native 274 Thiever also
completed its 180-second observation without script errors; the read capture
shows three steals/90 coins with banking off, not eating/restocking proof.

Independent core cells use frozen c933f37c/client 56d8027, which adds the
client display-name comparison to the 1947741f harness. The initial 289
1947741f cell failed before Start on underscore versus space. Corrected
Alcher and ChickenKiller core cells pass for catalog 100adccc on both
revisions; Thiever core passes likewise. Later-catalog 289 Alcher, ChickenKiller
and Thiever pass; later-catalog 274 work continues. See 05-catalog-proof.md,
core-results.json and newer raw receipts in evidence/catalog-harness/live/.
Rows remain PARTIAL until required options and integrated review complete.

Alcher option fixtures t_a384fe64 use Luna defaults; actual Grok 4.5 review
requested an ordered negative proof, corrected at 41d7a1e3 and back in review.
No option LIVE acceptance yet. Serialized capability tasks after recovery are
recorded in evidence/combat-production-design/implementation-queue.json: briefs
34-40 for autocast, hostile/duel facts, special, teleport, shop, make panels
and fire. These are scoped pending implementation, not accepted capabilities.

Root packaged exact c933f37c/client 56d8027 plus both catalogs/nav identities
for platform smoke. Linux and Windows fixture/process identities were checked
again. The Hyper-V builder verified all 4807 archive files and is compiling the
headless catalog harness and TUI using its existing cache. Windows transfer
completed and extraction/verification is running. No remote gameplay result
is claimed; Concord remains TUI-only and sequential revisions.
