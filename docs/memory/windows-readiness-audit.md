# Windows build and measurement readiness audit

**Scope:** read-only source audit on `codex/memory-diagnostics` @ branch HEAD `5d971d1` (docs freeze; animation candidate host `ba4bbd0` is an ancestor) / client `e17deab`.  
**Machine intent (operator-supplied, not exercised here):** x86_64 Windows laptop (Ultra 9 275HX / RTX 5060 laptop / 64 GB); headed `panel-play` primarily over RDP to that box. Root owns setup and credentials. Campaign scope is **local engine + pack cache only** — production-server access (`BOT_TARGET=prod` / credentials) is **not** authorized here.  
**This audit did not:** build, test, install, RDP, or touch a Windows host. **No MSVC compile or Windows test success/failure receipt exists yet.** Linux/macOS receipts do **not** prove Windows.

---

## Verdict (one line)

Native Windows is **not compile-ready** today (unix `poll` + `UnixStream` + `AsRawFd` in the live host/client path). Measurement backends are **linux/macos only**. WSL2 can host a **Linux-target** TUI/measurement lane; it is **not** native panel/RDP/GPU validation.

---

## Three lanes (do not collapse)

| Lane | What it is | What source allows today | What it does **not** prove |
|------|------------|--------------------------|----------------------------|
| **A. Native Windows RDP/GPU** | `x86_64-pc-windows-msvc` `panel-play`, DX12/Vulkan wgpu, RDP session | Stack deps (winit/wgpu/dear-imgui) are cross-platform in principle; **host+client fail to compile** on unix-only I/O | Any live blit, GPU adapter, RDP present path, or Windows RSS/CPU card |
| **B. x86_64 WSL2 Linux TUI** | Linux userspace on the laptop; `tui-play` / headless harness as **Linux** target | Same as existing Linux paths (`/proc`, `getrusage`, vault unix modes) once WSL toolchain + engine/cache exist | Native Windows panel, RDP, DX12, or Windows process metrics for a Win32 PID |
| **C. Actual tests** | CI or on-box receipts for A or B | **None.** `.github/workflows/ci.yml` is `ubuntu-latest` only; no Windows job, no WSL job, no Windows build log in-tree | — |

README L86 documents panel-as-OS-window behavior under a **Windows:** heading; that is product prose, not a green Windows build.

---

## Minimal development prerequisites (native MSVC)

When root stands up the box (not done in this task):

1. **Rust 1.98.0** (`rust-toolchain.toml`) via rustup, target `x86_64-pc-windows-msvc`.
2. **Visual Studio Build Tools** (MSVC + Windows SDK) — required for that target and for crates that link system libs.
3. **Git** + recursive submodule `vendor/fr-client-rust` @ `e17deab`.
4. **Python 3** for `docs/memory/*` diagnostics (see Python gaps below — stock `import resource` is unix-only).
5. **Local 274 engine + pack cache only** (`BOT_TARGET` unset / `local`). Nav pack bake if WalkTo is needed. No prod world switch and no production credentials in this campaign.
6. **GPU drivers** for RTX 5060 if validating headed wgpu (not `BOT_CPU=1`). RDP GPU/encode behavior is an extra unknown.
7. Optional audio (lane A panel only): `crates/panel/Cargo.toml` enables client `features = ["audio"]` → `cpal` (WASAPI on Windows). That is **not** a TUI requirement.

### Lane B (WSL2 Linux TUI) — native deps are not “headed/ALSA”

Do **not** collapse TUI link needs into the CI panel package list.

| Crate | Client features | Linux native story |
|-------|-----------------|--------------------|
| **`tui` / `tui-play`** | `client` **without** `audio` (`crates/tui/Cargo.toml`) | No cpal → **no libasound** because of TUI. Still pulls always-on client deps: `wgpu` 29 (link/build surface even headless), `native-tls` / OpenSSL stack, `libc`, plus host unix park path. Crossterm/ratatui are terminal I/O, not X11. |
| **`panel` / `panel-play`** | `client` **with** `features = ["audio"]` | cpal → ALSA on Linux; plus winit/dear-imgui → X11/Wayland headers. Matches CI comment: *“panel enables client `audio` (cpal → alsa-sys)”* (`.github/workflows/ci.yml` L12–15, L44–55). |
| **client defaults** | `default = []`; `audio` / `window` optional (`vendor/.../client/Cargo.toml` L6–9) | `window`/`audio` off does **not** drop `wgpu` or `native-tls`. |

