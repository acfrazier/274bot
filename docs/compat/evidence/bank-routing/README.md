# Bank-routing deterministic evidence

Commands run from the campaign worktree on 2026-09-10:

- `cargo test -p script --test load_isolate`
  - 158 passed, 0 failed (`script-bank-loop.txt`).
- `cargo test -p host-play --lib`
  - 125 passed, 0 failed (`host-bank-loop.txt`).
- `RS2B0T=<100adccc capture> cargo test -p script --test load_isolate real_bone_burier_ -- --nocapture`
  - both exact-source Bone Burier tests passed.
- `RS2B0T=<8e7d965b capture> cargo test -p script --test load_isolate real_bone_burier_ -- --nocapture`
  - both exact-source Bone Burier tests passed (`captured-bone-burier.txt`).
- `cargo test -p script --lib`
  - 44 passed, 0 failed (reported in the task handoff; bank pending lifecycle regression included).
- `cargo fmt --all -- --check`
  - passed.
- `git diff --check`
  - passed.
- `cargo clippy -p script -p host-play --all-targets --all-features -- -D warnings`
  - passed (`quality.txt`; the format/diff checks are silent on success).

The live server matrix is intentionally not claimed here; it remains assigned to the final live task.
