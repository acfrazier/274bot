# Await asynchronous TaskBot validation

The TaskBot compatibility prelude now awaits each task's `validate()` result before selecting that task. This is the minimal promise-result correction: synchronous booleans retain their existing behavior, asynchronous false continues to the next task in priority order, asynchronous true selects one task, execution remains awaited, and a rejected validation reaches the existing compatibility-runner error path.

The defect was reproduced against the unmodified source at `ead4d5f5d57f4018b8857880c3daf8c2ddd1990b`: an async validation resolving false was treated as truthy and executed the wrong first task. The isolated regression failed with `left: ["wrong"]`, `right: ["right"]` before the prelude correction. Raw output is in `evidence/async-task-validation/red-async-false.log`.

The new isolated-runtime regression file exercises the real `LoadIsolate` ABI rather than matching prelude text. Its five tests prove:

- an async false task neither executes nor starves a later async true task;
- task selection remains parked on an earlier pending validation and resumes in priority order only after it settles;
- validation rejection is logged and does not execute the rejected task;
- Pause freezes a validation parked on `Execution.delayTicks`, and Resume settles it before execution;
- Stop destroys a slot with a parked validation and leaves no late gameplay request to forward.

The failing reproduction used committed host base `ead4d5f5d57f4018b8857880c3daf8c2ddd1990b`. Passing verification used exact committed host base `e05ecafb806b5af7f1d729b6f9c23c780956c118` plus only this card's scoped prelude and test overlay, with client `aef3952d1cd7bb3b93d39c497f0f476b68021c59`. The exports are `.check-t_620a63ab-red-aef` and `.check-t_620a63ab-final-e05-aef`; the exclusive compiler cache is `target-t_620a63ab`.

- `cargo fmt -p script -- --check`: passed.
- `cargo test -p script --test task_validation`: 5 passed, 0 failed.
- `cargo test -p script`: 524 passed, 0 failed, 3 ignored.
- `cargo clippy -p script --tests -- -D warnings`: passed.
- Scoped `git diff --check`: passed.

Raw logs and source identities are under `docs/compat/evidence/async-task-validation/`. No `load.rs`, `load_isolate.rs`, client, scenario, catalog, LIVE, support-matrix, or campaign-state changes were made.

The scoped commit is based on `bbe1742cd8c31e33edb4b95d6dd3babf9610d705`. The untested delta from `e05ecafb8` to that parent changes only host-play/scenario resource-start fixtures and their report/evidence; it does not touch `crates/script`. Root owns final composed integration checks on a frozen candidate.
