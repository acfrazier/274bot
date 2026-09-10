# Selective optimization merge assessment

Prepared September 9, 2026 (local time), from existing reports and source inspection.
Revised following the operator's clarification that the reusable harness is part
of the product contract for RuneScape preservation work, and that application
fixes and Windows/Linux/macOS portability should be preserved.

This is a fresh assessment requested by the operator, not a campaign resumption.
It proposes a smaller release scope for approval. It does not change historical
results or declare the former performance finish line complete.

Inspected host: `ef01b1e1b822c9fc6826eb6e77fe11e4b2a33bd6`.
Local main and merge base: `54cfcf8a33613735bde9f43dd9c9beb8ec06dae9`.
Main's client: `4f2048ea10f75b3bb92ff45610b35ba7313b0308`.
Campaign client: `5c73a4a27f3d72834c2c2a071668eb197eb39fd9`.
The initial assessment used local Git observations. At extraction start, origin
main and the authorized client branch were refreshed with `ls-remote` and matched
these main tips. The operator approved this plan and made delegation optional,
while preserving the final Grok 4.6 review.

## Recommendation

Retain the original five changes, **add shared navigation in seed preparation
and lazy panel CPU uploads**, and include the product fixes evaluated below.
Also include terminal scenario-snapshot cleanup with its ordering correction:
the scenario runner and its evidence are part of the retained harness.

Do not merge this campaign branch wholesale. Leave the experimental storage
architectures, advanced profiling/measurement controllers, reports, raw data,
and campaign-specific test tools here. Preserve the reusable Rust harness,
its required basic accounting, and its production/test integration. Do not
equate "used by a test" with "disposable tooling."

This is a recommendation to accept useful engineering improvements with stated
limits. It is **not** a claim that all selected changes have proven end-to-end RSS savings or
that the old CPU, latency, scaling, and low-end budgets passed. Those are stronger
claims than the documentation supports.

## Optimization and ownership changes

| Change | Available evidence | Cost and limitation | Recommendation |
|---|---|---|---|
| Reuse completed dynamic sprite slots | Native diagnosis found the sprite-model allocation growing from about 83 to 166 MiB. The regression keeps 1,000 frames to two arena slots instead of 1,001. | Small free-list change in two client files; static indices remain separate and render stamps are cleared. No isolated matched RSS result. | Include as a demonstrated storage-growth fix. |
| Read animation delay directly | Regression changes 100 fallback lookups from 700 allocations to zero. Two 32-bot comparisons report roughly 0.86–0.88 CPU cores versus 2.05–2.66. Improvement also survives normalization by client iterations. | About a dozen production lines. One-bot CPU is effectively unchanged. At 32 bots, iteration rates fell about 6–7%, and mean UI/script timings increased; full responsiveness non-regression is unproven. | Include for the substantial repeated CPU benefit, subject to the limited combined-build check below. Do not advertise a memory saving. |
| Box sparse appearance packets | Empty-table storage falls by 4.109375 MiB per client on the measured layout. Receive, reset, independence, and cached re-entry behavior have tests. A longer 16-bot pair observed 58.40625 MiB lower RSS. | Tiny production change. Occupied slots pay a separate allocation. Short results overlap, so the longer pair does not establish a reliable fleet RSS saving. The public Rust field type changes; known construction sites are updated. | Include for the concrete per-client storage reduction. |
| Share private animation bases | Repeated one-bot screens show a minimum 14.5625 MiB separation exceeding the observed repeat spread. A longer pair observed 18.265625 MiB lower RSS. Allocation capture removes the selected repeated-clone stack. | Roughly 50 added production lines in one file; most of the 491-line diff is regression coverage. Public lookups still return independent owned values. This is a process-wide saving, not a per-bot multiplier. Short CPU results are inconclusive. | Include: meaningful memory evidence for a contained implementation. |
| Release snapshot storage on explicit Stop | Regression grows an encoded snapshot beyond 1 MiB, checks fingerprint/builder release, preserves Pause behavior and diagnostics, and verifies a fresh restart keyframe. | Six production lines plus test support. No established post-Stop RSS saving; allocators may retain released pages. | Include as a simple ownership/lifecycle correction. |
| Share navigation during harness seeding | Removes per-seed navigation decodes and then the remaining second decode beside Play. Tests establish shared Arc identity, independent runner state, and no retry when Play has no pack. A longer 16-bot comparison observed 77.14 MiB lower RSS. | Real benefit to the harness, which is part of this product. The measured pair does not establish ordinary interactive Play savings or final latency acceptance. Requires the harness foundation absent from main. | Include both stages: `7d026bc` and the final `606c93b` behavior. |
| Lazily allocate panel CPU uploads | Avoids about 1.468 MiB of CPU staging payload and an applet-sized nominal GPU texture per actual GPU-only GameView. Eleven focused tests cover initialization, switching, retained pixels, and disposal. | Native RSS comparisons are confounded by startup decay; the allocation avoidance is concrete. Count views, not bots. | Include `daa97f7` together with pixel-preservation correction `c90053d`. |
| Release completed scenario snapshots after evidence | Tests preserve terminal screenshots, outcomes and same-tick failure evidence while clearing the completed runner's snapshot. | No matched process-RSS claim. Releasing before callbacks finish would change harness behavior. | Include final combined behavior from `122ad32` + `0e909b9`, never the first commit alone. |

