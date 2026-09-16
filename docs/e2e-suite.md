# Native e2e suite runner (`e2e-suite`)

`e2e-suite` is the tracked entrypoint for running the native end-to-end cases in one
ordered, resumable run. It drives the *existing* native executables — `panel`'s
`catalog_watch` (single-actor core witnesses), `pair_watch` (paired witnesses) and the
dedicated `external_watch` example (the external raw TypeScript loader smoke) — and
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
`--lowmem` (the default) or `--highmem`, `--mainland`, `--exec-core PATH`, `--exec-pair PATH`,
`--exec-external PATH`, `--external-ts ABS`, `--cwd DIR`, `--child-arg ARG` (repeatable).

The child command line is the *resolved* executable, the profile flags and the case's live
name. `--exec-core`/`--exec-pair` and `--cwd` must be absolute; the suite canonicalizes the
program and the effective working directory once and launches exactly those paths.

The manifest's `exec` entry is a cargo template that *names* the artifact
(`cargo run -p panel --example catalog_watch`); it is never the launch program. The suite
resolves it to the built file under `target/{debug,release}[/examples]`, hashes it, and
launches that path, so the bytes in the ledger and the bytes in the child are the same
executable — `cargo run` is not re-invoked after the identity was captured (it could
rebuild different bytes under the same command). `--exec-core`/`--exec-pair PATH` name a
direct executable instead; either way a case whose executable cannot be resolved and
hashed refuses the run (and `dry-run` refuses the plan) rather than printing a template it
would never launch. The external loader smoke resolves its own template
(`cargo run -p panel --example external_watch`) through the same path, or
`--exec-external PATH`, and only when the selection actually contains that case: a
core/pair-only run never resolves, hashes or requires the loader executable or its raw
source.

