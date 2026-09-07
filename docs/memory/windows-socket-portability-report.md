# Windows socket wake and park portability report

**Task:** t_d650a099  
**Branch (host):** `codex/memory-diagnostics` (must remain; no git mutations by implementer)  
**Client submodule branch:** `codex/windows-native-parking` @ `e17deab` base + local diff  
**Machine:** macOS implementer lane only  
**Date context:** operator note — Windows SSH healthy; MSVC Build Tools installer running; root will install Rust 1.98.0, freeze this diff, and run native on-box tests. Audit docs only at `3d4645f`.

## Scope completed

First native-Windows compile prerequisite only (hop 1 from `docs/memory/windows-readiness-audit.md`):

- Socket readability waits for host idle park and client WSS `available`
- Slot control wake/park portability

**Not done (by design):** measurement backends, HOME/USERPROFILE, protocol/catalog, JS compatibility, action API in client, Mac live runs, Windows remote actions, performance claims, native Windows compile/test claims from Mac.

## Round-2 review fixes (must-fix before freeze)

Addressed reviewer/orch feedback:

1. **No heap per park/wait**
   - Unix `park` restored **verbatim** behind `cfg(unix)`: stack `[libc::pollfd; 2]` + inline `libc::poll` (no `Vec`).
   - Windows `park` counterpart: stack `[WSAPOLLFD; 2]` + `WSAPoll` (max 2 handles).
   - `wait_readable` returns `[bool; MAX_WAIT_HANDLES]` with stack `[pollfd|WSAPOLLFD; 2]` — no `Vec` output or pollfd heap on either OS.
   - Public `SlotPark::wait_readable` wraps one stack entry (bool).
2. **Windows wake nonblocking fails loudly** — `set_nonblocking(true)?` in `loopback_tcp_pair` (no `let _ =` discard).
3. **TCP_NODELAY** on both ends of the Windows wake pair (`set_nodelay(true)?`) so kick latency is not Nagle/delayed-ACK bound.

## Design

### Shared wait abstraction (`crates/host/src/slot_io.rs`)

- `WaitHandle`: Unix `RawFd`, Windows `RawSocket` (pointer-width `SOCKET`)
- `MAX_WAIT_HANDLES = 2` (control + client socket)
- `wait_readable(handles, timeout) -> [bool; 2]` (stack-only; first `handles.len()` entries meaningful):
  - Unix: `libc::poll` on fixed `[pollfd; 2]` (`POLLIN|HUP|ERR|NVAL`)
  - Windows: `WSAPoll` on fixed `[WSAPOLLFD; 2]` (no fd truncation, no busy-spin / sleep polling)
  - Empty handle list: pure `thread::sleep(timeout)` (park no-fds path)
  - Timeout ms conversion: `i32::try_from(as_millis()).unwrap_or(i32::MAX)` (same as prior park)

### Control wake

- **Unix unchanged:** `UnixStream::pair()`, nonblocking, kick byte, drain loop
- **Windows:** owned nonblocking + TCP_NODELAY loopback TCP pair (`127.0.0.1:0`)
  - Peer identity checked: accept `peer == client.local_addr()` and `server.peer_addr()`
  - `set_nonblocking` / `set_nodelay` errors propagate via `?` through pair construction
  - Same kick/drain/coalesce/`Arc` clone / drop behavior
- Public surface:
  - Unix: `SlotPark::fd()` kept
  - Windows: `SlotPark::raw_socket()` added
  - Portable: `SlotPark::wait_readable(timeout)` for tests / host-play (one stack entry)

### Host park (`crates/host/src/lib.rs`)

- **Unix:** original production body restored under `cfg(unix)` — stack pollfds, no shared wait helper in the hot path
- **Windows:** parallel structure with stack `WSAPOLLFD`s
- `poll_socket == false` omits socket handle (stall suppression)
- On control fire: always `drain()` before classifying
- **Both ready → `ParkWake::Socket`** (socket priority preserved)
- Else control → `Control`; else `Timeout`

### Client stream (`vendor/.../client_stream.rs`)