Evidence: [sprite diagnosis and reviews](prerequisite-status.md),
[scalar delay](scalar-delay-experiment.md),
[appearance implementation](appearance-packet-box-report.md),
[appearance measurements](appearance-confirmation-report.md),
[animation implementation](animation-base-sharing-report.md), and
[animation measurements](animation-resource-evidence.md).
Stop evidence is in host commit `e65103b` and its embedded regression.
Harness-sharing evidence: [scope and tests](shared-play-nav-world-report.md)
and [longer comparison](shared-nav-confirmation-report.md). Panel evidence:
[implementation](panel-lazy-upload-report.md) and
[native comparison limits](windows-lazy-upload-comparison-report.md).

The strongest CPU evidence is at 32 bots. The animation-base benefit is shared,
whereas sparse appearances scale with client count. These measurements come from
different baselines and settings: **do not add them into a total saving**.

## Product fixes recommended for inclusion

These are justified by correctness and portability evidence, not by memory
savings. "Preserve behavior" means retain the corrected branch behavior in these
areas; it does not mean restoring the bugs or unsupported helpers on main.

| Area / source | Product behavior and evidence | Extraction / validation decision |
|---|---|---|
| Routing ownership and walk-near — selected `a52b12c` hunks | Limits each slot to one search worker and one latest pending request; rejects stale publication; retains an old route after failed search; clears failed request latches so retries work. Radius-aware approaches avoid an occupied bank tile. Regressions cover publication, retries and approach selection; final prerequisite review approved the corrected code. | Include the whole coherent routing capability, including script request encoding and host dispatch. Do not take only the new request tag or omit worker generation/token safeguards. Retain existing exact-walk, failure, teleport and bank-session behavior. |
| Banking, loadouts and food — selected `a52b12c` hunks plus `1211d0a` | Repairs actual script capabilities: selected loadouts reach the isolate, food is resolved through host data, bank approaches skip unnecessary walking when adjacent, and Rust sequences withdrawals. Later `Bank.withdrawX` waits for published inventory instead of declaring success after queuing an answer. The diagnosed bug overfilled backpacks with 28 food and stalled thieving; the corrected live 32-bot run restocked all bots to 22 and every bot resumed stealing. | Include Rust helpers, their thin JS adapters, loadout delivery before ticks, wire round-trip tests, and delayed-publication regression as one dependency group. These include implemented capabilities that were stubs on main, not merely internal refactoring. Preserve remaining unsupported errors, including `nextWithdrawChunk` and unsupported bank access; no expansion of the compatibility roadmap. Keep the documented existing bounds, including 4000 ms for corrected Withdraw X and the other helpers' separate timeouts. |
| Door generation and recovery — `a52b12c` recovery pieces, `72804cd`, `b9f40fe`, `7992f68`, `94adac4`; final comments from `5e0f3ec` | Fixes walking back toward an already-crossed door; prevents baked edges jumping through blocked scenery; adds the existing one-tile movement packet at the useful moment after Open. Regressions and live closer tests support the fixes; normal and reversed login order both reached the exact destination. | Include the completed sequence, API packet helper and affected harness tests together. Preserve scene/plane/cardinal/adjacency checks, server collision authority, arrival predicates and retry budgets. No invented opcode. Regenerate navigation packs: a new binary does not repair an old v8 pack. |
| Bounded thieving-stun recovery — `a52b12c` snapshot/Traveller pieces | Host observes spot-animation 245 and conservatively waits eleven distinct player-update ticks, once per follow run, before rearming an affected walk. Unit tests and Grok review cover the bound; the documented successful fleet run did not trigger this path. | Preserve as existing reviewed behavior, with an explicit limitation: it is an inference for this revision/content, not an authoritative or universal stunned flag. Include a targeted existing-harness exercise before claiming live recovery proof. Do not generalize it to another RuneScape revision as part of this merge. |
| Focused reconnect priority — `d32b493` | Priority formerly disappeared after the first login permit, so mainland reconnects fell behind other slots. Queue tests reproduce and fix it; the captured run shows the focused bot first on initial and reconnect handshake starts. | Include. Preserve spacing, IP/device limits, no phantom queue reservation, and preference clearing when focus/slot changes. Priority does not preempt an existing connection. |
| Overlay, minimap and modal restoration — client `bc8bb4e`, `451759f`, `3456edc` | Changed/removed overlays now invalidate their upload; the correction preserves held minimap pixels during freeze. The modal fix handles sealed scene windows that otherwise stay black. Readback regressions exercise movement/removal/freeze; the modal report records all nine iface-model tests passing on macOS after restoration. | Include all three in order. The minimap correction is required with overlay invalidation. Keep ordinary lazy uploads and CPU fallback; no shader redesign or lowered rendering cadence. Do not carry forward older reports' modal failures as if the restoration never happened. |
| Windows sockets and control wakes — client `35f1c13`, host `9cdd4a2` | Replaces Unix-only socket assumptions with Windows readiness handles and a nonblocking loopback wake pair. Stack-based waits preserve control draining, socket priority and Stop wakeups. Native Windows host tests (179) and host-play tests (119) passed in the recorded freeze; native TUI/panel builds and gameplay evidence exist. | Include both repositories' implementations and target-specific dependencies. Keep the original Unix park body. No polling loop, shortened timeout or per-wait heap allocation. |
| Windows home paths — client `4b35300`, host `1e33d28` | Falls back to USERPROFILE only when HOME is unavailable, while preserving explicit blank HOME and Unix behavior. Covers cache, pack, scenario/e2e, and script paths. Native selector tests and independent review passed. | Include all retained consumers. Preserve the script-local adapter so script does not acquire a production client dependency. Include client lockfile `2b1af85`; generate the final host lockfile from selected dependencies rather than copying the campaign lockfile. |
| Portable process metrics and fixtures — `8ee4bf3`, `c8b9334`, `61b7b7d` | Windows panel/harness receives real current working set, peak working set, and process CPU. Captions label peak correctly. Windows NUL fixtures replaced Unix-only /dev/null assumptions; the native memory-feature suite then passed 180 tests. | Include: these are useful product/harness compatibility changes. Keep current and peak distinct and unavailable results honest; Windows TCP-count sampling remains unsupported. This does not require the external managed-process accounting system. |