WSL2 lane B needs normal Linux rustup + whatever the **TUI** graph actually links (`libssl`/OpenSSL tooling for `native-tls`, GPU/Vulkan or `BOT_CPU=1` policy for always-on `wgpu`, build-essential). CI’s `libasound2-dev` + X11/Wayland set is driven by **workspace panel** builds, not by tui-play. Building headed panel *inside* WSL (WSLg) is still not lane A and still needs the panel package set.

---

## Existing support vs concrete gaps

### Already portable or gated

| Area | Evidence | Note |
|------|----------|------|
| Vault file write without unix modes | `crates/vault/src/lib.rs` L317–360 `#[cfg(not(unix))]` `File::create` / `create_dir_all` | Compiles; ACL privacy weaker than `0o600`/`0o700` |
| Script/loadouts/js_cache mode bits | `cfg(unix)` around `PermissionsExt` | Non-unix skips mode tighten |
| Panel resource *formatters* | `crates/panel/src/resource.rs` | Pure; OS-agnostic except macOS “peak” caption L52–59 |
| Counting allocator | `crates/host-play/src/memory.rs` L51–87 | Portable; independent of RSS |
| `available_parallelism` for CPU % denom | `crates/panel/src/app.rs` L471–474 | OK on Windows once samples exist |
| Client GPU default / `BOT_CPU=1` | `vendor/.../render/renderer.rs` L40–44 | Policy portable; adapter still untested on Win |
| Dep graph intent | `panel`: winit 0.30, wgpu 29, dear-imgui-*, client **+audio**; `tui`: client **default features only** (no audio); client always: wgpu 29, `native-tls` | No Cargo `windows`/`winapi` pins; relies on crate defaults |

### Hard compile blockers (native Windows)

These are unconditional unix APIs on the **live runtime** path (not test-only):

1. **Client stream fd API** — `vendor/fr-client-rust/crates/client/src/io/client_stream.rs`  
   - L11: `use std::os::unix::io::AsRawFd` (unconditional).  
   - L114: WSS connect stores `tcp.as_raw_fd()`.  
   - L152–157: `fd()` is `#[cfg(unix)]` only.  
   - L264–269: WS `available()` calls `libc::poll` unconditionally in that branch.

2. **Slot control wake** — `crates/host/src/slot_io.rs`  
   - L2–3: `AsRawFd`, `UnixStream`.  
   - L176–201: `SlotWake`/`SlotPark` = `UnixStream::pair()` + `as_raw_fd()` — documented as required because `poll(2)` needs an fd (L179–182).

3. **Idle park scheduler** — `crates/host/src/lib.rs`  
   - L690–725: `park()` builds `libc::pollfd` from control fd + `stream.fd()`, calls `libc::poll`. This is the multi-slot idle path used by panel/TUI/host-play.

**Implication:** `host` (and thus `panel`, `tui`, `host-play`, `e2e`) does not currently build for `*-windows-*`. Fixing measurement alone does not yield a Windows panel binary.

Secondary (tests / polish, still unix-shaped):

- `crates/host-play/src/lib.rs` L4514–4519: test parks on `libc::poll`.  
- `crates/host/src/slot_io.rs` tests L355+ same.  
- `count_tcp_to` shells out to `lsof` (`rss.rs` L119–128) — absent on stock Windows.

### Runtime / path assumptions

| Item | Evidence | Windows risk |
|------|----------|--------------|
| Operator home | `script::bot_home` only reads `HOME`, else `"."` (`isolated_env.rs` L28–35) | No `USERPROFILE`/`HOMEDRIVE`+`HOMEPATH`; vault/UI state may land in cwd unless `HOME` is set |
| e2e HOME | e.g. `crates/e2e/tests/...` `std::env::var("HOME")` | Same |
| Smoke/nav defaults | `~/.274bot/...` via `HOME` | Same |

### Measurement — Rust (`host-play`)

Root finding confirmed in source:

| API | File | Non-linux/non-macos / non-unix behavior |
|-----|------|----------------------------------------|
| `current_resident_bytes` | `crates/host-play/src/rss.rs` L49–93 | L90–92: **`None`** |
| `sample_process` (peak RSS + CPU) | `rss.rs` L23–41 | L38–40: **`(0, 0.0)`** |
| `rss_bytes_from_ru_maxrss` | `rss.rs` L8–20 | L16–19: **0** |
| `process_cpu_seconds` | `crates/host-play/src/memory.rs` L36–47 | L46–47: **`None`** (`#[cfg(not(unix))]`) |
| Tests | `rss.rs` L149–167; `memory.rs` L1642+ | Explicitly expect None/zeros off linux/macos; CPU test is `#[cfg(unix)]` only |

**Harness emission footgun:** memory samples set  
`resident_bytes: current_resident_bytes()` (honest `null` JSON when None) but  
`peak_resident_bytes: Some(sample_process().0)` (`memory.rs` L1052–1053, L1064–1066).  
On Windows that becomes **`Some(0)` peak** and null CPU fields — not a successful measurement, easy to misread if gates treat 0 as real.

**Panel resource card:** `crates/panel/src/app.rs` L451–466 treats `(rss, cpu) == (0, 0.0)` as `Metric::Error("process sample failed")` — correct fail-closed UI once the binary runs; still no Windows sampler.

Linux path uses `/proc/self/statm` + pagesize (`rss.rs` L75–88). macOS uses Mach `task_info` (L50–73). **No Win32 `GetProcessMemoryInfo` / `GetProcessTimes` branch.**

### Measurement — Python (`docs/memory`)

| Module | Behavior on Windows |
|--------|---------------------|
| `server_resources.py` L170–176 | linux → `/proc`; darwin → `/bin/ps`; else **`SampleError("unsupported OS: …")`** |
| `server_resources.sample_pressure` L179–188 | Linux PSI only; else unavailable (not zero) |
| `native_process_sample.py` L31+ | **`sys.platform != 'darwin'` → error**; libproc only |
| `process_accounting.py` L27 | **`import resource` at module import** — CPython on Windows has no `resource` module → **import fails before any sampler** |
| Children CPU | `resource.getrusage(RUSAGE_CHILDREN)` (`process_accounting.py` L225–233) — unix |

So continuous multi-role accounting and server-side sampling are not Windows-host portable without new backends (and fixing the hard `import resource`).

---

## What is *not* claimed

- **Source-only audit.** No actual Windows MSVC compile success or failure receipt, clippy, or test run exists for this candidate (not softened: absence is not a predicted compile error log).  
- No RDP frame-present, DXGI/Vulkan adapter pick, or multi-slot park latency data.  
- Linux reference builds under `docs/memory/_linux_ref_*` and macOS diagnostic JSON are **out of scope as Windows evidence**.  
- WSL2 success would validate **lane B only** (Linux TUI / `/proc` metrics), never native Win32 panel or Windows PID samplers.

---

## Bounded next implementation recommendation

Do **not** redesign measurement policy, add test waivers, or invent cross-OS metric equivalence in the first hop. Ordered, minimal work:

### Hop 1 — Native compile (lane A prerequisite)

Replace unix-only wake + park with a small OS abstraction used by `Host::park` and `ClientStream` readability:

- Control channel: today `UnixStream::pair` (`slot_io.rs` L196–200). Windows options (pick one, keep kick-byte semantics): TCP `127.0.0.1` pair, Win32 event/pipe + wait, or a crate that yields waitable handles.  
- Socket wait: today `libc::poll` on raw fds (`host/src/lib.rs` L690–725; client WSS `available`). Windows needs `WSAPoll` / overlapped / std nonblocking + sleep fallback that **must not busy-spin** (see park comments L685–688).  
- Gate `AsRawFd` / `fd()` behind `cfg(unix)`; provide Windows handle/socket accessors only where park needs them.  
- Keep behavior: wake drains, no-consumption socket re-poll guard, timeout.

**Exit criterion:** `cargo build --release -p panel --bin panel-play` on `x86_64-pc-windows-msvc` (root-run). No measurement claim yet.

### Hop 2 — Fail-closed Windows process metrics (Rust)

Add `cfg(target_os = "windows")` implementations:

