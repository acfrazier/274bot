# Native panel adapter diagnostic

Task: `t_d1671c3c`
Brief: `docs/compat/briefs/72-native-adapter-diagnostic.md`
Branch: `codex/rs2b0t-multirevision`

## Wiring

After the production panel window creates a surface-compatible adapter and
successfully requests a device (`crates/panel/src/window.rs`), `BOT_DEBUG=1`
prints one stderr line with that adapter's actual `AdapterInfo`: name, backend,
device type, and vendor/device IDs. The values come from the selected wgpu
adapter, not from `BOT_CPU`, platform, or environment. Failed adapter or device
attempts are not logged as a selected device. Default non-debug output is
unchanged.

Device selection, limits, lifecycle, sharing, CPU fallback, frame output,
last-FBO freeze, surface configuration, and allocation are unchanged.

## Limits

This line records the panel's selected adapter. It is not proof that client 3D
used GPU, and it is not a performance result. Existing GameView PresentStats
(`pixmap` vs `tex`) and resource-pane debug output distinguish Texture versus
PixMap frames; scene2 captures remain required for rendering qualification.
Root owns native LIVE and final branch review.
