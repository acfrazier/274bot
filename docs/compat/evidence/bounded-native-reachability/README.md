# Bounded native reachability evidence

Task: `t_25797d04`

Source provenance:

- implementation base: `f0249f96283c4c4540b7fe308b46972fc24516dc`
- implementation commit: `bebd2fe8b056b3680f240d5c1bfd66490cb5cf14`
- client gitlink: `aef3952d1cd7bb3b93d39c497f0f476b68021c59`
- brief SHA-256: `63778485b398d45ea09ac3b6cc6d44d07533e3b95c894bc5bc46468695b6e001`
- target: `.superpowers/task-targets/bounded-reachability-t_25797d04`
- Cargo mode: `--locked --offline`
- LIVE: not run

Verification results:

- `cargo test --locked --offline -p api -- --test-threads=1`: 206 passed.
- `cargo test --locked --offline -p script --lib -- --test-threads=1`: 83 passed.
- `cargo test --locked --offline -p script --test load_isolate reachability_ -- --test-threads=1`: 7 passed.
- Final independent lifecycle assertion: `cargo test --locked --offline -p script --test load_isolate isolate_reachability_unavailable_and_omitted_delta -- --exact --test-threads=1`: 1 passed.
- `cargo test --locked --offline -p script --test client_adapter_local_walk -- --test-threads=1`: 8 passed.
- `cargo test --locked --offline -p script --test host_js -- --test-threads=1`: 2 passed, 1 ignored regeneration helper.
- `cargo test --locked --offline -p host-play --lib -- --test-threads=1`: 144 passed.
- Strict affected-target Clippy with `-D warnings`: passed for API, script, and host-play.
- Scoped `rustfmt --check`: passed.
- Scoped `git diff --check`: passed.

The API regression directly compares the packed O(1) answer with `SceneQuery::can_reach` across open-grid, corridor, collision-pocket, and wall-constrained adjacency fixtures and budgets 0, 1, 2, 64, 400, 20,000, plus omitted/default 400. Script tests cover missing-rank fail-closed behavior, coordinate and entity-colocated budget uniformity, full/delta/clear/replacement lifecycle, and synchronous reads with no interaction command.

Logical 104 by 104 vector accounting, computed from the actual packed layout:

- 10,816 tiles;
- 4,056 bytes for three 338-word `u32` bitsets;
- 10,816 bytes for step masks;
- 43,264 bytes for two 10,816-entry `u16` maps;
- 58,136-byte logical derived view total, versus 14,872 bytes before this change.

This is structural byte accounting only. No RSS, heap, serialized-payload, latency, or savings claim is made.