Sources: [prerequisite diagnosis/reviews](prerequisite-status.md),
[bank and contested-door proof](bank-and-door-stability.md),
[door forward progress](door-approach-fix.md),
[door-edge generation](door-edge-correction.md),
[stun scope and evidence](stun-recovery.md),
[overlay and login proof](overlay-login-follow-up.md),
[modal restoration](modal-restoration-report.md),
[socket implementation](windows-socket-portability-report.md),
[native Windows proof](windows-native-first-proof.md),
[independent home/fixture review](windows-native-first-review.md), and
[native Windows/Linux gameplay](native-platform-functional-report.md).

Native support is demonstrated in bounded configurations, not universally
certified. The recorded Windows/Linux GPU shade-boundary failure and Windows
CRC unreachable-server timing failure are separate unresolved issues, with no
shipping correction in the inspected client ancestry. Keep them visible in the
final platform notes; do not fix them by weakening tests or claim every native
client test passes. See [native failure analysis](windows-native-additional-failures.md).

## Harness boundary for preservation work

Retain the existing scenario/e2e interfaces and useful branch additions: isolated
vault/account preparation, explicit fixture loadouts, seeded-idle and sustained
workloads, panel/TUI launch integration, navigation sharing, terminal evidence,
Stop/error handling, and portable basic resource sampling. Preserve the existing
LIVE/local-service guard, readiness requirement, meaningful progress checks and
failure exit behavior. These are product contracts even when their users are
another project's tests.

