# Appearance packet box implementation

Implements the ONE approved sparse appearance storage proposal from
`docs/memory/incremental-owner-attribution-report.md` (Grok 4.5 approved
`t_be58be8d` on `8d1ea2d` + prose clarification `6417872`).

## Scope done

| Item | Result |
|---|---|
| `Client::player_appearance_buffer` | `Vec<Option<Packet>>` → `Vec<Option<Box<Packet>>>` |
| Receive path | `Some(Box::new(Packet::new(data)))` once per appearance |
| Indices | Still `MAX_PLAYER_COUNT` (2048) slots |
| take / update / restore | Unchanged control flow; `Box` moves, `set_appearance` still gets `&mut Packet` via deref |
| Login / reset clear | Existing `iter_mut` → `None` retained |
| Packet / ISAAC / wire / crypto | Untouched |
| Host action API / reader gates | Untouched |

## Structural note (not RSS)

Approved layout arithmetic (arm64, rustc 1.98.0 against cached client rlib):

- `Packet` = 2112 (inline `Option<Isaac>` with 2064-byte Isaac)
- `Option<Packet>` = 2112
- `Option<Box<Packet>>` = 8
- Empty table: 2048 × (2112 − 8) = **4,308,992 bytes = 4.109375 MiB** structural difference per client

This is **not** measured RSS. Populated slots pay `Box` allocator overhead plus the full `Packet` on the heap; actual occupied overhead was not measured here. Root owns freeze and clean N1/N16 measurements.

## Code touch list

Client submodule branch `codex/memory-appearance-packets` (base HEAD still
`451759f2` — **no git commit by this task**; working tree only):

- `crates/client/src/client/client.rs` — field type, doc, box on write
- `crates/client/tests/login.rs` — fixture `Some(Box::new(Packet::new(...)))`
- `crates/client/tests/player_info.rs` — new remove/re-entry regression

Host workspace branch `codex/memory-diagnostics` @ `6417872` (this report only;
submodule gitlink not updated by this task):

- `docs/memory/appearance-packet-box-report.md` (this file)

## Protocol regression

New test `player_info_cached_appearance_reapplied_after_remove_reentry`:

1. `PLAYER_INFO` new-vis introduces remote indices 5 and 7 with distinct
   APPEARANCE blocks (gender/combat differ).
2. Asserts both ready, cached buffer occupied, and mutating slot 5 packet
   bytes does not change slot 7 (independent boxed ownership).
3. Leaves buffer `pos` at end-of-packet so re-entry must reset the cursor.
4. Advances `loop_cycle`, sends empty old-vis removal frame; players drop,
   appearance cache retained.
5. Re-enters 5 and 7 **without** extended appearance; cached packets re-apply
   with correct gender/combat per slot.

Existing `player_info_appearance_lands_on_local_player` and login appearance
clear tests retained.

## Commands and results

Workspace: `/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`

Client (run inside `vendor/fr-client-rust`):

```text
cargo test -p client --test player_info -- --nocapture
# ok. 2 passed (including new remove/re-entry)

cargo test -p client --test login --test server_packets --test gens --lib
# lib: 66 passed
# gens: 12 passed
# login: 10 passed
# server_packets: 19 passed
```

Host:

```text
cargo test -p tui --features memory-profile-no-alloc --lib
# ok. 92 passed

cargo test -p host-play --features memory-profile-no-alloc --lib prepare_
# ok. 12 passed (isolated filter)

cargo test -p host-play --features memory-profile-no-alloc --lib
# 171 passed; 4 failed — see Limitations
```

## Limitations

1. **No commit / gitlink freeze.** Root owns client commit on
   `codex/memory-appearance-packets`, host gitlink update, and review of the
   frozen SHAs. Implementer left working diffs only; base client HEAD remains
   `451759f2`, host `6417872`.
2. **No live / matched RSS matrix.** Explicitly out of scope; structural
   4.109375 MiB empty-table delta is not a resident claim.
3. **host-play full `--lib` flake under concurrent load.** Four
   `memory::tests::prepare_*` cases failed twice with
   `vault already exists: .../T/274bot-memory-live10b…/vault` while root TUI
   origin smoke / other agents can share the machine. The same `prepare_`
   filter run alone passed 12/12. Failures are temp-vault path collisions,
   not appearance-buffer behavior. Not treated as a NEW product failure from
   this change; isolated re-run is green. TUI lib suite was clean.
4. **Populated overhead** unmeasured (table pointer + heap Packet + allocator
   padding when many slots filled).

## Branch verification (read-only)

| Tree | Branch | HEAD |
|---|---|---|
| Host worktree | `codex/memory-diagnostics` | `6417872fa2f99e608f2a104f39cc8e8932ec47d9` |
| Client submodule | `codex/memory-appearance-packets` | `451759f2a7df9c57895657d5b8d506172860cee1` (base; dirty working tree) |

Neither is `main`. No checkout/switch/commit/push/gitlink mutation by this task.


## Orchestrator verification and commit freeze

The orchestrator committed the three client files as `85266df` on `codex/memory-appearance-packets` after reviewing the diff. The regression test lost one redundant same-value write and its colour-index comment was corrected; `cargo test -p client --test player_info` passed both tests again. No implementation behavior changed in that cleanup.

The full host suite was rerun after live smoke completion with `cargo test -p host-play --features memory-profile-no-alloc --lib -- --test-threads=1`: **175 passed, 0 failed**. Earlier parallel temp-vault collisions remain recorded above; the complete serial pass replaces reliance on the filtered pass. Root owns the host gitlink/report commit and later matched measurements; no RSS saving is accepted yet.