Only flags the executables actually accept are emitted. The native entrypoints share
panel argument parsing for profile, memory, paints and supported execution modes.
The suite's typed memory selection is consumed by the native adapters and emitted on every core, pair and external child command; it defaults to
`--lowmem`, with `--highmem` as the explicit alternative. Conflicting `--lowmem` and
`--highmem` selections fail closed, and raw `--child-arg --lowmem`/`--child-arg --highmem`
cannot rebind the typed selection. `--mainland` is therefore passed as `BOT_MAINLAND=1` in the
child's environment (and an inherited `BOT_MAINLAND` is removed when `--mainland` is not
set). A nonempty *inherited native deadline control*
is refused too: `BUDGET_S` (the panel's runner-deadline override and post-PASS soak window)
would move the child's inner deadline behind the case budget the run recorded, so `run`,
`--resume` and `dry-run` all refuse it by name before any launch instead of clearing it —
the enforced policy is always the manifest's budget plus the scenario's own deadline. The
refused list is exactly the controls the launched executables consume (`BUDGET_S`); every
other session knob (`BOT_MAINLAND`, `BOT_DEBUG`, `BOT_CPU`) stays untouched.
Input paths must be absolute: a child resolves
a relative path against its own working directory, which the suite cannot reproduce when it
binds the identity, so a relative `--catalog`/`--engine`/`--cache`/`--vault`/`--exec-core`/
`--exec-pair`/`--exec-external`/`--cwd` (or a relative `ENGINE_DIR`/`CLIENT_UNPACK_DIR`/`NAV_PACK`/
`NAV_FLAGS`/`BOT_CACHE_MANIFEST`/`CARGO_TARGET_DIR`) is refused. `--external-ts` is refused
unless it is absolute, which is the rule the panel applies to its own raw-source flag.
Every child also receives `274BOT_SMOKE_DIR` pointing at the run's `shots/` directory.
Reference `args`/`env` from the frozen manifest are preserved as metadata and are never
applied wholesale; a typed option is added only when a native adapter really consumes it.
`--child-arg` is appended to the child argv; identity is captured from that *effective*
argv, so a `--child-arg --vault PATH` is bound as the vault the child will parse. A typed
binding cannot be re-bound that way: `--child-arg --live …` or `--child-arg --external-ts …`
is refused, because the case's live name and the loader smoke's raw source are the
selection the suite bound and hashed.

## Run directory, identity and resume

`run` requires `--run-dir DIR` and refuses to reuse an occupied directory (state.json or
any other content); `--resume` continues one. The identity is *content-bound* and must
match exactly before any spawn:

* manifest bytes, suite id and the frozen reference commit/tree/archive hash;
* the host and client checkout content (HEAD, index and the working-tree diff are hashed,
  so the same path with different bytes is a different identity);
* every launched executable: a direct `--exec-*` file is hashed, and the manifest's cargo
  template is resolved to the built artifact under `target/{debug,release}[/examples]` and
  hashed too — the suite never records an unresolved command string, and it launches the
  path it hashed, so build the executor once before the run (or pass
  `--exec-core`/`--exec-pair`);
* the profile/input configuration as the *child* resolves it: the suite runs the same
  read-only native resolver (`host_play::ProfileOptions::resolve_with_env`) over the
  effective argv it hands the child (profile flags then `--child-arg`), with the child's
  canonical working directory and inherited env. It records the selection
  (`local-274`/`local-289`/`public-289`) and the resolved cache, vault, nav pack/flags,
  content and unpack paths. Content-bound inputs:
  * catalog script tree (`<catalog>/src/bot/scripts`);
  * the vault the selection implies (a file, including a followed symlink; `NotFound` is a
    defined absence);
  * cache jag archives via the P1 `CacheManifest` identity (not a walk of the pack dir);
  * nav pack and nav flags files;
  * nav content inputs (`maps/`, door configs, `gates.loc`) — not models/sprites/fonts;
  * the engine RSA pem (`data/config/private.pem`), never the engine tree.
  Default unpack is derived runtime of the bound cache (path recorded). An explicit
  `--unpack` / `CLIENT_UNPACK_DIR` must be identifiable as a cache pack or the run refuses.
  `catalog_watch` isolates script stores on its own thread; those are not the operator
  vault. `LOGIN_RSAN`/`LOGIN_RSAE` refuse the run (credentials are not recorded).
* settings (level, `--only`, changed paths, memory mode, extra child args, child env names,
  canonical cwd) and the ordered selection; changing lowmem/highmem therefore refuses resume;
* the external loader smoke's raw source, when — and only when — the selection launches
  that case: the typed `--external-ts` file or the producer's tracked default fixture, bound
  by path *and* by content SHA-256 (its size and the digest the producer's harmless
  whitespace reload would produce are recorded too). A same-path byte change, a different
  `--external-ts`, a changed external executable or any changed typed option refuses the
  resume before a child is launched, and the receipt's source hash is checked against the
  *bound* bytes rather than the temporary path the producer copies them to.

An input the suite cannot bind (no built executable, a catalog without a script tree, a
profile the native resolver rejects, an unreadable repository) refuses the run before the
ledger exists; a recorded identity with an unresolved component refuses resume, because
such a run cannot prove the input is unchanged. A changed input *at the same path* — a
vault file, a script source — refuses resume with `profile/input configuration`.

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

### The external loader smoke

The loader smoke (`external_loader`, runner `external`) prints no scenario receipt. Its
producer writes the external record twice — as the `EXTERNAL_LOADER:` witness and as the
payload of the terminal line — and the suite requires the two to be the *same* JSON object
under the producer's live name `script_external_loader`:

    EXTERNAL_LOADER: script_external_loader {"stage":"qualified","script":{…},…}
    PASS: live script_external_loader {"stage":"qualified","script":{…},…}

A missing witness, a witness on a core/pair case, a `CATALOG_CORE`/`PAIRED_CORE` witness
on this case, both PASS and FAIL, a PASS with a nonzero exit (or the reverse) or a
payload that is not a JSON object is a shared failure. A qualified record is then checked
field by field against the production constants and the bound source:

* `script.name`/`script.version`, and `script.sha256` equal to the **bound** raw source
  digest (the producer never overwrites the watch's configured source hash);
* `compiled_sha` — the loaded raw File card's *cache key*, not a compiled-artifact hash —
  equal to the reloaded key, and all hash fields real lowercase SHA-256 digests. The
  producer's qualified record is written after the harmless whitespace reload, which
  overwrites that key with the reloaded card's, so the suite pins it to the digest it
  computed itself from the bound bytes plus the producer's one trailing newline; the
  load-time key is not serialized separately, and `script.sha256` is what carries the
  before identity;
* the fixture's own prerequisite: `initial.bones` is exactly `BONES_COUNT`, the final bone
  count is lower, Prayer xp is higher, and the distinct-burial count and raw burial counter
  agree and reach `MIN_DISTINCT_BURIALS`;
* registration once before and once after the reload, the in-game scene gate at
  `scene_state == 2`, `auto_start == false`, a non-empty account, the producer's own
  `START_DEADLINE`/`STOP_DEADLINE` values, a Stop inside its bound, and
  `reload_unchanged == NOTHING_CHANGED` with changed post-reload identities.

A failed record must carry the failed stage and a non-empty reason; the case failure keeps
the stage, the requested and completed operation and the cleanup outcome, so a failed
cleanup stays visible. The capture is mandatory and is not the prerequisite: the matched
record must be the producer's `external_loader terminal` label, written by this case, with
a sidecar naming the account the receipt qualified for. `external_loader prereq` is a
different capture and never satisfies the contract; a complete capture is still
`pending_visual_review`.

The run exits `0` only when nothing was unsuccessful, nothing was left unreached and no
shared stop happened; `1` for any unsuccessful execution, `2` for a usage or configuration
error, `130` when interrupted.

## Process ownership

Every launched child runs in a process group (unix) or a Windows job object and is wrapped
in an RAII guard, so success, an early error return and a panic all reap the tree. Ownership
is the *tree*, not the direct child: a direct child that exited is not a tree that exited,
so after the run the tree is checked independently of the pipes and of the direct child's
exit. A descendant that inherited stdout/stderr, or one detached with its own stdio that
nothing here can see, is terminated with a bounded escalation and `reaped` is only true once
the direct child has been waited, the tree holds no process, and the pipes reached EOF. A
tree that cannot be reaped inside the bound is recorded `cleanup_failed` on every exit path
— not only on a timeout — and stops the run instead of launching the next case.

* **unix**: the child is started in its own process group; the graceful stop is `SIGTERM`,
  the forced stop `SIGKILL`, and "still alive" is `killpg(pid, 0)`.
* **Windows**: the child is created suspended and in its own console process group, assigned
  to a job object (created with `KILL_ON_JOB_CLOSE`) and only then resumed, so no descendant
  can exist outside the job. The graceful stop is `CTRL_BREAK` to the child's console group,
  the forced stop is `TerminateJobObject`, and "still alive" is the job's active-process
  count (a failed query counts as alive). Closing the job handle — including through a panic
  — is the last-resort kill. `killed_signal` is `None` there (Windows has no termination
  signal); the cleanup note names the mechanism.
* **other platforms**: the suite refuses to launch a child it cannot own.

Interrupts are handled on both platforms by a real handler that only stores a flag
(`SIGINT`/`SIGTERM` on unix, a `SetConsoleCtrlHandler` for `CTRL_C`/`CTRL_BREAK` on Windows),
so the wait loop reaps the owned tree and the run exits 130 instead of leaving a child
running.

The log file is opened *before* the spawn, the wait loop is deadline-aware (never an
unbounded `wait`), and the pipes are drained with a deadline rather than joined: an
abandoned drain is reported in the ledger instead of hanging the suite. Output is read in
bounded chunks; a line past 64 KiB is emitted wrapped (and marked) rather than buffered
without limit.

The manifest cargo-template resolution uses the platform executable suffix, including
`.exe` on Windows; `--exec-core`/`--exec-pair` remain optional explicit overrides.
The runner owns process groups on unix and job objects on Windows as described above;
platform-specific LIVE proof remains a separate host run.


## Verification without a game

The suite's own behavior is checked offline, with the disposable `e2e-suite-fixture`
child standing in for a panel executable:

    cargo test -p e2e --lib
    cargo test -p e2e --test suite_offline

The offline suite (see `crates/e2e/tests/suite_offline.rs` and
`crates/e2e/fixtures/native-suite/offline-suite-manifest.json`) covers printed command
construction (with and without `--exec-*`: the resolved artifact path is launched and
printed, `cargo run` never is), a plan whose executable cannot be resolved refusing,
a zero-exit child with no receipt stopping the run, an isolated assertion
failure continuing, a successful case retained as `pending_visual_review` with a real
capture, a contracted terminal shot that never arrives (or arrives under another label)
stopping the run, resume carrying the result without relaunching it, resume refusing a
changed settings/manifest request *and* a changed input at the same path (vault content,
script source, the profile's *default* vault at its resolved path) before any launch,
refusing an unbindable catalog, an unresolvable executable, an unresolvable profile and a
relative input path, refusing an inherited `BUDGET_S` before a run, a resume and a
`dry-run`, budget expiry with forced-stop process-tree cleanup, a normal exit that is
reaped without a termination, interrupt cleanup, a log that cannot be opened refusing
before any spawn, a descendant holding the pipes not hanging the suite, and detached
descendants — including one that ignores the graceful stop — being force-stopped and
verified gone.

The external loader smoke has its own offline coverage. The disposable child drives the
producer's real `ExternalWatch` state machine and prints its real record, so the tests
exercise the adapter rather than a relabelled core receipt: a qualified record with its
terminal capture is retained `pending_visual_review`, the bound source path/digest/whitespace
digest land in the ledger and the child argv, and a core/pair-only selection never resolves
the loader executable or its raw source. Refused as shared failures: a missing witness, a
terminal line that disagrees with the witness, a record that does not confirm the capture
request, a failed receipt, no capture, only the prerequisite capture, a torn capture, a
capture of another actor, an arbitrary non-digest hash and a receipt whose source hash is
not the bound input. Resume refuses a same-path byte change of the bound source, a
different `--external-ts`, a relative `--external-ts` and a `--child-arg` re-binding of a
typed selection — all before any child is spawned.

The tests are portable: the real-process coverage runs on unix *and* Windows, the fixture
gates only the platform-specific actions (unix `SIGTERM` ignore vs Windows console-control
ignore), and the two genuinely platform-only cases are gated with their reason (the
`escape-pipe` fixture leaves a unix process group — a job grants no breakaway — and a
symlinked vault needs a Windows privilege). The fixture prints the panel's real line
contract and writes the scenario's declared terminal shot, and the tests resolve their
profile against a disposable `$HOME`/`USERPROFILE`. Native Windows verification of this
suite is a separate, still-pending run on a Windows host: `LIVE=1` harness tests under
`crates/e2e/tests/` remain separate and are not part of a suite run.

## Manifest

`crates/e2e/fixtures/native-suite/suite-manifest.json` is tracked, derived data (regenerate
with `derive.py` against a read-only frozen reference checkout) and is embedded at compile
time, so a run never depends on a local campaign path. It carries the frozen reference
commit/tree/archive hash, the reference case statuses, budgets and coverage as evidence,
the native adapter map (witness identity, live name, variants, declared gaps) and the
desired run options with their adapter coverage metadata. Reference `vetted`/`provenAt` are
historical upstream evidence, never a native PASS.

## Not claimed

Headed runs default to `--nav-paints on`; use `--nav-paints off` to disable diagnostic
layers. The choice is recorded in resume identity and forwarded to both native watcher
entrypoints. It changes session visuals without changing routing, teleport policy,
deadlines or saved operator preferences.

The RockCrab cases use the native stand `(2712,3707,0)` and are executable for fixture
qualification. Their live baseline still requires visible dormant Rocks before Start
and actual script-caused activation afterwards. A runnable case is not a qualification.

This entrypoint does not establish script qualification by itself. Requested extra captures,
remaining adapters and platform-specific LIVE qualification require their own evidence.
The external loader row stays `unvetted` in the manifest; neither an offline green case nor a
reference row's upstream status is a native PASS. Native terminal captures are structurally
validated, including both actors for paired cases, and remain pending visual review until readback.
