# Responsive panel startup preparation

Task: `t_779f58e5`
Branch: `codex/rs2b0t-multirevision`
Implementation: `814e5293fec426b722e450580c514b9cffca826f`
Design: [06c-panel-startup-design.md](06c-panel-startup-design.md)
Verification: [evidence/panel-startup-preparation/verification.md](evidence/panel-startup-preparation/verification.md)

## Result

Panel profile startup now preserves the existing three resource-identity checks without running their large cache/navigation reads on the winit/imgui event thread. After the first blank panel frame presents, the UI captures the selected profile options, environment, and saved revision, then launches one detached preparation worker. That worker resolves and binds the immutable `ServerProfile`, performs template-load validation, and decodes the shared cache/interfaces/navigation template. The UI polls a bounded channel, installs only a matching-generation result, and keeps painting a plain preparation status.

Unlock and live boots stay pending after template installation. A second detached worker performs the required final disk-resource validation immediately before Play construction. Success yields a one-use `ValidatedTemplate` whose field is private and whose value is consumed by `run_prepared_template` on the UI thread. `Play`, vault mutation, slot threads, scenario setup, JS stores, picker globals, and GPU ownership remain on their prior thread. The ordinary `run_with_template` API still performs final validation itself, so TUI/CLI and direct checked callers retain the existing refusal behavior.

## Lifecycle and error behavior

`configure_profile` and pre-bind revision changes advance a generation. A completion from an older generation is dropped before profile/template installation. Preparation failures preserve the underlying profile/resource error and leave profile binding, vault, Play, and slots absent. Final validation failure creates no vault, Play, scenario, or slot. Interactive Unlock failure remains non-fatal and returns to the panel prompt; live/smoke preparation failure keeps the existing `FAIL:` and exit-1 policy.

An Unlock click is queued instead of synchronously binding. Repeated Unlock input can replace the pending passphrase without logging it. Deferred BOT_VAULT_PASS and live modes use the same preparation path. Closing drops channel receivers and never joins either hashing worker on the event-loop thread. The first-present GPU ordering is unchanged: no Play or slot can be assembled before `inject_device` and the first frame.

## Validation contract

`SharedClientTemplate::validate_for_play` validates the selected cache JAGs, nav pack, and optional nav flags against the hashes frozen in its own immutable profile. It is the only constructor for `ValidatedTemplate`. The ticket exposes the template only by reference for same-Arc verification, cannot be cloned or constructed by callers, and is moved into Play construction. This makes bypassing final validation through the prepared API impossible while avoiding a fourth UI-thread hash.

The validation-to-construction interval retains the same bounded filesystem TOCTOU class as the former synchronous checked function. The implementation does not use path/mtime shortcuts, does not change `hash_file`, and does not claim to make mutable disk resources atomic.

## Scope

Changed source is limited to:

- `crates/host-play/src/profile.rs`: bind/load preparation helper.
- `crates/host-play/src/lib.rs`: consuming validation ticket and prepared Play constructor.
- `crates/panel/src/session.rs`: captured preparation inputs, generation-safe installation, queued Unlock, and prepared-ticket consumption.
- `crates/panel/src/app.rs`: post-first-present worker launch/polling, redraw/status, and deferred boot continuation.
- Focused tests in the existing host-play and panel files.

No client, nav hashing/format, TUI event loop, banking/script dispatch, login timeout, external fixture, operator setting, gitlink, or runtime dependency changed.

## Verification and remaining proof

The affected host-play integration suite (11), host-play library suite (125), and panel library suite (389) pass. Formatting, scoped diff checks, strict host-play Clippy, and panel Clippy apart from four unchanged pre-existing type-complexity signatures pass. Exact commands and source hashes are recorded in the verification receipt.

Native Mac and Windows responsiveness plus changed-resource refusal on disposable full-size resources remain root-owned. The operator's Windows diagnostic at frozen `584d05bc` predates this implementation and is not evidence for or against `814e5293`. No native or LIVE acceptance is claimed here.
