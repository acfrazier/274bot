# Host snapshot and reconnect publication

Date: 2026-09-10. Scope: step 4 snapshot/lifecycle boundary on
`codex/rs2b0t-multirevision`. Client base `2be16970`, candidate
`6cb5a0b17aeef74da6b57b205e916681daee4f76`. Host parent `6277a93f`.
Source checks pass; independent review and controlled live proof are pending.

## Result and ownership

The client records actual PLAYER_INFO publication, successful login/reconnect
identity and explicit all-family invalidations. Successful responses 2 and 15
also save the exact generation watermark at the grant. These are generic
observations: packet bytes, old family generations, login order, response
handling and timeouts are preserved. Failed login/reset cannot create a
successful-session identity. The additional client state is 136 inline bytes
(two u64 observation counters plus invalidation counter, and a 112-byte grant
watermark); it is not a world copy or a performance claim.

`Pump::drain_client` excludes packets decoded before a reconnect, even when
`tcp_in` replaces the stream and continues reading within the same frame.
`host::publish_snapshot` is shared by the host slot, production host-play
observer and controlled live harness. Reset drops current views and retains the
grant watermark. Packet families require a new-session observation; explicit
all-family invalidations are excluded from that decision. A later scene rebuild
or unrelated interface packet cannot republish a retained actor or inventory.
Fresh packets after the grant in the same drain remain publishable. Scene-ready
scalars still refresh when `check_scene` changes state without a packet; the
existing family gates continue to own collision and other large shared data.

The host-play observer closes its published producer gate before clearing
cheat/wire queues, route work and script work. Producers keep that gate locked
through enqueue, so an old producer cannot append after reset. The reset runs
before the frame hook and script/nav dispatch. Guardian and auto-run state also
reset at the connection boundary. Actions wait for a current local player.
Script ticks stay monotonic across the outer login loop.

The Rust isolate transport tags ticks, interaction batches and completions with
a dispatch generation. Reset discards unread and late old batches, skips old
queued ticks and clears only the host interaction queue. It preserves the
script instance, operator run intent, Pause/Stop behavior and parked wait
deadlines. Reset also clears the snapshot fingerprint, forcing the next post to
be a complete keyframe. No FlatBuffer schema or foreign JS policy changes.

The inventory passed to script dispatch now borrows the current snapshot.
Snapshot observation can refresh between ticks, while encoding/posting remains
gated to Running script tick edges. No per-frame inventory zipper is added.

## Verification

- API unit/integration tests: 173 passed. Existing snapshot sharing/idempotence
  coverage remains green, including reset/retained-iface tests.
- Host unit tests: 136 passed. Production decoder fixtures cover both revisions,
  g2 inventory counts/gsmart slots, bank and interface updates, player/NPC
  publication, REBUILD without a tick, and logout clearing.
- Real response-15 mock handshakes cover old packets before the grant, REBUILD
  after it, fresh player/interface packets, and later fresh inventory. A separate
  same-drain case retains new inventory and PLAYER_INFO immediately after grant.
- Host-play: 116 default unit tests and 147 with `memory-profile` passed; support
  integration tests also passed. Live tests remained ignored in these runs.
- Script: 41 unit tests passed, 144 existing isolate tests passed, the new unread
  batch reset test passed after correcting its fixture tag, and all 8 slot state
  tests passed. The queued-old-tick test uses a real isolate and passed.
- Separate client integration targets `gens`, `login`, `lost_con`,
  `revision_289_actors`, `revision_289_stage2`: 97 passed. Final gens target rerun:
  14 passed. These are separate from host compilation of the client library.
- Strict Clippy across API/host/host-play/script all targets with
  `host-play/memory-profile`: passed. Host/client formatting and diff checks pass.

Raw logs: `evidence/host-boundary/snapshot/`. Earlier compile failures, obsolete
synthetic player-generation fixtures and the malformed new interaction fixture
are retained. Corrections used actual PLAYER_INFO and the existing `op` request
tag; no assertion, timeout or live predicate was weakened. No live cell has run
for this change yet. The harness source now consumes the shared production
publication helper; setup-only fixture preflight is in the neighboring `live/`
folder.

The operator's later instruction makes script loading and starting revision
agnostic. Earlier requirements for revision/catalog loading gates are superseded;
root is applying that separately. Operation refusal stays at the host/client
boundary, and catalog qualification remains evidence rather than an allowlist.
