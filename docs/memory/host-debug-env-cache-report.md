# Host debug environment cache report

## Scope

This bounded correction addresses Grok 4.6 integration review finding 1. It changes only `crates/host/src/lib.rs`; the reviewed capture lifecycle, input trace module, client, panel, reader, publisher, native, and launcher remain untouched.

## Source correction

`host::debug_enabled()` now reads `BOT_DEBUG` through a process-wide `OnceLock<bool>`. The initializer uses `std::env::var_os("BOT_DEBUG")` once and accepts only an exact `OsStr("1")`; unset, `0`, other strings, and non-Unicode values are false. Subsequent calls load the cached decision and still read the live `DEBUG` atomic first, preserving the OR contract:

- `BOT_DEBUG=1` enables host debug logging.
- `set_debug(true)` enables it immediately.
- `set_debug(false)` disables only the host latch; the cached environment decision remains effective.
- No environment lookup or environment-string allocation occurs on repeated calls after initialization.

`input_seam_trace::enabled()` remains independent and unchanged. It retains its own test force override and cached opt-in path; host logging does not call the trace gate.

## Tests

Added host unit coverage for:

- exact `BOT_DEBUG` matching across unset, `0`, `1`, other text, whitespace, and (Unix) non-Unicode values;
- one-time cache initialization using a counting initializer;
- independent live `set_debug(true/false)` transitions against the cached environment result.

Commands and results:

- `cargo test -p host --lib bot_debug_env -- --test-threads=1` — 2 passed, 0 failed.
- `cargo test -p host --lib set_debug_is_a_live_latch -- --test-threads=1` — 1 passed, 0 failed.
- `cargo test -p host --lib input_seam -- --test-threads=1` — 7 passed, 0 failed.
- `cargo test -p host --lib -- --test-threads=1` — 213 passed, 0 failed, 1 ignored (the existing real-GPU adapter test).

The repeated-check test uses an isolated local `OnceLock` and counting initializer rather than mutating process-global environment state. Production source inspection confirms `debug_enabled()` has one `get_or_init(read_bot_debug_env)` path and no per-call `std::env` access; `set_debug` remains a separate live atomic read.
