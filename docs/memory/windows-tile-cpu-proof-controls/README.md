# Native CPU fallback functional-proof controls

These controls are functional/visual proof only. They preserve the configured
CpuPix3D cadence and explicitly select `--cpu-fallback`; they do not optimize or
budget the CPU renderer and never use `--gpu-completion-profile`.

The paired cells are baseline/reference and candidate, N=1, focused-one and
focused-plus-background, with 30s warmup / 120s observation / 60s teardown.
Each cell binds role, mode, CPU backend, fixture/loadout/server/native conditions,
source identities, and binary digest. Owner census, debug, stack logging and
allocation counting are disabled. Navigation and failure captures remain enabled
for functional inspection; performance acceptance is always false.
