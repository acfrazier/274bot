# Protected architecture gate

`trusted_gate.py` admits local orchestrator merges and runs the GitHub detective check. It reads product trees as data. It loads the graph, ownership, and Rust-inventory checkers only from the exact protected enforcement checkout. It never imports a checker, helper, shell step, or build script from candidate H.

The gate implements §26 §4.1–4.4. The product target is a data baseline, not an executable trust root. The approved checker is stored outside the product repository at `refs/heads/arch-enforcement`. A separate trusted records ref stores bootstrap identity, receipts, approvals, and an optional pending enforcement transition.

## Required trusted stores

Provision these stores before using the local gate or enabling the CI job. The simplest compliant setup is **one dedicated repository with two branches**, readable anonymously over HTTPS (the push job clones it without a token):

- `refs/heads/arch-enforcement`: its tip must **be** the active E commit, not merely contain it. E only needs `tools/architecture/` and `.github/workflows/ci.yml`; an orphan commit holding exactly those files from the product commit is enough and has the same bundle digest. Keep every E commit reachable so a run can retrieve the active E SHA from the records chain. Only the operator/orchestrator may advance it.
- `refs/heads/arch-gate-records`: the records tree. For each product target, store files under `targets/<branch>/`: `gate.json`, `receipts.jsonl`, `approvals/<H>.json`, and, only during an enforcement update, `pending.json`. For example, the `refs/heads/release/0.2` target uses `targets/release/0.2/`. Only the operator/orchestrator may push it.

Protect both branches against force-push and deletion. The repository must be separate from the product repository; the product repository never stores these trusted values.

Runtime identities:

- **rust-analyzer**: the exact bytes with SHA-256 `88ae4c54c68d6899010c18074b9e415f2dbee09527fb703fd4536b665571db0a`, whose `--version` prints `rust-analyzer 1 (9074e9b4c6 2026-09-06)`, served at any HTTPS URL that needs no authentication (for example a release asset on the arch repository). The source is the orchestration host's `/usr/bin/rust-analyzer` (Arch package `rust-analyzer 20260907-1`), not a rustup/nightly build. CI downloads it, checks the SHA-256, and checks the version. **Known blocker:** this build imports `log10f@GLIBC_2.43`; `ubuntu-24.04` runners ship glibc 2.39, so the version check fails there. Before enabling CI, either run the push job on an image with glibc 2.43 or later, or pin a parser build that runs on `ubuntu-24.04`. Both change protected bytes or the runtime pin and therefore need a qualified enforcement update. Confirm with `"$parser" --version` inside the exact runner image.
- **Python**: pin `python=3.14.8`. `actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1` is v6.3.0; it resolves versions from `actions/python-versions@main/versions-manifest.json`, which lists 3.14.8 as stable with `python-3.14.8-linux-24.04-x64.tar.gz` (checked 2026-10-12). The orchestration host runs CPython 3.14.8, so the local gate and CI use the same pin. A different host interpreter fails closed; changing the pin needs a qualified enforcement update.

The full runtime pin is `python=3.14.8;rust-analyzer-sha256=88ae4c54c68d6899010c18074b9e415f2dbee09527fb703fd4536b665571db0a`.

The workflow requires these GitHub repository variables on the product repository:

| Variable | Value in the simplest setup |
|---|---|
| `ARCH_RECORDS_URL` | HTTPS clone URL of the arch repository. |
| `ARCH_RECORDS_REF` | `refs/heads/arch-gate-records`. |
| `ARCH_ENFORCEMENT_URL` | The same HTTPS clone URL. |
| `ARCH_ENFORCEMENT_REF` | Exactly `refs/heads/arch-enforcement`. |
| `ARCH_RUST_ANALYZER_URL` | HTTPS URL of the exact rust-analyzer bytes. |

The push job also clones the product repository and the client submodule without a token; both are currently anonymous-readable. Missing variables, refs, records, runtime pins, parser bytes, or matching checkouts fail closed. Do not store these trusted values in the product checkout or accept a candidate's pin, version, `approved` field, or CI result as authority.

## Bootstrap

There is no self-service bootstrap command. The operator must create the first records commit out of band from verified inputs, then publish it on the protected records ref. Bootstrap approval must bind all of the following in one operator-approved record:

- target ref and exact accepted target commit B;
- exact enforcement commit E and the bundle digest computed by `trusted_gate.bundle_digest(E)`;
- exact Python runtime pin and the fixed rust-analyzer digest;
- `contract_epoch`, initially `0`;
- schema versions computed from B by `trusted_gate._schema_versions(B)`;
- `input_digest` computed from B by `trusted_gate.input_digest(B)`;
- `orchestrator_commit`, equal to B;
- a nonblank reason, operator inline approval, and nonempty affected-owner acknowledgements.

The `gate.json` top-level object has exactly `format`, `schema_version`, `target_ref`, and `bootstrap`. The format is `f0-architecture-gate-v1`; the schema version is `1`. The bootstrap object has exactly `accepted_sha`, `contract_epoch`, `enforcement_sha`, `bundle_digest`, `runtime_pin`, `schema_versions`, `input_digest`, `orchestrator_commit`, `reason`, `operator_inline_approval`, and `owner_acknowledgements`. Start `receipts.jsonl` as an empty file. Commit the records to the protected records ref. Do not initialize the accepted B from a product-supplied manifest or a candidate branch.

For a trusted local tool checkout E, compute the protected identities without importing product modules:

```sh
export E_ROOT=/absolute/path/to/clean/enforcement-checkout
export B_ROOT=/absolute/path/to/clean/accepted-B-checkout
export PYTHON=/absolute/path/to/the/pinned/python
"$PYTHON" -I -S -c 'import sys; sys.path.insert(0, sys.argv[1]); import trusted_gate as g; from pathlib import Path; e=Path(sys.argv[2]); b=Path(sys.argv[3]); print("enforcement_sha=" + g._git(e, "rev-parse", "HEAD")); print("bundle_digest=" + g.bundle_digest(e)); print("schema_versions=" + repr(g._schema_versions(b))); print("input_digest=" + g.input_digest(b))' "$E_ROOT/tools/architecture" "$E_ROOT" "$B_ROOT"
```

The operator must independently confirm the target ref, exact commits, runtime, owner acknowledgements, and approval source before publishing bootstrap. This snippet only computes hashes and versions; it does not create trusted records or approve them.

### Operator bootstrap checklist (target `dev`)

Run each step on the orchestration host. Use `PYTHON=/usr/bin/python3` (CPython 3.14.8) and the host `/usr/bin/rust-analyzer`.

1. **Merge first.** The gate must be on `dev` before bootstrap. Push that merge as B. Its own push run is red, because no records exist yet; the first green run is the first admitted H.
2. **Create the arch repository.** Make it public, give push access only to the operator, and protect `arch-enforcement` and `arch-gate-records` against force-push and deletion. Set `ARCH_URL=https://github.com/<owner>/<arch-repo>.git`.
3. **Publish E.** From B, copy only the enforcement bundle into a new empty directory and commit it as an orphan:
   ```sh
   mkdir "$E_ROOT"
   git -C "$PRODUCT" archive "$B_SHA" tools/architecture .github/workflows/ci.yml | tar -x -C "$E_ROOT"
   git -C "$E_ROOT" init -q -b arch-enforcement
   git -C "$E_ROOT" add -A
   git -C "$E_ROOT" commit -q -m "Architecture enforcement from $B_SHA"
   git -C "$E_ROOT" push "$ARCH_URL" HEAD:refs/heads/arch-enforcement
   ```
4. **Publish the parser.** Upload `/usr/bin/rust-analyzer` unchanged as an HTTPS asset, for example a release asset on the arch repository. Check `curl -fsSL "$ARCH_RUST_ANALYZER_URL" | sha256sum` against the pin. Resolve the glibc blocker above before you expect a green push run.
5. **Check out B.** Make a clean detached checkout of `$B_SHA` at `$B_ROOT` and run `git submodule update --init vendor/fr-client-rust` in it. `$B_SHA` must equal `git ls-remote https://github.com/acfrazier/274bot.git refs/heads/dev`.
6. **Compute identities.** Run the snippet above with `E_ROOT`, `B_ROOT`, and `PYTHON`. Run it again with `E_ROOT="$B_ROOT"`: both `bundle_digest` lines must match, or E and B disagree.
7. **Write the records.** In a new directory `$RECORDS`, run `git init -q -b arch-gate-records`. Write `targets/dev/gate.json` by hand from the computed values, with these fields: target `refs/heads/dev`; accepted SHA and orchestrator commit both `$B_SHA`; epoch `0`; the runtime pin above; a reason; the operator's inline approval text; and owner acknowledgements. Create an empty `targets/dev/receipts.jsonl`. Validate the result with `"$PYTHON" -c 'import json,sys; json.load(open(sys.argv[1]))' targets/dev/gate.json`. Commit and run `git push "$ARCH_URL" HEAD:refs/heads/arch-gate-records`.
8. **Set the five repository variables** on the product repository (Settings → Secrets and variables → Actions → Variables) to the values in the table above.
9. **Admit the first H.** Prepare H from B. Run `admit` (next section) with `--records-root "$RECORDS/targets/dev"`, records commit = ref SHA = `$RECORDS` HEAD, and `--enforcement-ref-sha` = the pushed `arch-enforcement` tip. Commit and push the new receipt to `arch-gate-records`. Only then fast-forward `dev` to H and push. The push event's `before` must be B and its `sha` must be H.
10. **Verify.** The push job prints `architecture CI pass: admitted push <H> against B=<B>`. Every red run stops the line: do not push past it.

