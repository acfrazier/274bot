//! Shared helpers for `script` integration tests.
//!
//! Intentional fixture variants (`base_snapshot`, seeds, `scene_state`, …) stay
//! in each test file; only byte-identical scaffolding is centralized here.

use script::isolate_fb::SnapshotInput;
use script::LoadIsolate;

pub fn post_snapshot_input(iso: &LoadIsolate, input: &SnapshotInput<'_>) {
    iso.post_snapshot(script::isolate_fb::encode_snapshot(input));
}
