# Native quest status transport evidence

Captured 2026-09-11 11:52 EDT on branch `codex/rs2b0t-multirevision` from campaign base `6dd6f9102b1111fdcadf551ccfd3a24658d81661` with client gitlink `aef3952d1cd7bb3b93d39c497f0f476b68021c59`.

Every Cargo invocation used the task-exclusive `CARGO_TARGET_DIR=target-t_95823e19`, `--locked`, and `--offline`.

| File | Command | Result | SHA-256 |
|---|---|---|---|
| `api-test.txt` | `cargo test -p api --test native_quest_status -- --test-threads=1` | PASS, 2 tests | `af6b409f773dd22a38e32e0336fd9b04e76debad91e1338ef3553a6c94e21663` |
| `script-test.txt` | `cargo test -p script --test native_quest_status -- --test-threads=1` | PASS, 5 tests | `3a68bc8fdf25259a51c8cd959b31e880ca26031a4e4eac6a8b286accc9f27352` |
| `host-producer-test.txt` | `cargo test -p host-play --lib script_snapshot_posts_native_quest_rows_and_clears_them_without_a_snapshot -- --test-threads=1` | PASS, 1 test | `cebe8ddbe316ed52c0481dc69f838b92e725105b04cabd121d35ace3001d5900` |
| `host-js-test.txt` | `cargo test -p script --test host_js host_js_dts_is_fresh -- --test-threads=1` | PASS, 1 test | `d645355e03bdfc3e3c44d40db38ade5c55d8dc56f53b224e2b7c2856b7e28ac1` |
| `api-script-full-test.txt` | `cargo test -p api -p script --lib --tests -- --test-threads=1` | PASS, all selected package suites | `9e421605437c3d8fbbd7189d3654edac2e11e0ee9fe11f6f4327a6b8fa814188` |
| `host-play-lib-test.txt` | `cargo test -p host-play --lib -- --test-threads=1` | PASS, 144 tests | `bd287652365ef532294fc1c55263a55e6eb180e370b96503ff84ef4f0d741eee` |
| `api-script-clippy.txt` | `cargo clippy -p api -p script --lib --tests -- -D warnings` | PASS | `ac53664daa6ef45551c0c0a671cd6e389ab73e845c84e1ed6c646304f71f5a0f` |
| `host-play-clippy.txt` | `cargo clippy -p host-play --lib -- -D warnings` | PASS | `155dcdccca6529cf7e28b1ad95550108d78ebc6f6c067040c3d7c6f6f278e371` |

The focused script suite covers exact status strings, case-insensitive lookup, duplicate-name first-match behavior, unknown rows, old buffers without the additive fields, changed-to-unavailable clearing, loaded-empty availability, and two-isolate separation. The host-play test constructs the native quest-tab controls and verifies producer encoding plus a later unavailable replacement update.