## Local ordinary admission

Use detached, clean checkouts of exact B and would-be merge H. B must equal the target SHA already accepted in `gate.json` plus the verified receipt chain. The orchestrator prepares H from current B; it must not use H's first-parent metadata or a candidate baseline to choose B. Keep the integration checkout at B while running the gate.

First verify that the local enforcement ref still points to the active E recorded in the records chain. Then run the trusted command with these exact inputs:

```sh
"$PYTHON" "$E_ROOT/tools/architecture/trusted_gate.py" admit \
  --records-root "$RECORDS_ROOT/targets/${TARGET_REF#refs/heads/}" \
  --records-commit "$RECORDS_COMMIT" \
  --records-ref-sha "$RECORDS_REF_SHA" \
  --enforcement-root "$E_ROOT" \
  --enforcement-sha "$E_SHA" \
  --bundle-digest "$E_BUNDLE_DIGEST" \
  --runtime-pin "$RUNTIME_PIN" \
  --enforcement-ref-sha "$ENFORCEMENT_REF_SHA" \
  --target-ref "$TARGET_REF" \
  --repo-root "$INTEGRATION_ROOT" \
  --base-root "$B_ROOT" \
  --candidate-root "$H_ROOT" \
  --head-sha "$H_SHA"
```

Set `RECORDS_COMMIT` and `RECORDS_REF_SHA` to the same exact fetched protected records-ref commit. Set `ENFORCEMENT_REF_SHA` to the actual protected enforcement-ref tip. `E_ROOT` must be a clean Git checkout at `E_SHA`; its `vendor/fr-client-rust` worktree state is outside the executable bundle and is ignored. `B_ROOT` and `H_ROOT` must be clean Git checkouts at `B_SHA` and `H_SHA`, with `vendor/fr-client-rust` initialized at its recorded gitlink commit. The gate's source digest binds gitlink identities and ignores checkout-local ignored files; B/H checkout cleanliness rejects changed submodules. `INTEGRATION_ROOT` must have `TARGET_REF` pointing at accepted B. Set `TARGET_REF` to a full ref such as `refs/heads/dev`.

A policy-only addition, widening, or re-admission needs an external approval record at `approvals/<H_SHA>.json`. The approval must name exact H, B, target, contract epoch, active E/digest/runtime, input digest, sorted unique transition findings, reason, owner acknowledgements, orchestrator commit H, operator inline approval, and the source message. Its exact fields and values are validated by the gate. A strict-subset policy removal verified by active E needs the orchestrator-created H but no operator approval and no epoch change. All other policy changes need exact approval. Any change to protected executable bytes requires the E' procedure below.

Two transitions are findings even though no policy row changes:

- Moving the `vendor/fr-client-rust` gitlink to another commit, with the same path, URL, and branch, produces `client gitlink moved: vendor/fr-client-rust <old> -> <new>`. This is an I-wire/T1 custody change (§23 §3). It needs an exact approval like any other widening, and admission records it as a contract transition.
- Owning-view violations record their exact cfg provenance in `cfg_or_target`. An unconditional violation is `production`. A conditional one keeps its predicate text, such as `cfg(any(test,feature="test-support"))`, including module, item, variant, field, and `cfg_attr(..., derive(Clone))` conditions. Deleting the gate changes the id, so the result is a `new inventory violation`, not an empty delta.

