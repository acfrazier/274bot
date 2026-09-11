# Remove bulk observation roundtrips from native sequencing

Work in the active campaign on codex/rs2b0t-multirevision. Read AGENTS.md,
docs/execution.md and fail-closed-dispatch. Scope follows123,128 and read-only
133; read the final09b audit before implementation. Preserve native Rust phase
ownership and existing command ordering, failure results and clocks.

Root source inspection confirms bank.js forwards all snapshot locs/banks on each
poll, cake_stall.js also forwards locs/inventory, and Rust parses these arrays.
This is avoidable copying introduced by123. Implement a compact Rust-owned
observation projection fed from the existing isolate FlatBuffer snapshot update.
Keep delta semantics correct: omitted fields retain the last native value,
explicit empty collections clear it, logout/reset/script replacement clear stale
facts, pause/hold retains the documented deadlines. Select only relevant bank /
cake facts in Rust with stable existing ordering; do not cache/copy the whole
world or move candidate selection into JavaScript. Prefer existing borrowed
readers and bounded native facts over duplicated snapshots. JS may send options,
tokens and caller-owned callback results, not bulk world/inventory arrays.
Inspect autocast too and remove any equivalent bulk observation transfer within
this same bounded seam. Do not rewrite unrelated periodic/death controllers.

Own crates/script/src/load.rs, bank_open.rs, cake_stall.rs, autocast.rs and the
corresponding three shim files/tests. No fixture, TUI, API, client, opcode or
navigation-policy changes. Preserve60s/5s bank and2400ms cake/3s autocast bounds.
Do not change arrival predicates based on speculative timing: if133 identifies
a separate navigation defect, report it for root scope decision. Current four
Bone runs pass after a timeout/retry; diagnostic concurrent timings are not a
performance baseline.

Use exact committed source export and focused meaningful tests for snapshot
projection/delta/reset, stable candidate identity and callback ordering, and
existing lifecycle/sequence regression suites. Include deterministic evidence
that repeated native polls do not deserialize/copy loc/inventory arrays. Do not
claim measured runtime savings without a matched measurement. No LIVE here;
root runs the controlled requalification after review. Formatting and affected
strict Clippy are required. Report09c-native-sequencing-observation-projection.md
with evidence under native-sequencing-observation-projection. Commit only named
owned paths, request same-card reviewer, then stop.
