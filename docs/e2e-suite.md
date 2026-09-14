# Native e2e suite runner (`e2e-suite`)

`e2e-suite` is the tracked entrypoint for running the native end-to-end cases in one
ordered, resumable run. It drives the *existing* native executables — `panel`'s
`catalog_watch` (single-actor core witnesses) and `pair_watch` (paired witnesses) — and
owns the run ledger, the receipts and the child processes. It contains no scenario engine,
no JavaScript runtime and no gameplay assertion of its own; a green child is a recorded
observation that a human still has to read back.

    cargo run -p e2e --bin e2e-suite -- list
    cargo run -p e2e --bin e2e-suite -- dry-run --level quick
    cargo run -p e2e --bin e2e-suite -- run --level quick \
        --profile local-274 --catalog /path/to/catalog/274 --run-dir /tmp/274-run-1

## Commands

`list` and `dry-run` resolve the manifest, apply the selection and report. They never
start a service, build, log in, play or mutate anything — including the run directory.
`run` executes the selection in manifest order.

## Selection

| flag | meaning |
| --- | --- |
| `--level quick` (default) | vetted, non-manual cases |
| `--level full` | vetted + documented + unvetted (manual and broken stay out) |
| `--level smart` | cases whose declared Rust paths the changed paths touch; a shared path selects every runnable case |
| `--changed-path PATH` / `--changed-paths-file FILE` | the changed paths `smart` maps (supplied explicitly, so verification is deterministic) |
| `--only SUB[,SUB...]` | substring selection over case id, live name, scenario, harness, script and reference case ids; replaces the level; repeatable |
| `--manifest PATH` | manifest override (default: the tracked fixture, embedded at compile time) |
| `--json` | machine-readable `list`/`dry-run` output |

Selection is not execution authorization. Reference cases with no native adapter, excluded
scripts and `BankSorter` (unavailable by operator decision) stay visible with their
reason and are recorded `unavailable`; nothing is substituted or silently dropped.
`--only` matching several cases is intentional, not an error.

## Configuration

`--profile NAME` and `--catalog DIR` are required by `run`; both are validated before any
launch. Also accepted: `--revision`, `--host`, `--port`, `--engine`, `--cache`, `--vault`,
`--lowmem` (the default), `--mainland`, `--exec-core PATH`, `--exec-pair PATH`,
`--cwd DIR`, `--child-arg ARG` (repeatable).

The child command line is the executable, the profile flags and the case's live name:

    cargo run --quiet -p panel --example catalog_watch -- \
        --profile local-274 --catalog /path/to/catalog/274 --live script_thiever

`--exec-core`/`--exec-pair` replace the manifest's cargo template with a direct
executable. Only flags the executables actually accept are emitted: `catalog_watch`,
`pair_watch` and `panel-play` parse flags with `host_play::parse_profile_args` and then
reject anything but `--live`/`--smoke`/`--prod`. `--mainland` is therefore passed as
`BOT_MAINLAND=1` in the child's environment, and `--highmem` is refused because the panel
takes the memory mode from the vault profile and exposes no flag (pending adapter work).
Every child also receives `274BOT_SMOKE_DIR` pointing at the run's `shots/` directory.
Reference `args`/`env` from the frozen manifest are preserved as metadata and are never
applied wholesale; a typed option is added only when a native adapter really consumes it.

## Run directory, identity and resume

`run` requires `--run-dir DIR` and refuses to reuse an occupied directory (state.json or
any other content); `--resume` continues one. The identity is *content-bound* and must
match exactly before any spawn:

* manifest bytes, suite id and the frozen reference commit/tree/archive hash;
* the host and client checkout content (HEAD, index and the working-tree diff are hashed,
  so the same path with different bytes is a different identity);
* every launched executable: a direct `--exec-*` file is hashed, and the manifest's cargo
  template is resolved to the built artifact under `target/{debug,release}[/examples]` and
  hashed too — the suite never records an unresolved command string, so build the executor
  once before the run (or pass `--exec-core`/`--exec-pair`);
* the resolved profile/input configuration with a content digest for the catalog script
  tree (`<catalog>/src/bot/scripts`), the vault file, `--engine` and `--cache`;
* settings (level, `--only`, changed paths, extra child args, child env names) and the
  ordered selection.

An input the suite cannot bind (no built executable, a catalog without a script tree, an
unreadable repository) refuses the run before the ledger exists; a recorded identity with
an unresolved component refuses resume, because such a run cannot prove the input is
unchanged. A changed input *at the same path* — a vault file, a script source — refuses
resume with `profile/input configuration`.

An attempt is written before its child is launched, so a crash or an interrupt still
leaves the case recorded. Resume skips every case that already has an attempt — passed
included — and never retries a failed, interrupted, cleanup-failed or still-running
attempt. A retry would need an explicit new attempt number; there is no silent retry.

    <run-dir>/state.json   ledger: identity, selection, attempts, summary, stop reason
    <run-dir>/logs/        per-case output (capped at 64 MiB, receipt tail 1 MiB)
    <run-dir>/shots/       captures written by the child under 274BOT_SMOKE_DIR

