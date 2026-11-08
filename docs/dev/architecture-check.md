# Architecture check in detail

> **Audience:** contributors who change a crate dependency, a Cargo feature or
> an API seam, and anyone who needs to reproduce the CI architecture check.
> The short version is in
> [CONTRIBUTING.md](../../CONTRIBUTING.md#run-the-checks).
>
> Local and hosted checks use protected `274bot-arch` `main`, including on
> release lines. There is no per-release checker branch. A local prepared clone
> can be selected for an operator proposal; hosted CI cannot select candidate
> checker code.

```bash
# Protected graph and API boundaries (Python 3.11+; CI pins 3.14.8).
# This does not replace fmt, Clippy or tests. The operator-owned checker and
# rules live in `274bot-arch` main. This local runner fetches that copy into
# ARCH_CACHE_DIR (default ${XDG_CACHE_HOME:-$HOME/.cache}/274bot-arch).
# --root selects the candidate product tree. Only semantic foreign-type
# classification comes from tools/architecture/foreign_types.json; source-file
# ownership, exhaustive type/test inventories and provenance baselines are gone.
# ARCH_REPO is a local-only URL or prepared-clone override. CI has no override
# and never executes a candidate-tree checker script.
tools/architecture/run.sh
tools/architecture/run.sh --root /path/to/tree
```

Crate edges are an explicit allowlist: the protected
[`policy.toml`](https://github.com/acfrazier/274bot-arch/blob/main/policy.toml)
in the operator-owned [`274bot-arch`](https://github.com/acfrazier/274bot-arch)
repo (workspace members, edges/kinds/migrations, configuration
feature/activation — see [docs/dev/architecture.md](architecture.md)).
An operator commit in protected `274bot-arch` is required for a new crate
edge (including a new target cfg or cross-crate feature request), an edge kind,
a migration, an optional-dependency activation, a vendored path package, or a checker change.
The operator edits `policy.toml` where needed.
Propose the rule change as a private patch or branch bundle. The operator
publishes it with the corresponding product change; agents cannot push that
repository. There is no provenance-baseline emitter or record refresh.
Ownership classes are derived from syntax. Existing seam and import exceptions
are exact protected entries, not candidate-side inventory rows. Candidate code
cannot add its own exception. Semantic foreign-type classification remains
ordinary reviewed product data. New reverse
or convenience dependencies, workspace aliases, target tables, and unknown
workspace members fail closed. Optional-only allows do
not accept a required target dependency. Membership is the explicit
`workspace.members` paths plus a root `[package]` if present; globs are
unsupported and fail closed. Lifecycle, FlatBuffer isolate/host wire,
last-FBO, and JS-shim policy are review requirements, not this checker.

GitHub Actions runs the same two manifests after installing ALSA + X11
headers (`libasound2-dev` — panel pulls client `audio` / cpal), plus the
feature-gated test lanes (`script-js` tests and V8-free `script --no-default-features`,
host `performance-profile`, host-play/panel/tui `memory-profile`, host-play
`memory-profile-no-alloc`, panel `render-diagnostics`) and the inline
architecture check: the step itself clones protected `main` with
`git clone --depth 1 --branch main https://github.com/acfrazier/274bot-arch.git
"$RUNNER_TEMP/274bot-arch"`, verifies the rust-analyzer binary (SHA-256
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