On success, `admit` appends one hash-chained receipt to the records checkout. Each receipt's `enforcement_sha`, `bundle_digest`, and `runtime_pin` identify the exact checker/runtime that evaluated H; its `activated_*` fields identify the post-admission pin. For an ordinary or policy receipt they match; for an E' transition, the receipt records active E as the checker and E' as the activated pin. Commit and publish that receipt on the protected records ref before moving the local integration ref to exact H and pushing H. `admit` does not advance the product integration ref. On refusal, it writes no receipt and does not change B, the enforcement pin, or the epoch.

## Protected enforcement update

An enforcement update needs an exact operator-approved `approvals/<H_SHA>.json` record before qualification. It must bind B/H, old E and digest, proposed E' and digest, runtime pins, contract epoch, input digest, exact transition delta, reason, owner acknowledgements, operator inline approval, source message, and orchestrator commit H. H's protected executable surface must match E' byte for byte.

1. With the active E ref still at E, run `prepare-enforcement` using the same common arguments as `admit`, plus the proposed E' checkout and evidence root:

   ```sh
   "$PYTHON" "$E_ROOT/tools/architecture/trusted_gate.py" prepare-enforcement \
     --records-root "$RECORDS_ROOT/targets/${TARGET_REF#refs/heads/}" \
     --records-commit "$RECORDS_COMMIT" \
     --records-ref-sha "$RECORDS_REF_SHA" \
     --enforcement-root "$E_ROOT" \
     --enforcement-sha "$E_SHA" \
     --bundle-digest "$E_BUNDLE_DIGEST" \
     --runtime-pin "$RUNTIME_PIN" \
     --enforcement-ref-sha "$E_SHA" \
     --target-ref "$TARGET_REF" \
     --repo-root "$INTEGRATION_ROOT" \
     --base-root "$B_ROOT" \
     --candidate-root "$H_ROOT" \
     --head-sha "$H_SHA" \
     --proposed-enforcement-root "$E_PRIME_ROOT" \
     --proposed-enforcement-sha "$E_PRIME_SHA" \
     --proposed-bundle-digest "$E_PRIME_BUNDLE_DIGEST" \
     --evidence-root "$EVIDENCE_ROOT"
   ```

   This command runs the active checker on B/H. It then runs `check.py --self-test` and the complete protected `test_*.py` corpus from both E and E', with E' as the checker under test. Finally it runs **admission-mode qualification**: for each probe it builds a fresh copy of the records chain with the exact activation receipt appended, a clone of E', a base at H, and one candidate derived from H. It then runs `E'/tools/architecture/trusted_gate.py admit` as a separate process with the same interpreter, environment, working directory, and command line an operator uses once E' is active. No flag or variable marks that run as qualification. The control (a comment-only workspace manifest change) must be admitted. Each known-bad candidate must be refused with no receipt and no pending record: an always-success `check.py`, an `inventories.py` that returns no findings, a replaced CI workflow, a normal dependency edge outside policy (restored or new debt), and an owning seam with a public field and `Clone` but no inventory row. E' must not execute candidate checker code. The command writes `pending.json` only after all of this passes. Fixtures and temporary directories must live under the evidence directory or the worktree's `target/`, not `/tmp`. Against the real repository, each probe runs a full `admit`, so expect several minutes per probe.

   The probe trees check out `vendor/fr-client-rust` from H's own initialized submodule, so `H_ROOT` needs it initialized, as for `admit`. No network is used. If it is not initialized, the honest control fails and qualification fails closed.
2. Commit and publish `pending.json` on the protected records ref. Publish exact E' to `refs/heads/arch-enforcement`. Do not move the product target yet.
3. Run `admit` again with the active E identity still supplied as `--enforcement-root`, `--enforcement-sha`, `--bundle-digest`, and `--runtime-pin`; set `--enforcement-ref-sha` to the now-published E' SHA, and pass the same proposed E' root/SHA/digest. The gate checks the pending identity, candidate bytes, approval, delta, and qualification digest before it appends a receipt recording E as the checker and E' as the activated pin, then removes `pending.json`.
4. Commit and publish the final receipt/removal on the records ref. Only then move the integration ref to H. Do not admit another candidate while `pending.json` exists.

This version refuses a runtime-pin change. A new Python or parser identity requires exact provisioning and a separately qualified enforcement change; absence of that runtime fails closed.

## CI advisory and detective checks

`ci.yml` keeps the `main`, `dev`, and `release/**` push branches and the standard `pull_request` trigger. On a PR, the advisory architecture lane checks out the PR head and runs its `check.py --self-test` and `check.py`; this executes untrusted candidate code with read-only CI permissions, but is not trusted admission evidence or authorization. The local orchestrator is the enforcement boundary for PRs. The pinned architecture gate runs only on pushes: it fetches E and the records chain from external refs, checks out B/H as source data, requires the exact `.gitmodules` path, URL, branch, and key set before fetching the client gitlink commit, and verifies pushes against the latest admission receipt. Both lanes verify FlatBuffer bindings. The trusted gate does not execute candidate checker code, candidate workflow shell/build scripts, or submodule code; it detects drift after admission and never approves or promotes a candidate.

`ARCH_RECORDS_URL`, `ARCH_RECORDS_REF`, `ARCH_ENFORCEMENT_URL`, `ARCH_ENFORCEMENT_REF`, and `ARCH_RUST_ANALYZER_URL` are external setup; this repository does not configure them. Until the record/enforcement refs and exact parser artifact are provisioned, the push-only pinned architecture job fails closed. The PR advisory lane does not use those protected refs.

## Later F0 integration boundary

`seat-model` is already a known workspace dev-tool node. Main approved exactly one future edge: `script -> seat-model` as a path dependency under `[dev-dependencies]`, with no normal/build edge or feature that turns it into a product dependency. Do not add that edge to the current baseline. When F0-3 lands, add only its `dev`-kind allowance with this exact provenance: `Main 2026-10-12, F0-SCRIPT-SEAM conformance oracle, §27 §2.2 / ADR 0008`. The paired graph fixture rejects the same edge as `normal` or `build`. Any other consumer or non-dev edge needs a new approval.

At that integration, update `ownership.json` with exact rows for the changed seat-model source files and their designated owners. Prepare the candidate from accepted B, run the protected gate under the active E, and obtain an external approval record bound to that exact H, B, input digest, E identity, and exact sorted transition findings before admission. Main's boundary decision approves only the future dev-only edge; it is not an approval for any concrete H and supplies no exact SHA, digest, inline approval, or admission delta.

## Protected fixtures

Run the full trusted fixture corpus from the worktree with all temporary files under `target/`:

```sh
mkdir -p target/architecture-gate-evidence
export F0_ARCH_TEST_ROOT="$PWD/target/architecture-gate-evidence"
export TMPDIR="$F0_ARCH_TEST_ROOT"
export F0_ARCH_TEST_CHECKER_ROOT="$PWD"
export PATH="/path/to/pinned/rust-analyzer-directory:$PATH"
printf '%s  %s\n' \
  88ae4c54c68d6899010c18074b9e415f2dbee09527fb703fd4536b665571db0a \
  "$(command -v rust-analyzer)" | sha256sum --check -
: "${PYTHON:?set the exact Python version from the trusted runtime pin}"
"$PYTHON" --version
"$PYTHON" tools/architecture/check.py --self-test
"$PYTHON" -m unittest discover -s tools/architecture -p 'test_*.py' -v
"$PYTHON" -m unittest discover -s tools/architecture -p 'e2e_*.py' -v
```

The `test_*.py` corpus is what qualification runs. It includes the two-merge always-success and omitted-inventory poison variants, workflow poison, forced-base rejection, stale and forged approvals, approval-free strict-subset removal, a qualified E' control, restored-debt rejection under E', a SHA-only client gitlink move (refused without approval, admitted as an approved contract transition), and removal of a `cfg(any(test, feature = "test-support"))` gate (a new production violation). It also tests exact-approved unsafe E' candidates: an always-success and an omitted-inventory checker; the review r1 E' that is honest only under the old `F0_ARCH_QUALIFICATION_CHILD` flag; and an E' that is honest whenever a test imports it but cheats as the `admit` executable. Each must be refused at `prepare-enforcement` by the admission-mode probes, and no later `admit` may activate it. The corpus never launches a nested fixture suite, so no process in it needs a recursion flag. `e2e_*.py` repeats the two-merge flow and the cheating-executable E' with the real nested suites. It must stay outside the `test_*.py` pattern. These gate fixtures treat candidate code as bytes and assert that candidate checker payloads never execute.