- Unix `AsRawFd` / `fd()` surface **unchanged** for callers
- Windows: `AsRawSocket` / `raw_socket()`; WSS stores `RawSocket`
- WSS `available()` zero-time readability: Unix `poll` timeout 0; Windows `WSAPoll` timeout 0
- TCP `available` still peek + nonblocking restore; 30s read timeout, writer path, EOF/close, byte counters untouched

### Dependencies

- `client` + `host`: `libc` under `cfg(unix)`; `windows-sys` 0.59 `Win32_Networking_WinSock` under `cfg(windows)`
- Workspace `Cargo.lock` updated on Mac resolve (windows-sys already present transitively)

## Tests added / adapted

| Test | Covers |
|------|--------|
| `slot_io::wake_channel_fires_…` / `clones_share_one_wake` | Portable wait (no raw `libc::poll` in test body) |
| `wake_channel_queued_kicks_drain_without_residual` | Coalesced kicks + drain leaves no residual wake |
| `wait_readable_reports_socket_data_and_close` | Socket data + peer close readability |
| `wait_readable_honors_bounded_timeout` | Bounded timeout |
| `park_prefers_socket_when_control_and_socket_both_ready` | Both-ready priority |
| `park_suppresses_socket_when_poll_socket_false` | No socket re-poll when suppressed; control still works |
| `park_no_fds_sleeps_timeout` | No-handles sleep |
| `client_stream::reader_handle_tracks_…` | Compiles on Unix+Windows cfg (was `all(test, unix)`) |
| host-play `stop_slot_wakes_a_parked_thread_before_joining` | Uses `SlotPark::wait_readable` instead of `libc::poll` |

## Mac verification (this machine only; after round-2 fixes)

Commands and results:

```text
cargo test -p client --lib io::client_stream
# 1 passed

cargo test -p host --lib
# 179 passed; 0 failed; 1 ignored (gpu_real_queue)

cargo test -p host-play --lib stop_slot_wakes_a_parked_thread_before_joining
# 1 passed

cargo test -p host-play --lib --features memory-profile-no-alloc -- --test-threads=1 wake_channel stop_slot park
# 4 stop_slot* passed (filter matched stop_slot*)

cargo test -p host-play --lib --features memory-profile-no-alloc memory::
# 45 passed (shared-vault / env_lock fixtures)
```

**No-alloc park path:** production `park` does not call `wait_readable` and does not allocate; Unix body is the pre-change stack poll. Test-only `wait_readable` is also stack-fixed (max 2). memory-profile-no-alloc host-play suite green as above.

**Not claimed:** `x86_64-pc-windows-msvc` compile, Windows unit tests, panel-play on Windows, RDP/GPU.

## Diff surface (implementer working tree; root owns commit freeze)

Host tree:

- `crates/host/src/slot_io.rs`
- `crates/host/src/lib.rs`
- `crates/host/Cargo.toml`
- `crates/host-play/src/lib.rs` (poll test only)
- `Cargo.lock`
- `docs/memory/windows-socket-portability-report.md`

Client submodule (`codex/windows-native-parking`):

- `crates/client/src/io/client_stream.rs`
- `crates/client/Cargo.toml`

## Remaining Windows proof (root)

After root freezes source + installs MSVC/Rust 1.98.0 on the laptop:

1. `cargo test -p client --lib io::client_stream --target x86_64-pc-windows-msvc`
2. `cargo test -p host --lib --target x86_64-pc-windows-msvc`
3. `cargo test -p host-play --lib stop_slot_wakes_a_parked_thread_before_joining --target x86_64-pc-windows-msvc`
4. Exit criterion from audit hop 1: `cargo build --release -p panel --bin panel-play --target x86_64-pc-windows-msvc` (may still fail on later blockers; this hop only removes unix poll/socketpair from the live path)

## Reviewer notes

- Request review profile **reviewer** (Grok 4.5).
- **Root commit freeze and on-box Windows proof are pending** — do not treat Mac green as Windows green.
- Unix production park body restored verbatim behind `cfg(unix)`; Windows is counterpart only; no per-park heap on either OS.
