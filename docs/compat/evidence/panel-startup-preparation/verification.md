# Panel startup preparation verification

Task: `t_779f58e5`
Date: `2026-09-10T23:45:17Z`
Branch: `codex/rs2b0t-multirevision`
Implementation base: `ca7d4c24c301898bbdf19e51d34f23755e76d3b1`
Implementation commit: `814e5293fec426b722e450580c514b9cffca826f`

## Source identity

```text
c04b072e4a2cfdec3df14646e8e2063f3f40728bba44e04b160ab8c2ffc3fc66  crates/host-play/src/lib.rs
98bbc40fed97755fa8d6906fd74070a26630eef641467c44e6033ddbcd424204  crates/host-play/src/profile.rs
1ba6ccef5b4b1a49a55c7f0804586a5b9969814e96e706b3a6601335b63f5548  crates/host-play/tests/session_profile.rs
8af2c5477dfc9ee0b54a52c1d0a29b809b4dd093943ada99ee61df6b37700c57  crates/panel/src/app.rs
81a44f0780bf1bb2e7fac470f6cf9c3fb49f1ce01d2a8cff20e26495e24822d4  crates/panel/src/session.rs
```

`git show --stat 814e5293` reports only those five files: 652 insertions and 56 deletions.

## Final checks

All commands ran in the shared campaign checkout after the source was formatted.

- `cargo test -p host-play --test session_profile` — 11 passed, 0 failed.
- `cargo test -p host-play --lib` — 125 passed, 0 failed.
- `cargo test -p panel --lib` — 389 passed, 0 failed.
- `cargo fmt -p host-play -p panel -- --check` — passed.
- `cargo clippy -p host-play --all-targets --all-features -- -D warnings` — passed.
- `cargo clippy -p panel --all-targets --all-features -- -D warnings -A clippy::type-complexity` — passed. The narrow allow is for four unchanged pre-existing complex frontend map signatures already present at the implementation base; the initial strict run also found one task-owned needless borrow, which was corrected before this final pass.
- `git diff --check` on the five source/test paths — passed before commit.

The first full panel run exposed a collision between two parallel tests using the same process-wide fixture directory: 387 passed and 2 failed with missing copied cache files. The fixture path was made thread-unique; the complete 389-test rerun then passed.

## Covered behavior

- `ProfileSelection::prepare_template` keeps bind and template-load validation together on a worker-owned captured input.
- Cache, navigation pack, and flags mutations are rejected by the final validation path.
- The checked `run_with_template` entry still validates disk resources.
- `ValidatedTemplate` has private fields, can only be produced by final validation, and is consumed by `run_prepared_template`; the prepared Play path does not hash again.
- Stale preparation generations do not install a profile, template, vault, Play, or slot.
- Preparation failure leaves vault, Play, and slots absent and preserves the actual error.
- Normal Unlock waits for preparation and final validation, then starts from the prepared template without rebinding.
- Deferred live boot remains pending while final validation is in flight; no vault, Play, scenario, or slot is created first.
- The first UI frame still presents before preparation begins, and UI polling/request-redraw continues while workers are active.

## Limits

No native Mac or Windows panel run was performed by this task. Root owns responsiveness and changed-resource proof on disposable full-size resources. This evidence does not claim a hashing speedup, a responsiveness budget, or elimination of the existing hash-to-construction filesystem TOCTOU class. TUI/CLI keep the checked synchronous entry. No LIVE harness was launched.
