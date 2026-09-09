# Direct-owner native regression and original GPU attribution

Root evidence boundary, 2026-09-09 20:03 UTC. Functional diagnostics only.

Coverage02 and remaining03 together attempted all19 reviewed gap/feature matrices:
15 commands passed and4 failed. Both host-play feature settings fail the golden
transcript because generated loc IDs differ (0/1 versus4671/4672); the scoped
transcript fixture task t_8d078f01 is pending implementation and same-card review.
Both client integration commands fail gpu_textured_shade_scales_texel_brightness.
The first disk-exhausted run is separate failed evidence, never a passing run.

Root then extracted original client commit3456edc8dabf7b25ada78110ffa56327af9f67a4
from its Git object into an independently hashed archive. Native original GPU
execution fails the same assertion at gpu_texture.rs:485: shade16 expected red
about223, observed255. The frozen candidate and original have byte-identical
GPU texture test, gpu.rs backend and render/world.rs (hashes in preparation
manifest). This establishes the failure exists in the original client on this
builder; it does not prove the hardware/backend cause or qualify final GPU mode.
No renderer changes or assertion tolerance changes were made.

Root ran both complete candidate client test inventories with --no-fail-fast,
so the earlier GPU failure did not prevent later integration harnesses from
executing. Feature-off:66 harness results,767 passed,1 failed,0 ignored.
Feature-on:66 harness results,771 passed,1 failed,0 ignored. Both commands exit101;
the sole failure is the identical GPU shade assertion. Original targeted command
also exits101 (0 passed,1 failed,9 filtered). These are failed commands, not a
fully passing native suite. SKIP_GPU=0 and no forced-no-GPU environment were used.
Actual adapter was not recorded by these tests, so no native backend is inferred
from the older linux-gpu-shade-forensics.md container investigation.

All3 owned process groups were absent afterward with no timeout. Original183
source files were checked before/after and independently against preserved archive
bytes. Candidate1124-file frozen/derived source verification remained identical.
The root wrapper records results independently; its zero process exit means all
three commands and source receipts were recorded, never that tests passed.

Evidence under diagnostics/direct-owner-native-preparation/:
- root-coverage-native-03-audit.json:10 remaining commands,7 pass3 fail; full
  source/log verification for archive SHA3b50cc0b2893aa3203a3115fdada7d225aab9fc82a4eab04309baa29b9ca3b8e.
- root-gpu-baseline-preparation/manifest.json: original commit archive and exact
  three-file GPU source comparison. Original tarSHA448b2ea7e4f76658bed71d5ec1016bc084230894530f9b35073cabd24c67e18b.
- root_gpu_baseline_and_full_client.py: explicit three-command orchestration.
- root-gpu-baseline-and-client-01-audit.json: independent raw-log hash checks and
  recomputed harness counts.
- root-gpu-baseline-and-client-01/{launch.json,steps.json,result.json,logs/}:
  source identities, exact commands/environment, raw failures and cleanup.
- root-gpu-baseline-and-client-01.tar.gz: full original source/raw archive711019B,
  SHA1d3793c0484253820523b0c0194d3beb8a0b3e2ef128d9dfd2586ea420e7f5ed.

Next boundary: reviewed deterministic transcript correction and native rerun;
controller review plus native lifecycle tests; independent acceptance of the
functional evidence and explicit original GPU failure disposition before live
release. Final GPU target qualification remains required. No new live capture,
performance comparison, savings claim, or campaign completion is established.

## Proposed disposition for independent readiness review

Root proposes retaining the exact failed GPU test as an original-client defect
while allowing the bounded TUI owner diagnostic only after the remaining
transcript/controller/native prerequisites pass. This is a proposal for review,
not a release or a conversion of failed commands into passing qualification.

The admitted diagnostic source supports this separation: host/src/lib.rs
slot_want_cpu (lines665-668 in the clean materialization) combines the per-slot
preference with BOT_CPU=1, and Renderer::new_prefer receives !want_cpu at line533.
The same host loop lazily creates renderers only for drawing slots and detaches
them for draw-off slots (lines456 onward). Client render/renderer.rs:40-45 also
makes BOT_CPU=1 override the GPU preference. The TUI diagnostic must retain its
reviewed draw/renderer/environment contract; these source facts do not prove a
future runtime followed it. Runtime receipt/qualification must confirm that.

The original-client reproduction, byte-identical GPU source and complete client
inventory support a pre-existing defect disposition for this one TUI diagnostic.
They do not admit GPU panel measurements, waive final GPU regression/cadence/
visual requirements, or authorize a renderer fix under owner instrumentation.
Independent readiness review must explicitly accept or reject this distinction.
