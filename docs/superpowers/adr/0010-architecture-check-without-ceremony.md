# ADR 0010: The architecture check without the admission ceremony

**Status:** direction set by the operator 2026-10-15 ("it's mostly the architecture gate … let's fix that"); this design is Main's, in effect unless the operator objects. Supersedes the protected-enforcement parts of ADR 0007 and §26 (epochs, exact-byte approvals, hash-chained receipts, qualification of proposed enforcement, pending records, hosted replays).

## Context

The F0 gate admits every `dev` merge through `trusted_gate.py`:
- an active enforcement pinned by receipt;
- an epoch;
- exact-H approvals published by the operator;
- a hash-chained receipt per admission;
- for any change to the checker itself, qualification of the proposed enforcement against known-bad changes, inside a sandbox, under a 300 s per-probe deadline.

It took several days to bring up, and keeps costing time:
- **The hosted runner never matched local qualification.** Its Git 2.55 maintenance race kept CI red for a day and took an 8.8 h fix round.
- **A parser limitation blocked a legitimate change** for want of a checker transition.
- **Ceremony per change:** each client gitlink move, ownership row or seam type needs an approval round.
- **Fragile roots:** stray `__pycache__` dirtied the enforcement root twice.

The one property the gate needs: **a candidate change can't alter the checker or the rules that judge it.** Everything else the ceremony adds protects against a compromised orchestrator or provides a cryptographic audit trail. That isn't worth its daily cost for this project.

Reward hacking is handled where it has been caught in practice:
- cross-vendor reviews that run the changed path;
- live cells on the real engine;
- tests that fail on the old code;
- the rule that a gate is never loosened to pass.

## Decision

1. **The checker and the crate-dependency policy live only in `acfrazier/274bot-arch` `main`** (operator choice B, 2026-10-15). That covers `check.py`, `graph.py`, `inventories.py`, `ownership.py`, their tests, and the crate-edge policy.
   - Agents' scoped token can't push that repo.
   - A checker or crate-edge change is an ordinary commit the operator pushes. These are rare and structural.
   - No epochs, approvals, receipts, pending records or qualification.
   - **The fine-grained rule data stays in the product repo** next to the runner: seam types, ownership rows and inventories. It's declared in the same diff as the change that needs it and judged by the cross-vendor reviewer, not the operator. That's the rubber stamp this ADR removes.
   - The checker still refuses any undeclared seam crossing or ownership gap. Product data can never widen the protected crate-edge policy.
   - Every `Cargo.toml` below `vendor/fr-client-rust/` is an exact protected input in `274bot-arch/vendor-manifests.json`.
   - One digest covers the sorted path and exact-byte pairs. Added, removed, or changed manifests require an operator commit to update the pin.
   - The digest is identity data for a protected input, like policy data, not bookkeeping.
   - This replaces interpretation of the client workspace's members, exclusions, globs, and inheritance. The pinned set has no `workspace = true` dependencies.
   - Generate the replacement with `python3 -B vendor_manifests.py --emit-pin /path/to/274bot > vendor-manifests.json` in the arch repository.
   - The checker refuses a `git` key in any dependency, target, workspace-dependency, `[patch.*]`, or `[replace]` entry. The refusal names the manifest file and the entry. The checker never reads a git crate's manifest.
   - Only root `[patch.*]` and `[replace]` entries listed by name and path in `policy.toml` `[root_patches]` may appear in the product root. The list now holds only `dear-imgui-wgpu`. Any other entry needs an operator commit that adds it.
   - Registry dependencies stay allowed. A crates.io crate cannot name a local path package without a root patch, and the root patch list covers that route.
2. **The product repo holds no copy of the checker.** `tools/architecture/` keeps a small runner, `run.sh`, and the fine-grained data files. The runner fetches `274bot-arch` `main` (public, read-only) into a cache and runs it against a tree. Agents and Main use the same runner locally.
3. **CI** has one `architecture` job on pushes and PRs:
   - it checks out the candidate;
   - it fetches `274bot-arch` `main` and runs the checker against the candidate.

   The workflow file is already protected: agents' token can't change workflows. It pins the parser (rust-analyzer `fd89304c…`) and Python, as now. There's no sandbox, no B/H transition machinery and no records.
4. **Merging to `dev`:**
   - focused tests and the relevant suites (or the full matrix for wide changes);
   - the cross-vendor review;
   - a clean run of the checker from `274bot-arch` `main`;
   - Main pushes.

   A new seam type or ownership row is part of the change's own diff. New crate edges, client manifest changes, and checker changes need an operator commit in `274bot-arch` first.
5. **Client gitlink moves are ordinary reviewed diffs when every vendored `Cargo.toml` remains byte-identical.** Manifest changes also need an operator commit to the protected pin. Fork review enforces Java conformance, not the gate.

## Retired

- `trusted_gate.py`, `e2e_trusted_gate.py`, `test_trusted_gate.py`;
- the `arch-enforcement` and `arch-gate-records` branches (kept as history, not used);
- `ADMISSIONS/` scripts;
- epochs and receipts;
- the CI `architecture` job's protected fetch, pins and evidence steps;
- the advisory PR lane, now folded into the single job.

**Kept as evidence:** the F0-CORE directories and the records history.

## Consequences

- A wrong rule can be fixed in one commit. A checker bug no longer blocks merges for a transition cycle.
- A compromised orchestrator could push `dev` without running the checker. CI still runs it on every push and shows red; there's no cryptographic proof of who ran what. Accepted.
- The checker's useful in-flight improvements carry over as ordinary `274bot-arch` commits:
  - the `PhantomData<fn..>` marker rule;
  - evaluation-local scan reuse, if still wanted for speed.

  The GC-race fix dies with `trusted_gate.py`.
- The checker still refuses ASCII case variants of `.cargo` configuration names, at every depth and through symlinks.
- Unicode look-alike names are outside that refusal because Cargo on Linux does not read them.
- A product build script can rewrite a vendored manifest on disk during a CI job, between steps. The checker reads committed manifests only. A new `build.rs` needs an ownership row, so a reviewer sees it.
