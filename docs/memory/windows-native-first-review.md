# Review: Windows home polish, NUL fixture, first native evidence

**Task:** `t_bc2fc7da`  
**Reviewer:** Hermes profile `reviewer` (Grok-4.5), round 1 (artifact lens + bounded Mac re-check)  
**Verdict:** **APPROVED**  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**Host branch / HEAD:** `codex/memory-diagnostics` @ `952ba22` (not `main`)  
**Client branch / HEAD:** `codex/windows-native-parking` @ `2b1af85` (gitlink matches `HEAD:vendor/fr-client-rust`)  
**Scope:** read-only. No git, code, evidence, remote Windows, or UI mutation. This is not whole-branch Grok-4.6 review.

---

## Reviewed freezes

| Ref | Role |
| --- | --- |
| host `1e33d28` | Windows operator home + host callsite wiring + script local adapter; gitlink → client `4b35300` |
| host `61b7b7d` | host-play memory failure fixtures: `/dev/null` → cfg `NUL` / `/dev/null` |
| client `4b35300` | canonical `operator_home` / pure `operator_home_from` + `engine_dir` / `unpack_dir` / unpack-cache defaults |
| client `2b1af85` | standalone client `Cargo.lock` records direct `windows-sys 0.59.0` on the `client` package |
| host `952ba22` | gitlink pin `4b35300` → `2b1af85` |

Earlier socket/metrics work (`9cdd4a2`, `c8b9334` / client `35f1c13`) already task-reviewed; used only as native baseline context. Uncommitted Python under `docs/memory/*.py` (concurrent `t_5c6d1143`) excluded.

---

## Code: home paths (`1e33d28` / `4b35300`)

**Cold read of current trees matches the intended semantics.**

- Canonical helper: `vendor/fr-client-rust/crates/client/src/bot_target.rs` — `operator_home()` → `operator_home_from(env::var("HOME"), \|\| env::var("USERPROFILE"))`.
- Windows: `Ok(h)` (including **empty string**) wins; `USERPROFILE` only on `Err` (absent or non-Unicode). Non-Windows: `userprofile` discarded (`let _ = userprofile`), no lookup.
- Lazy profile read: `FnOnce` closure; unit tests panic if profile is queried when HOME is explicit (client + script).
- Callers keep prior empty/missing fallbacks (`unpack_dir` empty → relative; `script::bot_home` empty/err → `"."`; pack defaults join or relative; live `options()` still `expect`).
- Host production path defaults switched from raw `HOME` to `client::operator_home()` in host-play / nav / scenario / e2e listed sites.
- **Dependency boundary preserved:** `crates/script` keeps `client` under **`[dev-dependencies]` only**; production `script::bot_home` uses a **local private** adapter with the same cfg rules (no production `client` edge).

**Behavior preservation:** Unix path is still verbatim `HOME` Result shape; no new Unix env reads or allocations in the selector. Explicit blank HOME is preserved on Windows (does not fall through to USERPROFILE).

**Bounded Mac tests (this review):**

- `cargo test -p script --lib isolated_env::` → **8 passed** (includes selector + no-profile-query tests).
- `cargo test -p client --lib bot_target:: --manifest-path vendor/fr-client-rust/Cargo.toml` → **5 passed**.

No must-fix defects found in home wiring.

---

## Code: NUL fixture (`61b7b7d`)

`crates/host-play/src/memory.rs` test module:

- `#[cfg(windows)] NULL_DEVICE = "NUL"` / else `"/dev/null"`.
- Write sink and **read-only** qualification handle both open `NULL_DEVICE`.
- `handle_harness_failure_preserves_err_and_latches_on_writer_fail` still opens **read-only** and asserts harness error preserved + latch — intent “reject writes / fail writeln path” unchanged; only the device path is portable.

**Bounded Mac test:** `cargo test -p host-play --lib memory::tests --features memory-profile-no-alloc` → **45 passed** (includes the read-only writer-fail path).

Native before/after logs corroborate the fixture class:

- `test-c8b9334-hostplay-memory.log`: **175 passed / 5 failed**, failures are missing `/dev/null` on Windows.
- `test-61b7b7d-hostplay-memory.log`: **180 passed / 0 failed**.

---

## Code: lockfile gitlink (`2b1af85` / `952ba22`)

- Client lockfile one-line add: package `client` dependencies list gains `windows-sys 0.59.0` (matches `crates/client/Cargo.toml` socket feature pin).
- Host `952ba22` only bumps submodule gitlink; working tree HEAD client == gitlink `2b1af85`.

---

## Native evidence honesty

Primary synthesis: `docs/memory/windows-native-first-proof.md`  
Raw: `docs/memory/diagnostics/windows-native-20260907a/`  
GPU write-up: `docs/memory/windows-gpu-shade-diagnosis.md` (+ probe outs under the same diagnostics dir)

### Checks that hold

