# Direct-owner deadline and feature-off coverage follow-up

## Scope and disposition

This card adds a hash-bound **test-only** overlay and a bounded staging runner for
the frozen direct-owner source. It does not change the frozen archive, its
manifest, any production source, any lockfile, `STATE.md`, the client submodule,
or the reviewed production binary. The local result is generated macOS evidence,
not Linux-native or live qualification.

The completed local matrix is green: 12/12 requested commands exited zero,
none timed out, every owned process group was absent after its command, and all
1124 source members plus all three lockfiles matched before and after testing.
The receipt deliberately retains `native_linux_qualified=false` and
`live_qualified=false`.

Evidence:

- `diagnostics/direct-owner-native-preparation/coverage-overlay/local-verification-corrected/overlay-receipt.json`
  - 35105 B, SHA-256 `16e78322c1d7844a66f052feb7e12ee6a9f7231f2995990300cca96a61d7c056`
- `diagnostics/direct-owner-native-preparation/coverage-overlay/local-verification-corrected/steps.json`
  - SHA-256 `a67fb29a211d3cab819e0c2770c5cb8b6eb92e07b20f492569a5da023e4b26f7`
- Per-command raw logs are in that directory's `logs/` subdirectory and are
  individually size/hash-bound by both JSON records.

## Frozen input and derived test identity

The runner accepts only this frozen source identity:

- archive: 5075876 B, SHA-256
  `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`
- manifest: SHA-256
  `ca69e70352c5f6f8d1c9db026959af6ba8fd64886be533a0095c3ca94bb5732e`
- original host Git object: `c0709aba2f8b45e42193225cf8f4e7325b5ca9bf`
- original client Git object: `3456edc8dabf7b25ada78110ffa56327af9f67a4`
- source members: 1124
- locks: root `03059d02…`, client `015530e3…`, shade-probe `d0285dae…`

It applies, in order:

1. the already-reviewed TUI cfg(test) guard, SHA-256
   `3bf08949fee5f44a2c6dc82525004a9cc0f47759526d43bda6f77cc9f33a51e4`;
2. `coverage-test-only.patch`, SHA-256
   `dcb84047157f75c7ad21868b72eb7c3434cb818e8ba3e4b732f6b40d435c1b6e`.

The coverage patch adds nine test functions inside existing cfg(test) modules;
it does not alter the bytes before those modules. Together with the reviewed
TUI guard, six derived paths are allowed and exact-hash checked:

- `crates/api/src/owner_capture.rs` — `5d91a1a7…`
- `crates/host/src/owner_capture.rs` — `138a8838…`
- `crates/host-play/src/owner_capture.rs` — `b758ca47…`
- `crates/host-play/src/owner_capture_output.rs` — `59421d88…`
- `crates/tui/src/bin.rs` — `aee97345…` (reviewed TUI guard only)
- `vendor/fr-client-rust/crates/client/src/core/world_owner_capture.rs` —
  `7b572b26…`

All other 1118 members remain byte-for-byte equal to the frozen manifest.
`stage_coverage_overlay.py` is 24072 B, SHA-256
`6e1aab8eae3bd217f1f2b320f8b1b45e7a8fae924563dd7c74449ed4af5a3082`.

## Assertion audit and test cases

The combined native review was correct that the prior logs had no explicit
named deadline or stale-frame/stale-slot test. Existing tests already covered
the visit cap, fixed mailboxes, mailbox-full non-retry, runtime/feature off,
COW scratch, cumulative output cap, and Stop/fingerprint behavior. This overlay
adds only the missing discriminating cases:

| Added test | Required contract exercised |
|---|---|
| `deadline_failure_is_latched_and_suppresses_later_rows` | API 5 ms deadline latches `BudgetDeadline`; later field rows are suppressed |
| `linked_square_walk_honors_fragment_deadline` | client linked-square walk checks the deadline before following the link and reports an incomplete deadline row |
| `token_addressed_queue_does_not_deliver_stale_request_to_current_slot` | a stale-token request is not consumed by the current token; the matching token can consume it and releases its bounded byte charge |
| `wrong_token_and_request_are_rejected_before_publication` | wrong token and wrong request-id fragments both return `WrongEpoch` without publishing a fragment row |
| `stale_frame_is_rejected_without_publishing_a_second_row` | the first same-phase fragment is retained; a different-frame companion returns `StaleFrame` and cannot publish a second row |
| `malformed_fragment_is_rejected_with_failure_receipt` | malformed/incomplete input returns `Incomplete` while preserving one raw failure record (`complete=false`, `reason=malformed`) |
| `delayed_fragment_is_rejected_while_preserving_raw_evidence` | an over-budget fragment returns `BudgetDeadline` while preserving its raw diagnostic record |
| `missing_fragment_deadline_cannot_finish_successfully` | a request with no response crosses the output deadline and finishes with an unsuccessful `owner_terminal` (`reason=missing`) |
| `delayed_fragment_latches_failure_and_terminal_is_unsuccessful` | `PhasePublisher::poll` latches a delayed-fragment error; `finish` rejects qualification and writes `owner_terminal complete=false, reason=budget_deadline` |

