# Combat startup helper mappings

Implemented brief 115 over the posted isolate facts. `rangeLoadoutOf` now preserves empty/non-dart inputs and recognizes only content item rows whose object alias ends in `_dart`; `foodForms`, `isFoodItem`, and `foodCount` project generated item aliases and count inventory slots; `SettingsStore.displayString` and `saved` stringify the posted settings bag without coercing values or changing `resolve`.

Unsupported behavior remains explicit: `rangeSupplyEmpty`, `eatAtHpThreshold`, `SettingsStore.save`, `SettingsStore.globalBag`, and irregular pizza/pie/chocolate aliases remain unimplemented. No inventory or equipment fallback, foreign tables, restock policy, or runtime host callback was added.

Verification:

- `cargo test --locked --offline -p script --test combat_start_helpers -- --test-threads=1` — 3 passed.
- `cargo test --locked --offline -p script --test load_isolate eat_predicates -- --test-threads=1` — passed.
- `cargo test --locked --offline -p script --test load_isolate isolate_settings_store_does_not_return_schema_defaults -- --test-threads=1` — passed.
- `cargo clippy --locked --offline --no-deps -p script --lib -- -D warnings` — passed.
- `rustfmt --edition 2021 --check` on owned Rust — passed.

No LIVE run was performed, as required by the brief.
