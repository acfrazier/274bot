# Windows operator home path (HOME / USERPROFILE)

Task: `t_f0b3369e`. Host branch `codex/memory-diagnostics`, client
`codex/windows-native-parking` @ `35f1c13`. Mac unit tests only; native
Windows proof is root-owned after freeze.

## Problem

Native Windows typically has `USERPROFILE` and no `HOME`. Production path
defaults that only called `env::var("HOME")` fell back to cwd-relative
`.274bot/...`, and live harness helpers unwrapped missing `HOME`.

## Semantics

Shared pure selector (`operator_home_from(home, userprofile)`):

| Platform | Rule |
|----------|------|
| non-Windows | Verbatim `home` (`env::var("HOME")` shape). `USERPROFILE` ignored. |
| Windows | Explicit `HOME` wins **including empty string**. On `HOME` `Err` (absent or non-Unicode), use `USERPROFILE`. If both missing → `Err`. |

`operator_home()` = `operator_home_from(env::var("HOME"), env::var("USERPROFILE"))`.

Callers keep their own empty / missing fallbacks unchanged:

| Caller | Ok non-empty | Ok empty | Err |
|--------|--------------|----------|-----|
| `engine_dir` (after `ENGINE_DIR`) | `$home/experiments/Server/engine` | same join (empty prefix) | relative `experiments/Server/engine` |
| `unpack_dir` | `$home/.274bot/unpack` | relative `.274bot/unpack` | relative |
| `script::bot_home` | `PathBuf` home | `"."` | `"."` |
| pack/smoke defaults | `$home/.274bot/...` | join with empty home | relative `.274bot/...` |
| live `options()` / e2e pack_path | require Ok (expect) | Ok empty path | panic |

No dependency additions, no global env mutation, no filesystem creation, no
protocol/action/credential/settings changes.

## Dependency boundary

- Canonical helper: `client::bot_target::{operator_home, operator_home_from}`
  (re-exported from `client`).
- `script` only has `client` as a **dev**-dependency. Production
  `script::bot_home` uses a **local tiny cfg adapter** with identical
  rules so the crate graph is not widened.

## Affected paths

Client:

- `vendor/fr-client-rust/crates/client/src/bot_target.rs` — helper + tests + `engine_dir`/`unpack_dir`
- `vendor/fr-client-rust/crates/client/src/lib.rs` — re-exports
- `vendor/fr-client-rust/crates/client/src/bin/unpack-cache.rs` — default cache/out

Host workspace:

- `crates/script/src/isolated_env.rs` — local adapter + `bot_home` + selector unit test
- `crates/nav/src/world.rs` — test `default_pack_path`
- `crates/nav/src/bin/nav-pack.rs` — `default_out`
- `crates/host-play/src/lib.rs` — `default_pack_path`
- `crates/host-play/src/scatter.rs` — `pack_path`
- `crates/host-play/src/nav_capture.rs` — smoke root
- `crates/host-play/tests/common/mod.rs` — live `options()`
- `crates/scenario/src/lib.rs` — `default_pack_path`
- `crates/scenario/src/shot.rs` — `default_shot_root`
- `crates/e2e/tests/common/mod.rs` — live `options()`
- `crates/e2e/tests/nav_collision.rs` / `nav_seers_crabs.rs` — pack fallback
- `docs/memory/windows-home-path-report.md` — this file

**Not touched** (owned by `t_597230fa`): `host-play` `rss.rs`/`memory.rs`/`Cargo.toml`,
`Cargo.lock`, `panel/src/resource.rs`. Host crate / frozen socket stream untouched.

Unrelated client test HOME reads (`unpack_cache`, `gpu_depth`, `iface_model`)
and isolation-test process-HOME assertions left as-is.

## Tests run (Mac)

See task completion metadata for exact cargo invocations and results.
Native Windows proof pending root freeze.