## Verdicts

A zero exit code alone never qualifies a case. The panel prints the *live* name as the
proof name while the scenario and the host enum's wire form identify the case inside, and
the suite checks all three independently:

    PASS: live script_thiever {"scenario":"thiever","outcome":"PASS",...}
    CATALOG_CORE: script_thiever {"case":"thiever","post_start_observations":2,...}

`script_thiever` is `live.name`; `thiever` in the evidence is the scenario; `thiever` in
the witness is `CoreCase::Thiever` serialized snake_case (`air`/`mule`/`flax` for
`PairCase`). A PascalCase witness, or a receipt that names the scenario where the live
name belongs, is rejected.

Each case must show exactly one terminal scenario receipt (never both PASS and FAIL), the
witness the scenario declares, and — when the case declares a capture, or when the
scenario itself declares a terminal shot (the panel holds a PASS until that shot is
written) — a capture *written by this case*: attributed by the before/after shot-root
diff, so a same-labelled file from an earlier case is never reused. A capture counts as
evidence only when it decodes as a real PNG with pixels and its snapshot sidecar was
recorded in game at `scene_state == 2`; file magic and existence are not visual approval.

| status | meaning |
| --- | --- |
| `passed` | terminal receipt plus witness identity, no capture involved |
| `pending_visual_review` | the above, plus captures a human has not read back yet |
| `failed` | a case assertion failure: recorded, the run continues while the shared harness is healthy |
| `shared_failure` | missing/duplicate/malformed/dual receipt, wrong witness identity, an infrastructure signal, a malformed capture or a contracted capture that this case did not write: the run stops |
| `timeout` / `cleanup_failed` | the case budget expired; `cleanup_failed` means the owned tree could not be reaped |
| `interrupted` | the operator interrupted the run; the owned tree was terminated |
| `unavailable` | selected but not executable, with the explicit reason |

The run exits `0` only when nothing was unsuccessful, nothing was left unreached and no
shared stop happened; `1` for any unsuccessful execution, `2` for a usage or configuration
error, `130` when interrupted.

## Process ownership

Every launched child runs in its own process group and is wrapped in an RAII guard, so
success, an early error return and a panic all reap the tree. The wait loop is
deadline-aware (`SIGTERM`, a 10 s grace, `SIGKILL`, then a bounded reap — never an
unbounded `wait`), the log file is opened *before* the spawn, and the output pipes are
drained with a deadline: a descendant that inherits stdout/stderr after the direct child
exits is force-killed to close the write ends, and an abandoned drain is reported in the
ledger instead of hanging the suite. Output is read in bounded chunks; a line past 64 KiB
is emitted wrapped (and marked) rather than buffered without limit. Bounded process-group
ownership is a unix facility: on other platforms `run` fails closed instead of launching a
child it cannot own.


## Verification without a game

The suite's own behavior is checked offline, with the disposable `e2e-suite-fixture`
child standing in for a panel executable:

    cargo test -p e2e --lib
    cargo test -p e2e --test suite_offline

The offline suite (see `crates/e2e/tests/suite_offline.rs` and
`crates/e2e/fixtures/native-suite/offline-suite-manifest.json`) covers printed command
construction, a zero-exit child with no receipt stopping the run, an isolated assertion
failure continuing, a successful case retained as `pending_visual_review` with a real
capture, a contracted terminal shot that never arrives (or arrives under another label)
stopping the run, resume carrying the result without relaunching it, resume refusing a
changed settings/manifest request *and* a changed input at the same path (vault content,
script source) before any launch, refusing an unbindable catalog or executable, budget
expiry with forced-kill process-tree cleanup, interrupt cleanup, a log that cannot be
opened refusing before any spawn, and a descendant holding the pipes not hanging the
suite. The fixture prints the panel's real line contract and writes the scenario's
declared terminal shot. `LIVE=1` harness tests under `crates/e2e/tests/` remain separate
and are not part of a suite run.

## Manifest

`crates/e2e/fixtures/native-suite/suite-manifest.json` is tracked, derived data (regenerate
with `derive.py` against a read-only frozen reference checkout) and is embedded at compile
time, so a run never depends on a local campaign path. It carries the frozen reference
commit/tree/archive hash, the reference case statuses, budgets and coverage as evidence,
the native adapter map (witness identity, live name, variants, declared gaps) and the
desired run options with their pending adapter work. Reference `vetted`/`provenAt` are
historical upstream evidence, never a native PASS.

## Not claimed

This entrypoint does not make Pass 3 complete by itself. Still pending, and visible as
such in the manifest's `defaults.options`: default-ON headed nav paints (no paint flag on
`catalog_watch`/`pair_watch`), a requested-capture flag, per-run `--highmem`, the pair
cells beyond the existing Air witness, the RockCrab stand fixture, and the BoneBurier
external-loader smoke. No LIVE qualification has been run from this slice.
