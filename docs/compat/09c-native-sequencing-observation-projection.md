# Native sequencing observation projection

Status: implemented

Implementation commit: `0e7ba119b86d74cc04818c3f621d7835ddb4dbb4`
Client submodule commit: `aef3952d1cd7bb3b93d39c497f0f476b68021c59`

## Result

Bank-open, cake-stall, and autocast sequencing now consume compact Rust-owned observations projected directly from each decoded `SnapshotReader`. Their JavaScript shims still carry frozen API options, sequence tokens, and callback results, but no longer rebuild or send snapshot location, bank, inventory, side-tab, or varp observations on each native poll.

The projection hook runs once for every successfully decoded snapshot before JavaScript tick dispatch (`crates/script/src/load.rs:3144`). Autocast additionally records the selected revision's magic varp during runtime wiring (`crates/script/src/load.rs:1798`).

## Projected state

### Bank-open

`crates/script/src/bank_open.rs:53` keeps only the facts used by native bank sequencing:

- login state;
- local tile;
- bank-group presence;
- the observed nearest booth;
- compact bank-location candidates containing identity, tile, distance, reachability, and actions.

The runtime selects candidates with the existing ordered matching rules and retains the selected entity key across the walk/open boundary. A changed or missing entity therefore cannot be replaced silently by another location after walking.

### Cake-stall

`crates/script/src/cake_stall.rs:36` keeps:

- login state and snapshot tick;
- local tile;
- one baker-stall target selected by `api::cake_stall::select_baker_stall`;
- the stall-food count derived by `api::cake_stall::counts_as_stall_food`.

The JavaScript shim now sends only selected-fact validity, options, and caller callback results. It no longer sends the location or inventory collections.

### Autocast

`crates/script/src/autocast.rs:37` keeps the three values needed by native arm sequencing:

- active side tab;
- attached combat-tab root;
- the selected revision's magic varp value.

The autocast shim no longer reads those values through `ClientAdapter` for every `armed`, `staffTabAttached`, `begin`, or `next` poll.

## Full, delta, and lifecycle semantics

Projection uses FlatBuffer field-presence checks. On a keyframe, present fields initialize native state. On a delta, omitted fields retain their previous value; a present empty vector replaces the previous vector-derived state with empty/default state. A logout snapshot clears all projected world facts. Native reset clears the observations as part of the existing reset fan-out, and script replacement receives a fresh isolate thread-local projection.

The dispatch path remains fail-closed. Missing observations do not synthesize a target, bank presence, inventory contents, attached staff tab, or armed varp. Malformed JavaScript inputs still fail through the existing native error results, but snapshot arrays are no longer accepted from JavaScript as alternate evidence.

## Allocation and boundary effect

Before this change, bank and cake sequence polls mapped materialized JavaScript arrays into new objects and then deserialized equivalent `serde_json::Value` arrays in Rust. Autocast polled the materialized JavaScript snapshot for its three controls. Those operations repeated while a native sequence waited.

After this change, changed snapshot fields are inspected once at the native decode boundary. Sequence polls read thread-local compact state. Unchanged delta fields do no projection work, so repeated polls neither traverse JavaScript snapshot collections nor deserialize equivalent JSON arrays. Bank retains compact candidate rows only; cake retains one target plus an integer; autocast retains three integers and login state. No whole snapshot or full world model is retained.

## Regression coverage

The focused integration suites now prove:

- JavaScript snapshot getters can be replaced with throwing accessors while bank, cake-stall, and autocast native sequencing still completes (`named_bank_approach.rs:245`, `cake_stall_native_mapping.rs:254`, `autocast.rs:227`);
- selected bank identity survives an omitted `locs` delta and is revalidated before opening (`named_bank_approach.rs:545`);
- a reset clears stale bank observation state (`named_bank_approach.rs:601`);
- cake food progress works across a real delta with unchanged locations omitted (`cake_stall_native_mapping.rs:431`);
- autocast's three-step sequence works across deltas that alternately omit unchanged varps and side-tab interfaces (`autocast.rs:257`);
- existing missing/replaced target, depleted/wrong-op, pause/hold, timeout, selected-revision, and fail-closed tests remain green.

Exact-export commands and results are recorded in `docs/compat/evidence/native-sequencing-observation-projection/verification.json` and `checks.log`.
