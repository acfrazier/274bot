# Windows process metrics (hop 2) — implementer report

**Task:** t_597230fa  
**Branch:** `codex/memory-diagnostics`  
**Workspace:** `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`  
**No git mutations** (implementer). Native Windows MSVC unit-test execution is **root-owned / pending**.

---

## What changed

### `crates/host-play/src/rss.rs`

- **Windows current RSS:** `GetProcessMemoryInfo` → `WorkingSetSize` via `current_resident_bytes()`.
  - Success + nonzero → `Some(n)`; API fail or zero → `None` (never `Some(0)`).
- **Windows peak:** same call → `PeakWorkingSetSize` as `sample_process().0`.
  - Documented: first field is lifetime peak WS on Windows (parallel to `ru_maxrss` on unix).
  - Fail/zero peak or fail CPU → full public `(0, 0.0)` sentinel via `combine_windows_sample` (panel joint Error).
- **Windows CPU (sample_process second field):** `GetProcessTimes` user+kernel cumulative CPU durations (100 ns units).
- **Pure helpers:** `filetime_parts_to_seconds` (CPU duration 100 ns → s); `combine_windows_sample` (joint sentinel; mixed-failure unit coverage).
- **Shared:** `windows_cpu_user_kernel()` for memory harness split user/kernel.
- **Handles:** `GetCurrentProcess()` pseudo-handle only; never closed; no per-sample process open/close; no heap allocation in the sample path.
- **`count_tcp_to`:** `#[cfg(windows)]` always `None` — **no Windows TCP-count backend** this hop (`lsof` is unix-only). Explicitly marked in docs/comment.
- Unix/macOS/Linux paths unchanged in behavior.

### `crates/host-play/src/memory.rs`

- `process_cpu_seconds()`: Windows branch returns `Option<(user_s, kernel_s)>` from `GetProcessTimes` (null JSON fields on fail).
- `peak_resident_bytes` emission: **Windows only** maps `sample_process().0 == 0` → `None`. Non-Windows keeps prior `Some(sample_process().0)` (including `Some(0)` on unix sentinel).
- CPU monotonic test gated `#[cfg(any(unix, windows))]`.

### `crates/host-play/Cargo.toml`

- Target-only dependency:
  ```toml
  [target.'cfg(windows)'.dependencies]
  windows-sys = { version = "0.59", features = [
      "Win32_Foundation",
      "Win32_System_Threading",
      "Win32_System_ProcessStatus",
  ] }
  ```
- Aligned with host/client direct pin **0.59** (operator correction; not 0.61.2).

### `crates/panel/src/resource.rs`

- Windows-only caption: `format_rss_caption` appends ` peak` (same as macOS), because sample field is peak WS.
- Non-Windows non-macOS captions unchanged (Linux still plain `format_rss`).
- Test renamed to cover macos+windows peak wording.

### Out of scope (honored)

- No edits to host socket/wake, `host-play/src/lib.rs` tests, client submodule.
- No Python backends, no `count_tcp_to` Win32 port.
- No allocator/measurement policy redesign.
- No performance/savings claims.
- No remote Windows actions / Mac live runs / git commit-push.

---

## Tests

### Mac (this box) — run by implementer

| Command | Result |
|---------|--------|
| `cargo test -p host-play --lib rss::` | **ok** 5 passed (darwin_ru_maxrss, sample_process nonzero, current independent of peak, lsof parse, **combine_windows_sample joint sentinel**) |
| `cargo test -p host-play --lib --features memory-profile process_cpu_time_is_available` | **ok** 1 passed |
| `cargo test -p host-play --lib --features memory-profile-no-alloc process_cpu_time_is_available` | **ok** 1 passed |
| `cargo test -p panel --lib resource::` | **ok** 6 passed (incl. `rss_caption_mentions_peak_on_macos_and_windows`) |

`Cargo.lock`: host-play lists `windows-sys 0.59.0` target dep; no other lock churn intended.

### Windows native — **PENDING root**

Must run on installed **Rust 1.98 / MSVC** `x86_64-pc-windows-msvc`:

```text
cargo test -p host-play --lib rss::
cargo test -p host-play --lib --features memory-profile process_cpu
cargo test -p panel --lib resource::
```

Windows-only tests present in source (not executed here):

- `filetime_parts_to_seconds_controlled`
- `nonzero_resident_rejects_zero`
- `combine_windows_sample_joint_sentinel_on_any_failure` (mixed peak/CPU Option coverage; pure helper runs on Mac too)
- `windows_working_set_current_and_peak_nonzero`
- `windows_cpu_user_kernel_monotonic`
- `windows_count_tcp_to_is_unsupported`
- generic host tests extended with `windows` cfg for nonzero current/peak

---

## Semantics cheat sheet

| API | macOS/Linux | Windows |
|-----|-------------|---------|
| `current_resident_bytes` | task_info / statm | `WorkingSetSize` |
| `sample_process().0` | `ru_maxrss` (peak) | `PeakWorkingSetSize` |
| `sample_process().1` | utime+stime | user+kernel FILETIME |
| `process_cpu_seconds` | getrusage user/sys | GetProcessTimes user/kernel |
| peak JSON | Some(peak) always (incl. Some(0) sentinel) | Some(peak) if ≠0; fail/0 → null |
| `count_tcp_to` | lsof | **None (missing backend)** |
| panel RAM caption | peak on macOS | peak on Windows |

---

## Limitations

1. **Native Windows unit tests not run** in this task — report pending honestly.
2. Full panel binary still depends on hop-1 socket/wake portability (sibling worker); metrics alone do not prove headed panel.
3. Working set ≠ Linux RSS ≠ macOS footprint; no cross-OS equality claim.
4. TCP connection counts remain unavailable on native Windows.
5. No accepted memory savings from this hop.

---

## Source evidence (key symbols)

- `rss::windows_memory_counters` / `windows_cpu_user_kernel` / `filetime_parts_to_seconds` / `combine_windows_sample`
- `rss::sample_process` / `current_resident_bytes` / `count_tcp_to` (windows None)
- `memory::process_cpu_seconds` windows branch
- `memory` sample builder peak `None` on zero **cfg(windows) only**
- `resource::format_rss_caption` macos|windows peak

### Review round-2 fixes (this pass)

1. Gate peak JSON zero→None to Windows; restore Unix `Some(sample.0)`.
2. `combine_windows_sample`: either memory or CPU `None` → `(0, 0.0)`; mixed-failure unit test.
3. FILETIME comment: user/kernel are CPU durations in 100 ns units, not wall since 1601.
