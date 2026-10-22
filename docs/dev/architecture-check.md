# Architecture check in detail

> **Audience:** contributors who change a crate dependency, a Cargo feature or
> the architecture data files, and anyone who needs to reproduce the CI
> architecture check on their own machine. The short version is in
> [CONTRIBUTING.md](../../CONTRIBUTING.md#run-the-checks). This text moved here from
> CONTRIBUTING.md; only relative links changed.

```bash
# Crate-graph ownership (local Python 3.11+ minimum; CI pins 3.14.8). Does
# not replace fmt/clippy/test. The checker and the graph-creating policy
# live in operator-owned `274bot-arch` `main`; `run.sh` is a
# local-convenience-only runner — CI does not use it and never executes a
# candidate-tree script. Locally it fetches the rules into a bare-Git cache
# (`ARCH_CACHE_DIR`, default
# `${XDG_CACHE_HOME:-$HOME/.cache}/274bot-arch`) and runs them against a
# tree (`--root`, default the product root), reading the fine data
# (`tools/architecture/ownership.json`, `inventories.json`,
# `foreign_types.json`) from the checked tree. `ARCH_REPO` is a local-only
# override taking a URL (always `main`) or a local clone (its local `HEAD`,
# for pre-push proof); CI has no `ARCH_REPO` override.
tools/architecture/run.sh
tools/architecture/run.sh --root /path/to/tree
```

Crate edges are an explicit allowlist: the protected
[`policy.toml`](https://github.com/acfrazier/274bot-arch/blob/main/policy.toml)
in the operator-owned [`274bot-arch`](https://github.com/acfrazier/274bot-arch)
repo (workspace members, edges/kinds/migrations, configuration
feature/activation — see [docs/architecture.md](../architecture.md)).
An operator commit in protected `274bot-arch` is required for a new crate
edge (including a new target cfg or cross-crate feature request), an edge kind,
a migration, an optional-dependency activation, a vendored path package, or a checker change.
The operator edits `policy.toml` where needed.
First merge current `dev` into the branch being merged.
In the same arch commit, regenerate `graph-provenance.json` from that merged branch tree:
`python3 -B check.py --emit-provenance-baseline /path/to/merged-branch > graph-provenance.json`.
Review removed records. Retain records that other active branches still need.
The operator publishes first. Propose the edit as a patch or branch bundle.
Agents cannot push that repo.
Purely local features with no optional dependency or cross-crate edge, and
ownership/inventory/foreign-type rows, need no operator commit. These fine-data
rows are ordinary product diffs, reviewed like code. New reverse
or convenience dependencies, workspace aliases, target tables, and unknown
workspace members fail closed. Optional-only allows do
not accept a required target dependency. Membership is the explicit
`workspace.members` paths plus a root `[package]` if present; globs are
unsupported and fail closed. Lifecycle, FlatBuffer isolate/host wire,
last-FBO, and JS-shim policy are review requirements, not this checker.

GitHub Actions runs the same two manifests after installing ALSA + X11
headers (`libasound2-dev` — panel pulls client `audio` / cpal), plus the
feature-gated test lanes (script `load` and V8-free `--no-default-features`,
host `performance-profile`, host-play/panel/tui `memory-profile`, host-play
`memory-profile-no-alloc`, panel `render-diagnostics`) and the inline
architecture check: the step itself runs `git clone --depth 1 --branch main
https://github.com/acfrazier/274bot-arch.git "$RUNNER_TEMP/274bot-arch"`,
verifies the rust-analyzer binary (SHA-256
`fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49`,
`rust-analyzer 0.3.3081-standalone`), then runs `python3 -B
"$checker_root/check.py" --self-test` and `python3 -B
"$checker_root/check.py" --root "$GITHUB_WORKSPACE"`. It never executes a
candidate-tree script (including `tools/architecture/run.sh`) and has no
`ARCH_REPO` override. `path-schema`, `live-harness` and `test-support` are
exercised through the workspace test lane's feature unification. The separate
`grinch-oracles` lane explicitly enables `live-harness,test-support` and runs
`cargo test -p host-play --features live-harness,test-support --test grinch_live`,
so the offline Grinch oracles do not depend on workspace feature unification.
No CI lane runs `LIVE=1` tests. Independent test commands run as parallel matrix lanes;
the required `test` status aggregates every lane. It is still a **subset**:
`SKIP_GPU=1` (no adapter on those VMs) and never `LIVE=1`. A green GH job is
not a headed or engine pass.
