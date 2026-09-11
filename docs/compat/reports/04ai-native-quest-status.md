# Native quest-status transport

Date: 2026-09-11. Kanban: `t_95823e19`. Scope: brief 137 native API/script/host-play transport and tests only; no client, fixture, frozen catalog, quest-engine, journal, points, policy, or LIVE changes.

## Result

The headed host now publishes the existing `GameSnapshot::quest_statuses()` observation through the additive FlatBuffer snapshot boundary. Rust resolves each native quest-tab row colour to the frozen JavaScript strings before transport:

- `0xF80000` -> `notStarted`
- `0xF8F800` -> `inProgress`
- `0x00F800` -> `complete`
- every other colour -> `unknown`

These are the only classified colours in both selected frozen `Quests.ts` implementations. No quest table or text-derived completion policy was added.

The existing `quests.js` remains the thin consumer: `Quests.all()` returns the posted rows, and `Quests.status(name)` performs case-insensitive first-match lookup. Duplicate names preserve source order. Missing names return `unknown` once the quest tab is available.

## Availability and lifecycle

Availability is explicit and distinct from an empty loaded tab:

- loaded quest root, including zero rows: `quest_statuses` is an array;
- missing/unloaded quest root: `quest_statuses` is `null`, so the existing shim reports `Quests` unavailable;
- unchanged delta: the quest fields are omitted and the isolate retains its current per-slot value;
- available-to-unavailable transition, logout/session reset, or replacement without a snapshot: an explicit availability update sets the isolate value back to `null`.

The availability scalar is additive after the new vector. Buffers lacking both fields still verify and decode, initialize the JavaScript field as unavailable, and do not acquire a fabricated empty journal. The loader also accepts the vector-only additive draft as available. Fingerprinting includes availability and row order, so only changed quest data is emitted.

Each isolate owns its own snapshot object; focused tests verify that quest rows and availability do not cross slots.

## Native producer

`host-play::with_script_snapshot_input` reads only the current `GameSnapshot` quest rows. It marks the transport available only when the snapshot observed a loaded native quest-tab root. The producer test builds real client interface controls, including known red/yellow/green rows and an unrecognized heading colour, then decodes the resulting FlatBuffer. A later replacement without a snapshot emits an unavailable update rather than retaining stale rows.

## Public host shape

The generated host declaration adds:

- `QuestStatusRow { name, status }`
- `Snapshot.quest_statuses: QuestStatusRow[] | null`

`host-js/index.d.ts` was regenerated and its freshness test passes.

## Verification

All commands used `CARGO_TARGET_DIR=target-t_95823e19`, `--locked`, and `--offline`.

- focused API colour/string tests: PASS (2)
- focused script transport/query/lifecycle/isolation tests: PASS (5)
- focused real host producer test: PASS (1)
- generated declaration freshness: PASS
- all API and script library/integration test suites: PASS
- all host-play library tests: PASS (144)
- strict API/script and host-play library Clippy (`-D warnings`): PASS
- `git diff --check`: PASS before handoff

Exact command outputs and hashes are committed under `docs/compat/evidence/native-quest-status/README.md`.

LIVE requalification remains root-owned by brief 137 and was not run here.