This distinguishes queue admission from qualification. `try_publish_request`
is a bounded token-addressed queue, not the qualification boundary;
`try_take_request_for(current)` prevents cross-token consumption. Output and
`PhasePublisher` enforce response/fragment deadlines and ensure a stale or
missing response cannot produce a successful terminal result.

It also preserves the plan's failed-partial-facts rule. `Output::fragment`
deliberately writes an incomplete or late raw record before returning its error;
an empty-log requirement would discard required failure evidence. The terminal
record, not raw-row existence alone, carries overall capture qualification.

## Preserved RED and environment attribution

The first audit assertions were intentionally allowed to fail before contract
resolution:

- stale enqueue expected `Err(WrongEpoch)` but returned `Ok(())`;
- malformed and delayed fragments were expected to leave the file empty, but
  each preserved a raw fragment row;
- feature-off host under global `BOT_CPU=1` failed
  `prefer_cpu_rebuilds_renderer_not_client` (`paint_n` 1 versus 2).

Those raw attempts remain under
`coverage-overlay/local-verification-01/` for review but are not passing
evidence. Source inspection showed the first two expectations were stronger
than the approved contract, not production defects: token-addressed consumption
provides isolation, and failed raw rows must be preserved. The replacement
assertions above retain the actual required failure boundaries rather than
weakening them.

Root independently reproduced the host failure on clean original H/C source:

- `root-host-featureoff-baseline-01/result.json`: exit 101, 212 passed,
  1 failed, 1 ignored; raw log SHA-256 `2ba3c91b…`;
- cause: `slot_want_cpu` ORs `input.prefer_cpu` with `BOT_CPU==1`, so the test's
  intended false-to-true request transition becomes true-to-true;
- `root-host-featureoff-env-correction-01/result.json`: with only `BOT_CPU`
  unset and `R274_TEST_FORCE_NO_GPU=1`, the same original-source full command
  passed (exit 0, 10.7473205 s; log SHA-256 `173e65f9…`).

The final candidate runner makes that correction only for the full host suite.
Its receipt records `BOT_CPU=null`, `R274_TEST_FORCE_NO_GPU=1`, and the candidate
full host result: 213 passed, 0 failed, 1 ignored. Other commands retain
`BOT_CPU=1`. This is a test-environment correction, not CPU live execution and
not a source/assertion change. Both original failed records remain evidence.

## Local generated verification

Command (Darwin arm64, Rust/Cargo 1.98 toolchain already present):

    python3 diagnostics/direct-owner-native-preparation/coverage-overlay/stage_coverage_overlay.py \
      --archive diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz \
      --manifest diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-source-manifest.json \
      --output-dir diagnostics/direct-owner-native-preparation/coverage-overlay/local-verification-corrected \
      --run-gap-tests --run-feature-off --timeout 900

Gap matrix, all exit 0 with no timeout and clean process-group teardown:

- API deadline: 1 passed;
- client linked deadline: 1 passed;
- host token isolation: 1 passed;
- host-play output module: 7 passed (five new cases plus two existing output-cap/completeness tests);
- host-play publisher terminal deadline: 1 passed.

Feature-off matrix, all exit 0:

- API: all 10 emitted harness results passed;
- host: 213 passed, 0 failed, 1 ignored, plus empty doc-test harness;
- script with `load`: all 20 emitted harness results passed (two ignored tests retained);
- host-play with `memory-profile-no-alloc`: main library 182 passed, plus all emitted integration/doc harness results passed with their existing ignores;
- TUI with `memory-profile-no-alloc` and the reviewed cfg(test) guard: 92 passed;
- client unit: 75 passed;
- client integration command (`--tests`): unit harness plus every integration harness passed.

All commands were `--offline --locked --test-threads=1`. The runner removes
`LIVE`, vault, cache, server, host, and memory-N inputs; sets
`BOT_MEMORY_OWNER_CAPTURE=0`; uses one owned target directory; gives each
command a fresh process group; terminates/reaps that group on timeout; records
its cleanup; and continues the bounded matrix so each prerequisite has an
independent receipt. No frontend, account, cache, server, network, or live test
was used.

## Linux/root-ready command and remaining boundary

After same-card review, root can run the complete prerequisite matrix in a
fresh Linux output directory:

    python3 diagnostics/direct-owner-native-preparation/coverage-overlay/stage_coverage_overlay.py \
      --archive diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-direct-owner-source.tar.gz \
      --manifest diagnostics/direct-owner-native-preparation/artifact/frozen/frozen-source-manifest.json \
      --output-dir <fresh-linux-output> \
      --run-gap-tests --run-feature-off --run-feature-on --timeout 900

`--run-feature-on` adds full API, host, script (`load,memory-owner-capture`),
host-play/TUI (`memory-profile-no-alloc,memory-owner-capture`), and separate
client unit/integration commands. The prior combined Linux package already
qualified the focused feature-on owner tests, corrected client unit/integration
steps, and reviewed TUI overlay, but it did not run these new gap tests or the
full with/without-feature suites. Therefore this local result does not relabel
that package and does not satisfy the remaining native prerequisite.

The exact reviewed production binary `8e400e2d…` and original qualification
records remain immutable. A derived tree from this overlay is test-only and
must never be used for a release or live launch. Live admission, controller and
private preflight, source-Git derivative admission, RSS/savings conclusions,
and whole-branch Grok 4.6 review remain outside this card.