The new seed-sharing path depends on `Run`/`SeedNav` in
`crates/host-play/src/memory.rs`, which does not exist on main. Therefore
`7d026bc`/`606c93b` alone are not a valid cherry-pick plan. Extract the needed
harness foundation from `e6ec9e6`/`e1ebd59`, the required `a52b12c` setup/bridge
pieces, and subsequent functional changes including seeded-idle `d3abfb4`,
system-allocator mode `c4943a7`, and N16/render-policy support `9f687af`.
Use the final seed-sharing behavior, including bind-before-spawn and
`FromPlay(None)` without another decode. Keep terminal cleanup and native fixture
corrections with that foundation.

Preserve focused unit/integration tests beside this code. The final test-only
TUI preparation fix (`4b1d0bb` → `273ecd4` → `f9675b6`) is also worth retaining:
unit fixtures should not launch real network workers and hang on teardown.
Extract the final combined diff, not its superseded intermediate designs. Its
implementation report records blocked test execution at that boundary, so the
selected build must actually run those tests before treating them as verified.

Keep existing basic harness outputs and required optional accounting where used;
do not silently turn formerly measured fields into fabricated zeros or rename
the supported entry points. Inventory any externally consumed output fields
before pruning a dependency. This is extraction of existing working behavior,
not a proposal to build a new "minimal harness" framework. Detailed latency
journals, per-owner census, native admission controllers, replay experiments,
and machine provisioning remain campaign tools. Their absence must not prevent
the promoted Rust harness from running directly against its supported fixture.

## Leave these on the campaign branch

| Work | Assessment |
|---|---|
| Snapshot deduplication (`2d12d68`, `7c05f5c`, `d1de456` and follow-ups) | Adds registries, weak ownership, equality rules, feature combinations, and cross-crate lifecycle wiring. Mechanism tests are useful, but there is no demonstrated deployed benefit sufficient to justify maintaining it here. Keep it out entirely, including the disabled feature. |
| Borrowed fingerprint comparison (`188a520`) | Sensible idea, but substantial field-by-field comparison/retention logic and no qualified 16-slot performance candidate. The failed observation does not prove the optimization caused a gameplay bug: the later analysis records banking and movement despite zero additional steals. Park for insufficient payoff evidence, not an invented correctness verdict. |
| Tiled navigation (`c3d8086`, `8385bab`) and coordinate refinement (`ebf0f30`) | Real counting evidence removes 66.01 MiB of shared collision storage; the isolated world-live probe also shows lower RSS. This is real work, not merely a theoretical saving. However, the representation changes consumers across several crates and full routing cost remains unqualified. The coordinate experiment compares two tiled implementations, not the original dense baseline. Leave out of this batch. |
| Boxed scene tiles | Longer focused comparison found candidate RSS about 0.6% higher in the common window and 8% higher in the final 300 seconds. No sustained focused saving was established. The inspected campaign client ancestry does not contain the tested tile candidate; do not fetch it into the extraction just because reports exist. |
| Advanced latency journals, direct-owner census, privileged native identity adapters, VM/setup and campaign capture/replay tools | Not required to deliver the selected fixes or run the reusable harness. Leave here. Retain basic resource reporting and any small existing accounting dependency actually required by the promoted harness; do not confuse those with this external proof system. |
| Instrumentation-only corrections, such as TUI input-origin attribution `f946901` | Correctness fixes to campaign measurements, not established repairs to ordinary key handling. Leave with the corresponding instrumentation unless an actual retained harness interface depends on them. Do not label these as product input fixes. |

Sources: [shared-navigation scope](shared-play-nav-world-report.md),
[navigation confirmation](shared-nav-confirmation-report.md),
[snapshot implementation](snapshot-dedup-production-landing-report.md),
[fingerprint disposition](borrowed-fingerprint-decision-analysis.md),
[tiled cold screen](nav-stage-a-cold-screen-report.md),
[coordinate experiment](nav-coordinate-lookup-experiment-report.md),
[latest failed coordinate admission](nav-coordinate-cf2-failure-report.md),
[lazy uploads](panel-lazy-upload-report.md),
[lazy-upload comparison](windows-lazy-upload-comparison-report.md), and
[long tile comparison](windows-tile-boxed-long-screen-report.md).

The tiled startup result deserves perspective: **109 ms became 453 ms**. A
316.6% regression sounds dramatic, but an extra 0.34 seconds at startup could be
a perfectly reasonable price for 66 MiB in a hobby application. I would not reject
it on that percentage alone. The unresolved hot-path tradeoff and wider maintenance
cost are why it stays out now. Accepting that tradeoff would be a separate choice;
there is no need to restart its elaborate proof machinery to finish this harvest.

