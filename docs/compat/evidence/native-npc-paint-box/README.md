# Native NPC paint-box evidence

Task: `t_c48f1d6a`

Source provenance:

- brief: `docs/compat/briefs/144-native-npc-paint-box.md`
- brief SHA-256: `47cc91859323a800924cddf473592e51aa0874246dc0a05b65873333cf6c296e`
- root implementation base: `2c4fd6040f9cd0c9ab9b90143fd91576eaebf362`
- root implementation commit: `a1434ae2015a0af0b24bf4819ec8a919e5de28dc`
- root projection-gate correction: `e05ecafb806b5a0af0b24bf4819ec8a919e5de28dc`
- checked root archive: `.superpowers/review-exports/native-npc-paint-box-t_c48f1d6a-e05ecafb8-r1`
- checked root commit: `e05ecafb806b5a0af0b24bf4819ec8a919e5de28dc`
- client base: `aef3952d1cd7bb3b93d39c497f0f476b68021c59`
- client implementation/checked commit: `52c37f9ce50d1f184656d5b4469c007ec8a5791a`
- exact-check targets: `target-review-t_c48f1d6a-r2` and `vendor/fr-client-rust/target-review-t_c48f1d6a-r2`
- Cargo mode: `--locked --offline`
- LIVE: not run

Exact-export verification:

- `cargo test --locked --offline -p script --lib -- --test-threads=1`: 83 passed.
- `cargo test --locked --offline -p script --test native_npc_box -- --test-threads=1`: 5 passed.
- `cargo test --locked --offline -p script --test host_js -- --test-threads=1`: 2 passed, 1 ignored regeneration helper.
- `cargo test --locked --offline -p host-play --lib -- --test-threads=1`: 146 passed.
- Client `cargo test --locked --offline -p client --test overlays --test npc_overlay -- --test-threads=1`: 11 passed (7 overlay-equivalence, 4 NPC projection).
- Strict affected-target Clippy with `--no-deps -- -D warnings`: passed for script, host-play, and client.
- Root and client `cargo fmt --all -- --check`: passed.
- Scoped root and client `git diff --check`: passed.

The exact root archive was materialized at `e05ecafb806b5a0af0b24bf4819ec8a919e5de28dc`; its nested client source was replaced with the exact `52c37f9ce50d1f184656d5b4469c007ec8a5791a` archive before checks. Later root commits `bbe1742cd8c31e33edb4b95d6dd3babf9610d705`, `3bf3644ad5ed06ef7cbf52c87307e07885265526`, and `29bed3b73055e376b7f27f2aa87ec6d7e781b4d7` are unrelated fixture, TaskBot-validation, and qualification documentation work. The task-owned paths had no delta from the checked root at the recorded post-check HEAD.

Frozen-source SHA-256:

- both revisions' `ClientAdapter.ts`: `4e17962b60e126e33a34e56e6155b2813227f4955787bbfb55204325f61b1425`
- both revisions' `FireGiant.ts`: `b7463f7f4f81088270fbdf382aeb6da9eee58aa9ce64678382cd02b4f04e3f21`

Coverage includes exact eight-point native geometry and winding, existing-renderer projection equivalence, absent/unready/non-positive/behind-camera/scene-state rejection, bounded active-list extraction, lazy snapshot-demand gating, real FlatBuffer/isolate/adapter reads, fresh point objects, unchanged-delta retention, despawn replacement, unavailable clearing, old-buffer compatibility, and per-isolate separation.

Limitations:

- No LIVE or catalog acceptance claim.
- The current box is optional frame geometry, not visibility/occlusion policy; any unprojectable corner makes the box unavailable.
- No geometry history is retained. The host allocates only the current projectable bounded active-NPC vector, and only for a consuming isolate tick edge.
- No `locBox`, `playerBox`, TaskBot, scenario, fixture, catalog, navigation, or foreign-runtime change is included.