| Claim | Independent check |
| --- | --- |
| Branches / freezes | Host not main; client branch and SHAs as above |
| c8 host 179 / host-play unit 119 | Log `test result` lines match |
| c8 memory 175 fail /dev/null → 61 memory 180 | Logs + jsonl exit codes match |
| 61 script isolated_env 8 / client bot_target 5 / panel lib resource 6 | Logs match; earlier c8 **bin** `resource::` filter ran **0** tests — proof correctly refuses that as proof |
| Client full suite abort at gpu_texture shade 16 ~223 got 255 | `test-c8b9334-client-integration.log` + isolated `gpu-shade-isolated-35f1c13.log` (0 pass / 1 fail exact) |
| Panel smoke provenance | `launch.json`: host `61b7b7d`, client `4b35300`, PID 8888, session 1, `binarySha256` `0FC3F79B…A424` (case-normalized match to proof), `diagnosticOnly` true, `acceptedPerformance` false, server placement Mac reverse-forward note |
| Launcher `exitCode=null` | `completion.json` literally `"exitCode": null`; timedOut false; stderr ends `PASS: memory panel observation complete` |
| Qualification boundaries | `samples.qualification.jsonl` observe-start/end: `ingame=true`, `scene_state=2`, `state=Running`, `error=null`, GPU renderer available draw/full_rate; steals 4→12, coins 120→360, food 4→2 / ate 2, ticks 97→298; bank trips 0 |
| Observation samples | **119** `phase=observe` rows; all `ready=1` `active=1`; `resident_bytes` **476188672–482426880** (matches proof WS range wording) |
| No savings / no full-suite green | Proof status and body refuse savings, matched baseline, and suite green; LNK4098 not suppressed |
| Probe vs live adapter | Proof: driver from **separate matching-request GPU probe**; “does not directly identify the adapter inside the live panel” — correct distinction |
| Account prep | Disabled BotTest pending local password; no claim of quieter measurement without operator sign-out |
| Pending follow-ups | TUI / full locked client / matched measurements / Grok-4.6 called out as outstanding |
| Artifact SHA sidecars | Spot-checked `artifact-sha256.json` vs files: `native-tests-61b7b7d.jsonl`, hostplay-memory log, completion.json, shade-016 hist — **match** |

### GPU diagnosis vs later probe

- Exact test failure and H1 mechanism (smooth `f32` + truncating `u32` at block boundaries) are source-consistent; Mac pass vs Windows/Linux fail framing is appropriate; no tolerance/skip/shader redesign authorization claimed.
- **Probe artifacts now exist** under `windows-gpu-shade-out-20260907/`: selected adapter NVIDIA RTX 5060 Laptop **Vulkan** driver **616.56**; shade **16** red-dom hist **223×41435 + 255×31983** (bimodal); shade **17** unimodal 223; interiors clean — **H1 confirmed on this Windows probe path**.
- `windows-gpu-shade-diagnosis.md` **bottom line still says** Windows lacks adapter identity / histogram (“not measured”). That sentence is **stale relative to the same evidence tree’s probe outputs**. Treat the diagnosis as a **pre-probe source card**; current closed native shade classification should follow **proof + `probe-report.txt` / hist files**. Not a code defect; not a savings claim. Non-blocking for this freeze if readers use the synthesis + raw probe as authority (recommended).

### Explicit non-claims respected

- No matched CPU/RSS savings.
- No whole client suite green.
- No claim that live panel wgpu adapter == probe selection.
- No retrofit of `exitCode=null` receipt.
- Cross-host Mac server via loopback SSH forwards — not same-host managed six-role accounting.
- Python 573aca1 native 71 tests / 2 failures acknowledged; repair owned elsewhere.

### Minor nits (non-blocking)

- Transfer archive SHA256 in the proof is not re-verified here (archive blob not required in-tree for this card); file-level `artifact-sha256.json` samples verified.
- `windows-home-path-report.md` is a point-in-time implementer note (client pin `35f1c13` at write); superseded by `4b35300`/`2b1af85` for the home helper itself.
- Report lists `operator_home_from` beside the canonical helper; it is **crate-private** (only `operator_home` is re-exported) — accurate for tests, slightly loose as a public API bullet.

---

## Verdict

**APPROVED.**

Home-path Windows fallback (including explicit blank HOME), Unix non-lookup, script local adapter without production `client` dep, NUL fixture with retained read-only write-fail behavior, and lockfile/gitlink pin are correct and behavior-preserving. First native proof synthesis is honest about qualification limits, known GPU shade failure, probe-vs-live adapter, panel SHA/provenance, raw qual rows, and launcher `exitCode=null`. No savings or whole-suite green claim accepted.

**Non-blocking follow-through (orchestrator / later docs, not this implementer rework gate):** optionally amend `windows-gpu-shade-diagnosis.md` bottom line to point at the completed Windows probe histograms/adapter so a cold reader of that file alone is not told the probe is still missing.

**Out of scope / still open (as proof states):** TUI native, full `--locked` client coverage, Python collector native repair (`t_5c6d1143`), matched measurements, BotTest activation, whole-branch Grok-4.6.
