# Native NPC paint boxes

Date: 2026-09-11. Kanban: `t_c48f1d6a`. Scope: brief 144 reusable client projection, bounded host observation, additive snapshot transport, thin `reader.npcBox` compatibility facade, and focused regression coverage. No LIVE, scenario, fixture, catalog, navigation, TaskBot, or foreign-runtime changes.

## Result

`client::render::npc_overlay_box` now returns one live NPC's eight projected prism corners as overlay-canvas coordinates. It uses the native client's existing terrain-height and camera projection math through a pure read helper, honors the NPC's current size and model height, and preserves the historical ordering: four ground-ring points followed by the four top-ring points in the same winding. The fixed `area_game` projection origin is `(256, 167)` and the result adds the client's `(4, 4)` game-surface blit offset. The existing mutable renderer projection delegates to the same helper, with an equivalence assertion in the established overlay tests.

Projection fails closed when the client is not in a ready scene, the indexed NPC is absent or unresolved, size or height is non-positive, arithmetic cannot be represented, a corner is outside the playable scene, or a corner is behind the camera. No synthetic or partially projected box is returned.

The host samples only `npc_ids[..npc_count]`, projects each valid current NPC once, and posts an indexed replacement vector. Projection itself is lazy: it runs only on a tick edge for a Running load isolate, the same gate that posts the snapshot. Idle, Paused, compiled-only, and non-tick frames do not allocate or project NPC boxes. A held Running isolate still posts and projects before its paint-only tick, preserving the existing held-paint behavior. A frame that does not post cannot clear the isolate's current box.

## Transport and facade

The additive FlatBuffer payload carries `NpcBox { index, points }`, where each point is an integer screen coordinate. The native-facts fingerprint includes availability and the complete current vector:

- unchanged deltas omit the field and retain the current isolate value;
- an available empty vector replaces prior boxes after despawn or failed projection;
- an unavailable update clears prior boxes on a posted non-ready scene;
- old buffers initialize the reader as unavailable;
- replacement isolates start unavailable and cannot inherit another slot's geometry.

The isolate materializes compact native rows. `reader.npcBox(index)` validates a non-negative integer index, finds the exact indexed row, requires exactly eight finite integer points, and returns fresh `{x, y}` objects. Missing, malformed, stale-cleared, or out-of-range indices return `null`. JavaScript performs no camera, terrain, size, height, or scene projection.

## Frozen-source compatibility

Both frozen catalog inputs contain the same relevant `ClientAdapter.ts` and `FireGiant.ts` bytes:

- `100adccc037d9f6898080e1cad58fcfc43364775` `ClientAdapter.ts`: `4e17962b60e126e33a34e56e6155b2813227f4955787bbfb55204325f61b1425`.
- `100adccc037d9f6898080e1cad58fcfc43364775` `FireGiant.ts`: `b7463f7f4f81088270fbdf382aeb6da9eee58aa9ce64678382cd02b4f04e3f21`.
- `8e7d965be2071d6ec65c3265e12af797082d720a` `ClientAdapter.ts`: `4e17962b60e126e33a34e56e6155b2813227f4955787bbfb55204325f61b1425`.
- `8e7d965be2071d6ec65c3265e12af797082d720a` `FireGiant.ts`: `b7463f7f4f81088270fbdf382aeb6da9eee58aa9ce64678382cd02b4f04e3f21`.

The frozen contract is an optional eight-point current-frame box consumed by Fire Giant paint. No `locBox`, `playerBox`, planner, policy, or rendering runtime was imported.

## Verification

The exact checked export contains root commit `e05ecafb806b5a0af0b24bf4819ec8a919e5de28dc` with nested client commit `52c37f9ce50d1f184656d5b4469c007ec8a5791a`. All Cargo commands used `--locked --offline`; root and client used separate empty `target-review-t_c48f1d6a-r2` target directories.

- Script library tests: PASS (83).
- Native NPC-box transport/isolate tests: PASS (5).
- Generated host declaration tests: PASS (2; regeneration helper ignored).
- Host-play library tests: PASS (146), including live-client bounded extraction and lazy projection demand.
- Client NPC projection tests: PASS (4).
- Existing client overlay/projection equivalence tests: PASS (7).
- Strict Clippy for affected script, host-play, and client targets with `-D warnings`: PASS.
- Root and client `cargo fmt --all -- --check`: PASS.
- Scoped root and client `git diff --check`: PASS.

The first root implementation commit is `a1434ae2015a0af0b24bf4819ec8a919e5de28dc`; the projection-demand correction is `e05ecafb806b5a0af0b24bf4819ec8a919e5de28dc`. The client implementation commit is `52c37f9ce50d1f184656d5b4469c007ec8a5791a` on parent `aef3952d1cd7bb3b93d39c497f0f476b68021c59`. Subsequent fixture, TaskBot-validation, and qualification-documentation commits did not change task-owned paths and were not a reason to restart the exact checks.

Evidence, provenance, checked blob identities, and exact commands are under `docs/compat/evidence/native-npc-paint-box/`.

## Limits

No LIVE or catalog acceptance run was performed. Root owns composed-candidate and both-revision LIVE validation after review. This work does not add occlusion policy or partially clip a prism: if any required corner is unprojectable, the optional box is unavailable. It retains no geometry history and adds no `locBox`, `playerBox`, TaskBot, scenario, fixture, catalog, navigation, or foreign-runtime behavior.