- Current RSS: e.g. `GetProcessMemoryInfo` → `WorkingSetSize` (document as current WS, not macOS footprint / Linux statm).  
- CPU: `GetProcessTimes` → user/kernel durations for `process_cpu_seconds` and `sample_process` CPU field.  
- Peak: either real peak WS if available or keep peak distinct and **avoid `Some(0)`** — prefer `None` when unsupported (align peak with `current_resident_bytes` honesty).  

Panel already errors on total sample failure; keep that.

**Exit criterion:** panel resource card shows non-error CPU/RAM on native Windows; memory-profile JSON has non-null resident when sampler works; unit tests under `cfg(windows)` mirror linux/macos expectations.

### Hop 3 — Python (only if diagnostics must run on Win32)

- Lazy-import or cfg `resource` in `process_accounting.py`.  
- `server_resources.sample_process`: Win32 via `ctypes` (PSAPI/kernel32) **or** document “run collector under WSL against WSL PIDs only.”  
- Do not call Darwin `native_process_sample` on Windows.

### Hop 4 — Validation matrix (receipts, not vibes)

All receipts use **local engine + pack cache** only (no prod world).

| Receipt | Lane |
|---------|------|
| MSVC `panel-play` starts, vault unlock, one slot login to **local** engine, Game blit under **RDP** to the Windows laptop | A |
| Optional `BOT_CPU=1` vs default GPU on the 5060 | A |
| Resource card + one `BOT_MEMORY_N` panel sample with Windows provenance strings | A |
| WSL2 `tui-play` (no ALSA required for TUI) + Linux `server_resources` on WSL PIDs | B only |
| Still no Windows job in GH Actions until A is green locally | C |

### Explicit non-goals for the next hop

- Measurement policy redesign, cross-OS RSS equality, or waiving contaminated/null gates.  
- Treating WSL TUI N-bot tables as production Windows panel capacity.  
- Implementing full `lsof`-equivalent TCP counts on Windows unless a concrete harness needs them.

---

## File/line index (quick)

| Topic | Path |
|-------|------|
| RSS/CPU unix-only | `crates/host-play/src/rss.rs` L8–93, L119–128 |
| Benchmark CPU unix-only | `crates/host-play/src/memory.rs` L36–47, L1052–1066 |
| Panel sample fail-closed | `crates/panel/src/app.rs` L451–466 |
| Unix socketpair wake | `crates/host/src/slot_io.rs` L1–3, L176–226 |
| poll park | `crates/host/src/lib.rs` L690–745 |
| Client unix fd | `vendor/fr-client-rust/crates/client/src/io/client_stream.rs` L11, L109–130, L148–157, L264–269 |
| Vault not-unix write | `crates/vault/src/lib.rs` L308–360 |
| HOME only | `crates/script/src/isolated_env.rs` L28–35 |
| Python OS switch | `docs/memory/server_resources.py` L170–176 |
| Python `resource` import | `docs/memory/process_accounting.py` L27 |
| Darwin-only native sample | `docs/memory/native_process_sample.py` L31–33 |
| CI OS | `.github/workflows/ci.yml` L20, L36, L64 `ubuntu-latest` |
| CI ALSA/X11 why | `.github/workflows/ci.yml` L12–15 (panel `audio` / cpal); L44–55 apt set |
| TUI client features | `crates/tui/Cargo.toml` — `client` without `audio` |
| Panel client features | `crates/panel/Cargo.toml` L9 — `features = ["audio"]` |
| Client feature defaults | `vendor/fr-client-rust/crates/client/Cargo.toml` L6–9, L21–25 |
| Toolchain pin | `rust-toolchain.toml` channel `1.98.0` |

---

## Summary for orchestrator

Native Windows headed panel is blocked first by **unix poll/socketpair/AsRawFd** in host+client, then by **missing Win32 RSS/CPU samplers** (Rust and Python). Campaign validation stays on **local engine**. WSL2 is a viable **Linux** side path for TUI (`client` without `audio` — no libasound for TUI; still `native-tls`/`wgpu`/unix park) and `/proc`-based collectors only — not RDP/GPU proof. CI ALSA/X11 packages track **panel**, not TUI. Next implementation: compile abstraction → honest Windows metrics → local-engine RDP smoke receipts. **No Windows MSVC compile/test receipt yet.** Root commits and machine setup remain outside this audit.