## Extraction plan (approved September 9, 2026)

1. Freeze the approved source commits. Start a clean host integration branch from
   the verified main tip and a separate client integration branch from the client
   revision that main pins. Keep the campaign checkout and its evidence intact.
2. Prepare these client code commits in this order:

   | Order | Existing client commit | Content |
   |---|---|---|
   | 1 | `ca0a36fafdd53ae4d47bada16f8d9362e480c9a9` | Dynamic sprite reuse |
   | 2 | `12c2061a1f8ef701989e3d22c4b4493e2e480653` | Scalar animation delay |
   | 3 | `bc8bb4e2a6626e15d8dc50f24b3d8ef70c12788e` | Viewport overlay invalidation |
   | 4 | `451759f2a7df9c57895657d5b8d506172860cee1` | Held minimap during overlay-only uploads |
   | 5 | `85266df125487d7669320d53071eedc14caf9f89` | Sparse appearance packets |
   | 6 | `e17deab9085b97b0af42dc1329d7b88a0c7cb467` | Private animation-base sharing |
   | 7 | `35f1c131990b16c3fed541e79d753a52b60a2f98` | Windows socket readiness |
   | 8 | `4b3530049a2596f5081efb2f1345d35cbf2053e8` | Windows home fallback |
   | 9 | `2b1af850d1b39833a60508ceb73bf31c6df80cb9` | Standalone dependency lockfile correction |
   | 10 | `3456edc8dabf7b25ada78110ffa56327af9f67a4` | Sealed main-modal restoration |

   These contain source, focused tests and dependency files. Preserve that
   coverage. Animation sharing builds on the scalar-delay method; the render
   fixes are now deliberately included. The two remaining client commits in the
   inspected range are profiling `b555f84` and owner capture `5c73a4a`, not part
   of this selected client sequence. If the harness needs a basic reporting API
   from the former, identify its exact dependency before selecting additional
   code; the separate direct-owner feature remains excluded.
3. Prepare host **code-only extraction commits** by capability, using the source
   groups above. The mixed checkpoint `a52b12c` must be split, not cherry-picked
   wholesale. Recommended grouping/order: routing/banking/door correctness;
   harness foundation with sharing and terminal cleanup; focused reconnect;
   lazy uploads plus pixel correction; Windows sockets/home/metrics/fixtures;
   explicit-Stop snapshot cleanup. Retain final corrected behavior and focused
   tests within each group; build the combined dependency set before declaring
   the prepared commits independently usable. This is a source map, not a claim
   that every historical host commit can be applied directly to main.
4. Add a separate host commit pinning the final selected client revision. **Do
   not cherry-pick historical host gitlink commits**: they point at cumulative
   client history containing excluded work. Resolve Cargo manifests/lockfiles
   for the actual selected features and target-specific dependencies. Keep the
   dense navigation representation; port selected tests/call sites back to it
   where later source assumes the excluded tiled representation.
5. Perform the bounded validation below and the required final Grok 4.6 review
   of the actual selected host/client diff. Earlier campaign reviews support
   the evidence; they do not approve this newly assembled combination.
6. Publish the selected client history to the authorized client fork so the
   final gitlink is fetchable. Complete the requested host squash merge only
   after the selected result is approved. Then update main's documentation to
   describe what actually shipped and its measured limits. Preserve source-SHA
   mapping in the merge description; leave this assessment and campaign tools here.

Extraction feasibility checked in temporary copies: all ten selected client
patches apply in the listed order without the profiling/owner-capture commits.
On the host, Stop cleanup, both lazy-upload patches, focused reconnect and both
door-edge patches apply in sequence to main. The Windows socket patch encounters
manifest/lockfile context conflicts. Seed sharing fails directly because main
lacks `memory.rs`; standalone bank-wait application encounters test-context
dependencies from the earlier bridge work. These are reasons for the explicit
host extraction groups, not reasons to drop the requested functionality.

Temporary files were removed. **Patch applicability is not compilation or a
combined-behavior result.** No code commits, branch switches, builds, live runs,
merges, or remote changes were performed for this assessment.

## A proportionate completion check

Approval of this plan replaces the old campaign matrix as the prerequisite for
this selective merge. It does not waive failures or convert historical diagnostic
results into accepted benchmarks.

