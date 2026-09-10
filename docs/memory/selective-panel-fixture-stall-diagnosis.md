# Native panel fixture stall: bounded diagnosis

Inspected selected host `9527cc63` directly with `git show`, compared with
main baseline `54cfcf8`. No integration source, production behavior, process,
server, remote or test fixture was modified by this diagnosis.

## Finding

`session::tests::flat_model_spawns_every_member_as_a_client` is a preexisting
non-hermetic test: its slot workers run real client initialization and therefore
need the existing local cache HTTP service to avoid lengthy retry waits.
Its concluding comment incorrectly says the workers detach at process exit.
Both the test function and the relevant Session/Play Drop implementations are
byte-for-byte identical between main and the selected source.

Selected source locations:

- `crates/panel/src/session.rs:5450`: test starts real Play slots for alice,
  bob and carol, waits up to two seconds for three status rows, asserts three
  per-member IO entries, three Play status rows and a real carol control arm,
  then checks that focus changes preserve all three slots.
- `crates/panel/src/session.rs:5493`: stale comment promises detached workers.
- `crates/panel/src/session.rs:3403`: Session Drop sets `self.play = None`
  before releasing the picker's navigation pack.
- `crates/host-play/src/lib.rs:3492`: Play Drop stops and joins every thread.
- `crates/host-play/src/lib.rs:3629`: workers publish their initial status rows
  before real `prepare_client` and `maininit`. Thus the member-count assertions
  can finish while workers are still inside initialization.
- `crates/host-play/src/lib.rs:3659`: the short HTTP retry wait exists only
  under this crate's own `#[cfg(test)]`; Cargo does not apply that cfg to
  host-play when panel's unit tests use host-play as a dependency.
- `crates/host-play/src/lib.rs:3675`: real client initialization precedes the
  main loop's stop check, so Session Drop can wait for initialization retry
  behavior before the worker observes Stop.

Baseline provenance: commit `5824cdced` on August 30, 2026 introduced both
joining Play Drop and Session's explicit Play release before picker teardown.
That predates the approved selective extraction base. The obsolete detachment
comment belongs to the older test. This observation does not implicate Windows
socket wake behavior: the identified wait occurs before the normal parked
client-loop Stop path.

## Decision

Do not copy TUI's spawn suppression into this test. The TUI preparation tests
assert staged catalog/configuration behavior without workers; this panel test
asserts real Play membership, control arms and stable member counts. Suppressing
spawns would make its existing status/arm assertions fail. Replacing them with
synthetic counters would weaken the behavior being tested.

Per root's decision, no patch is proposed. Recheck this native panel suite once
with the existing local service running, matching the macOS combined-suite
condition, and record the outcome separately. If it remains blocked, report that
bounded result. Do not add a mock service, alter production retry/Stop timing,
or describe server-dependent passing tests as hermetic unit coverage. Several
nearby tests explicitly refer to the same obsolete detachment comment, so the
limitation applies to this fixture family rather than only its first observed
stall.
