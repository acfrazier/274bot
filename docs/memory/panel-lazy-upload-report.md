# Panel lazy CPU upload ownership

## Scope

`crates/panel/src/game_view.rs` now keeps a small initialized 1×1 placeholder for a newly-created `GameView`. The 765×503 panel-owned texture and RGBA staging buffer are allocated only when a `PixMap` frame is actually presented. A GPU-first `Texture` view therefore retains only the client texture registration and does not allocate dormant CPU upload storage or a dormant applet-sized panel texture.

The panel callers in `app.rs` remain unchanged. Client texture ownership, `FrameOutput` transport, mailbox consumption, registration cadence, and render timing are unchanged. CPU-first frames reuse the lazy owner and staging buffer; GPU-to-CPU and CPU-to-GPU transitions preserve the owned CPU texture once it has been used. GPU steady-state presents continue to avoid registration churn.

## Verification

- `cargo test -p panel game_view`
- Result: 8 passed, 0 failed (including the existing direct GPU bind, stable texture identity, disposal, readback, pixel packing, and RGBA tests).
- Added assertions prove GPU-first initialization and first GPU bind have no CPU owner, while the CPU fallback allocates the owner on demand.
- Disposal still unregisters exactly once through the consuming `dispose` path; the direct-bind test verifies replacement unregistration and the second `unbind_client` no-op.

Warnings from unrelated existing `host` and `script` code remain; no warnings were introduced in the panel change.

## Allocation accounting and remaining proof

The removed nominal CPU payload per qualifying view is `765 × 503 × 4 = 1,539,180` bytes (about 1.468 MiB). The previous applet-sized nominal GPU texture is also avoided until CPU fallback is needed. The number of qualifying owners is the number of actual GPU-only `GameView` instances (pane/rail/grid views), not the number of bots. These are allocation-domain accounting figures, not an RSS guarantee.

No native RSS, driver-residency, visual-capture, or Windows proof was run in this bounded task. Root still needs the approved native focused-one versus focused-plus-background evidence and capable screenshot inspection before making any residency or performance claim.
