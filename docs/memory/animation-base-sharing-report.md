# Private animation-base sharing — implementation report

Implements approved design `docs/memory/animation-base-sharing-proposal.md`
(host commit `79d8e5d`, design task `t_984bbb51` APPROVED). Bounded under
performance-finish-plan §4A. No live measurements, RSS claims, or freeze in
this hop; root owns client commit + gitlink + matched binary freeze later.

## Checkout

| Item | Value |
|------|-------|
| Host branch | `codex/memory-diagnostics` |
| Host HEAD (docs/report only here) | `79d8e5d` at task start; report uncommitted |
| Client submodule branch | `codex/memory-animation-bases` |
| Client submodule HEAD at task start | `85266df` |
| Client changes | **uncommitted** (root freezes SHA later) |
| Worker git | no commit / stage / branch switch / push |

## Baseline-first (public fixtures on unchanged representation)

Added four public behavior unit tests under
`vendor/fr-client-rust/crates/client/src/dash3d/anim_frame.rs` `tests` module
and ran them **before** the private-store change:

```text
cd vendor/fr-client-rust
cargo test -p client --lib -- anim_frame::tests::public_
```

Result on **unchanged** clone-per-frame store (after one fixture expectation
fix for transparency default axes `0` not `128`):

```text
running 4 tests
test ...public_multi_frame_defaults_origin_signed_and_transparency ... ok
test ...public_get_results_are_independent_nested_and_transforms ... ok
test ...public_partial_republish_preserves_old_result_and_sibling ... ok
test ...public_init_high_id_missing_and_struct_construction ... ok
test result: ok. 4 passed; 0 failed
```

Coverage: multi-frame shared base contents; ORIGIN auto-insert; SCALE default
128; signed transforms; TRANSPARENCY → `opaque[id]=false`; model vertex
translate via `Model::animate`; nested label/`type`/transform mutation
independence on `get` results and siblings; partial republish preserving old
owned result + untouched sibling; grow-only `init`; high ID; missing/negative
IDs; public `AnimFrame` / `AnimBase` / `AnimFrameStore` construction;
`animate_transparencies` stub (`-1` only).

## Implementation

**Source scope (client only):**

- `vendor/fr-client-rust/crates/client/src/dash3d/anim_frame.rs`

**Not changed:** `anim_base.rs`, `model.rs`, public field types of
`AnimFrame`/`AnimBase`/`AnimFrameStore`, packet format, SeqType delay path,
scheduler, host crates (except this report).

**Private representation:**

- Process-wide store is `PrivateStore { list: Vec<Option<PrivateFrame>>, opaque }`
- `PrivateFrame` holds frame-specific delay/size/ti/tx/ty/tz and
  `Option<Arc<AnimBase>>`
- Public `AnimFrameStore` type kept for API shape; not used as the live store
- `unpack`: decode one base → `Arc::new` once per unpack → `Arc::clone` into
  each published record (no cross-archive interning)
- `get`: materializes fully owned public `AnimFrame` via deep `AnimBase` clone;
  does not retain the Arc
- Incomplete public frames with `base: None` are never stored or returned from
  `get` for populated records
- Init grow-only, ID growth, replacement, opacity, scalar `delay`, and
  `animate_transparencies` behavior preserved

**Private ownership proofs (after change):**

- Same-unpack frames share one Arc (`ptr_eq`); separate unpacks do not
- `get` does not bump Arc strong count
- Partial replace keeps old Arc while siblings remain; replacing final owners
  drops the old base (`Weak` upgrade fails)

## Post-change verification

### Client unit (lib)

```text
cd vendor/fr-client-rust
cargo test -p client --lib
→ test result: ok. 72 passed; 0 failed
```

Includes 4 public + 2 private anim_frame fixtures.

### Client integration (separate binaries)

| Command | Result |
|---------|--------|
| `cargo test -p client --test seq_delay` | ok. 1 passed |
| `cargo test -p client --test inject` | ok. 7 passed |
| `cargo test -p client --test zone` | ok. 45 passed |
| `cargo test -p client --test iface_model -- --skip gpu_` | ok. 5 passed |
| `cargo test -p client --test iface_model -- gpu_` | **FAILED** 0 passed; 4 failed |

Honest GPU `iface_model` failures (unchanged class; also noted on scalar-delay
report as pre-existing overlay/compositor issues, not animation-store):

- `gpu_draw_does_not_crash_on_mysterious_cube_modal` — 0 composite px
- `gpu_main_modal_rect_is_opaque_over_the_scene` — scene hole remains 0
- `gpu_ship_journey_paints_the_title_over_the_scene` — 0 overlay px
- `gpu_ship_journey_stays_over_a_frozen_scene` — 0 overlay px

Non-GPU iface_model and all other listed suites passed.

### Host-play / TUI (`memory-profile-no-alloc`)

From host worktree
`/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`:

```text
cargo test -p host-play --features memory-profile-no-alloc -- --test-threads=1
→ lib: ok. 175 passed; 0 failed (also full package exit 0; live ignored)

cargo test -p tui --lib --features memory-profile-no-alloc
→ ok. 92 passed; 0 failed
```

## Pending (root / later hops — not claimed here)

- Client commit + host gitlink freeze; publish frozen SHA on kanban card
- Matched appearance-candidate baseline vs new candidate binaries
- Functional live qualification and short resource screens
- Native allocation confirmation (surviving base count, allocator overhead)
- Clean CPU/latency cells; any longer stage under investigate-or-park
- No accepted RSS saving; cannot close N16 per-bot budget alone
- Final whole-branch Grok4.6 review still required

## Reviewer note

Client changes are **uncommitted**, pending root freeze. Do **not** FINAL-approve
until orch posts the frozen client commit SHA on this card. Implementation
worker stopped at `kanban_request_review` per charter.

## Orchestrator freeze

Client commit `e17deab` on `codex/memory-animation-bases` freezes the implementation. Root tightened the missing-ID fixture to assert the selected missing ID directly, removing its unrelated OR fallback; all six animation fixtures passed again. Earlier full test results remain as recorded. GPU failure class is documented before the scalar-delay change in `scalar-delay-experiment.md`; it is not a passing renderer result. Independent code review and live evidence remain pending.
