# Record the actual native panel adapter for preservation proofs

Use grok46 profile defaults on codex/rs2b0t-multirevision. Read applicable
instructions and docs/execution.md. This is a bounded step8 diagnostic wiring
change, not implementation of the separately proposed harness architecture.
No agents, LIVE, source exports/archives, client changes, remotes or gitlink.

Current native receipts establish rendered frames and gameplay, but do not
record the actual panel adapter. crates/panel/src/window.rs creates the real
surface-compatible adapter in its production initialization around line272,
requests the device, and injects that same device into the client. Emit its
actual AdapterInfo once when BOT_DEBUG=1, after successful device creation.
Use the existing debug gate and a clear single diagnostic line with name,
backend and device type (vendor/device IDs if useful). Do not infer these from
BOT_CPU, platform or environment. Do not log failed candidate adapter attempts
as a successfully selected device. Keep the default non-debug output unchanged.

Own only the small diagnostic hunk in crates/panel/src/window.rs plus a short
note in docs/compat/06j-native-renderer-diagnostics.md. Preserve device selection,
limits, lifecycle, sharing, CPU fallback, frame output, last-FBO freeze, surface
configuration and all allocation behavior. Do not touch client/world/API files
owned by the static-loc task. Existing GameView PresentStats and resource-pane
debug output distinguish actual Texture versus PixMap frames; root will use
those and scene2 captures for rendering qualification. An adapter line by
itself is not proof that client 3D used GPU, nor a performance result.

This mechanical diagnostic does not need a new test or separate review card.
Inspect the exact scoped diff, run rustfmt on the owned file, and a syntax/build
check only if necessary. Do not start a broad compilation or add tests that
mirror the log. Commit only owned paths and complete with source identity,
what was checked and the limits. Root owns native LIVE and final branch review.
