# Borrowed fingerprint comparison (plan §4B)

**Status:** implemented on `codex/memory-diagnostics`, pending review.
**Scope:** script fingerprint compare/update on the live Slot encode path only,
plus oracle/allocation tests and this report. No JS policy, encodings, channels,
scheduling, timeout, buffer pool, GameSnapshot narrowing, or client changes.
No remote/live RSS measurement (root freezes matched binaries after review).

## Problem

`IsolateBuf::encode_snapshot_delta` always ran `SnapshotFingerprint::from_input`
before `DeltaMask::changed`. That clones every string/vector field on every
post, including when the observe is content-identical to the last post.
`SlotScript::encode_snapshot_delta` then replaced `last_snapshot` with that new
owned value every time. Native attribution already saw large
`SnapshotFingerprint` / `SceneEntityFp` traffic; that is allocation churn, not a
proven steady RSS saving for this candidate alone.

## Representation / ownership design

| Piece | Role | Ownership |
|:---|:---|:---|
| `SnapshotInput<'_>` | Host observe for this tick | Borrowed views (`&str`, `&[…]`) into host temporaries |
| `SnapshotFingerprint` | Per-slot last-post baseline | Owned `String` / `Vec` retained on the slot |
| `DeltaMask` | Which wire fields the delta carries | `Copy` flags; `hold` still always true (SEC-004) |
| Wire buffer | FlatBuffer bytes posted to the isolate | New `Vec<u8>` from the reusable `IsolateBuf` (unchanged) |

**Compare path (before any owned next fingerprint):**

- `DeltaMask::changed_from_input(last, input, force_banks)` walks each field
  and compares owned baseline rows to borrowed input rows with the same
  equality semantics as `PartialEq` on the owned fingerprint types (order
  sensitive, omission vs empty unchanged at the wire layer).
- `force_banks` still forces the packed `banks` wire field even when the stand
  list is byte-identical (NavWorld identity change).
- Public `encode_snapshot_delta` / `IsolateBuf::encode_snapshot_delta` keep the
  same signatures and still return a fully owned `SnapshotFingerprint` via
  `from_input` for compatibility callers and tests.

**Retain path (live Slot only):**

- `IsolateBuf::encode_snapshot_delta_updating(&mut Option<SnapshotFingerprint>, …)`
  is the live owner path used by `SlotScript::encode_snapshot_delta`.
- Keyframe (`last == None`): mask = all fields; one `from_input`; store it.
- Later posts: borrowed mask → encode → `SnapshotFingerprint::retain_from_input`
  rewrites **only** fields whose content actually differs. `force_banks` alone
  does not re-clone an equal `banks` vector into the retained fingerprint.
- Unchanged content (including hold-only wire posts): retained fingerprint
  buffers are left in place — no `from_input`, no full struct replace.

**Preserved behavior**

- Field order, omission (absent ≠ empty), forced banks, keyframes, Start/Stop
  reset of `last_snapshot`, pause/resume retention, sent packet lifetime, and
  public encode return types stay as before.
- `hold` remains on every delta; fingerprint `hold` tracks the posted value.

## Tests run

1. **Oracle / differential** (`borrowed_fingerprint_oracle_matches_owned_encoder`):
   empty, rich keyframe, equal, force_banks, changed inv, reordered npcs, empty
   after populated. Borrowed mask matches owned `changed(last, &from_input, …)`;
   public encode bytes match the oracle encoder.
2. **Allocation/copy proof** (`live_updating_encode_avoids_fingerprint_rebuild_when_unchanged`):
   after a rich keyframe, unchanged live encode keeps the same `Vec` buffer
   pointers inside `last_snapshot`; force_banks keeps banks ptr; single-field
   inv change rematerializes only inv and leaves npcs/banks/stats stable.
3. **Existing restart** (`slot::tests::stop_releases_snapshot_storage_and_restart_emits_keyframe`):
   Stop clears fingerprint; restart keyframe matches first post.
4. **Full script suite:** `cargo test -p script --features load` — all passed
   (lib 42, load_isolate 144, gold/js/settings/etc.).
5. **host-play:** not re-run end-to-end here; Slot public API
   (`encode_snapshot_delta -> Vec<u8>`) and free
   `encode_snapshot_delta -> (Vec, SnapshotFingerprint)` signatures unchanged.
   Root coordinates full builds; no remote freeze in this task.

`memory-profile-no-alloc` is a host/panel/tui frontend feature, not a script
crate feature; script path verified under production `load`.

## What this proves / does not prove

| Claim | Status |
|:---|:---|
| Unchanged live posts avoid owned fingerprint reconstruction | Proven by pointer-stability / retain tests |
| Borrowed mask matches owned equality / wire omission | Proven by oracle tests |
| Steady process RSS reduction at N bots | **Unproven** — needs root matched freeze after review |
| Consumed-buffer pool / heightmap CoW / snapshot narrowing | Out of scope |

## Files

- `crates/script/src/isolate_fb.rs` — borrowed mask, retain, updating encode, tests
- `crates/script/src/slot.rs` — live path uses updating encode
- `docs/memory/borrowed-fingerprint-report.md` — this report