- Run the existing affected tests on the extracted combination: client library
  tests and separate `seq_delay`, `player_info`, `login`, `inject`, `zone`, and
  renderer integration tests; API/nav, host/login/socket, host-play, scenario,
  panel and TUI tests; `script` tests with `load`. Build ordinary TUI/panel and
  the retained harness feature combinations, including system-allocator mode.
  A host test run alone does not run the client's integration tests.
- Check the existing supported environments: macOS, native Windows/MSVC and
  Linux. Reuse installed environments; no new provisioning. Run native socket,
  home, resource and affected harness tests, plus a short TUI/harness smoke per
  OS and panel/rendering checks on the available native desktop backends.
  Record exactly which frontends/backends were exercised. Do not turn this
  into cross-platform performance certification or claim universal GPU support.
- Use existing live facilities for one bounded comparison of main versus the
  extracted build on the same machine, ordinary workload and settings. Include
  a small normal session and a 32-bot session to revisit the scalar experiment's
  actual benefit and timing concern. Check useful progress, process CPU/current
  RSS, responsiveness, scene/minimap freeze, focus changes, and Stop/restart.
  Exercise CPU fallback once and GPU-to-CPU-to-GPU view switching. Reuse campaign
  comparison tools without shipping them; ship the reusable Rust harness itself.
- Functional acceptance must cover exact bank restocking and return, contested
  door crossing, reconnect priority, and terminal evidence/Stop semantics. For
  the pack fix, bake and validate a pack with the selected code and documented
  inputs; do not reuse an old pack and claim the edge correction was exercised.
  A controlled stun case must actually trigger recovery before that path is
  described as live-proven. Existing failed and partial evidence stays visible.
- Keep comparison purposes separate: main may fail the very gameplay case being
  fixed. That is meaningful correctness evidence, not an eligible performance
  baseline. Do not weaken progress checks to obtain a number. Earlier campaign
  runs do not validate the newly extracted combination; confirm its supported
  harness entry points directly, without requiring archived campaign controllers.
- No new instrumentation framework, platform provisioning, 36-cell matrix,
  hour-long soak, or 128-bot capacity gate for this release. A failed check gets
  one bounded diagnosis; unresolved implicated changes are left out or brought
  back as a concrete tradeoff. Missing evidence is reported, not replaced by
  repeated runs until favorable. Finish with the required review, then stop.

Those are prospective integration checks only; none was launched in this review.
There is no promise of a combined percentage improvement until that combined
build is measured. Likewise, this release should make no new low-end or large-fleet
capacity guarantee.

## Why the work expanded, and better guidance next time

The evidence supports your concern, but the entire effort was not wasted. The
early allocation fixes are useful. Tests, native measurements, and independent
reviews also uncovered genuine behavior defects and invalid performance claims.

The mismatch was the **completion contract**. The approved finish plan made a
wide set of memory, CPU, tail-latency, renderer, lifecycle, platform, and fleet-size
requirements prerequisites to finishing. When measurements could not satisfy or
even reliably evaluate them, measurement infrastructure became another product.
Individual experiments had stop rules; the overall campaign still had to satisfy
every budget. There was no comparable total effort or maintenance-cost ceiling.

For scale, this host branch differs from local main by 1,460 tracked files and
about 901,000 inserted lines, much of it evidence data. The host crate/test diff
alone adds about 31,400 lines, excluding the client submodule contents. These are
scope indicators, not counts of production complexity or an estimate of time
spent. They explain why the branch is unsuitable as one release unit.

You could have made the tradeoff clearer with guidance such as:

> This is a hobby project. Spend at most two focused days finding a few material
> improvements. Prefer local allocation and ownership fixes. Architecture changes
> or substantial new measurement tooling require a separate cost/benefit decision.
> Treat resource budgets as goals, not mandatory release gates. After one
> inconclusive follow-up, park the candidate and ship the proven subset. Bring me
> absolute costs and user-visible effects, not just percentages. Do not expand
> platforms or fleet sizes without a new scope decision.

The exact time limit is yours to choose. The key is a global stopping point and
permission to finish with useful partial gains. You should not have had to predict
every detour: the agent should also have surfaced the rising cost of proof and
offered a smaller harvest earlier. My first assessment also drew the product
boundary too narrowly by treating the harness and platform fixes as expendable.
Your clarification corrects that: preserve the useful application and preservation
harness, while stopping the expensive search for further performance claims.
Preserving behavior and platform support was reasonable guidance; it did not
require pursuing every possible performance certification.
