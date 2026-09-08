# Windows 08:30 loading-splash audit

Status: bounded source/evidence audit. This report does not claim full G1 or G4.

## Frozen subject and lineage

The captured native subject is the candidate recorded by
`docs/memory/windows-tile-boxed-native-freeze.json`:

- host `fb3589ac28583242b999ac864ea69c4ef8fa5923`
- client `fd956c91bf09e059359c8e182a33583e2c626cd3`
- staged executable `C:/ProgramData/274bot-Test/tile-boxed-fb3589a/panel-play.exe`
- executable SHA-256
  `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`

The 08:30 capture records repeat that path and hash. The current root snapshot
used for the audit is host
`0c4b1cdebfb367104785ebc47ffcbbaa8791357d` (`0c4b1cd`) with client gitlink
`3456edc`. The
frozen host commit is the sibling `windows-render-owner-census` checkout, whose
client is exactly `fd956c9`; `git merge-base --is-ancestor 3456edc fd956c9`
passes there. Source behavior below is therefore cited from the frozen client,
not assumed from the current root.

## Source-grounded expected behavior

At frozen client `fd956c9`:

- `crates/client/src/client/client.rs:5756-5783` handles
  `ServerProt::REBUILD_NORMAL`, sets `scene_state = 1`, records load start, and
  leaves the loading splash to the renderer.
- `crates/client/src/client/client.rs:9197-9217` is the simulation half of
  `check_minimap`; successful `check_scene` sets `scene_state = 2` at
  `:9269-9272`.
- `crates/client/src/render/draw.rs:3930-3955` implements
  `scene_loading_splash`: it leaves `area_game` uncleared, draws
  `Loading - please wait.` with `media.p12` when present, and blits retained
  `area_game` at `(4, 4)` when `client.draw` is true.
- `crates/client/src/render/draw.rs:3965-3968` invokes it on draw-only
  `check_minimap` passes while `scene_state == 1`.
- `crates/client/src/render/media.rs:194-210` loads `title` and depacks
  `p12_full`. Missing/unreadable data leaves `media.p12` as `None`, intentionally
  suppressing the text while retained game pixels still blit.
- `crates/client/src/render/backend/gpu.rs:1191-1203` avoids rebuilding the GPU
  scene mesh and retains the last scene texture during Game `scene_state == 1`.
  `:1329-1335` composites `area_game` over that texture; `:1612-1619` defines
  the Game-plus-scene1 freeze predicate.

Thus valid GPU freeze behavior is: last 3D scene held, minimap and retained
layers not blanked, eligible overlays composited, and loading text only when
runtime `p12` is available. Text is not required when only font availability is
unknown.

The operator also identified a concrete source discriminator for the bounded
follow-up: frozen `draw.rs:4222` calls `check_minimap`/`scene_loading_splash`
before `game_draw`, while frozen `gpu.rs:1068-1076` draws scene overlays, clears
`overlay_coverage`, and installs coverage tracking for later overlays.
The audit does not claim this ordering is a bug; it identifies the call/coverage
boundary to inspect, including whether the splash is re-run and represented in the
final atlas coverage at `gpu.rs:1350-1390` / `finish`.

This coverage-ordering hypothesis is not yet a memory-candidate regression. The
public stable client at
`4f2048ea10f75b3bb92ff45610b35ba7313b0308` has the same
`overlay_coverage.fill(0)` plus guarded-later-overlays pattern and no
`scene_loading_splash` call in `gpu.rs`. Any correction decision therefore needs
a bounded baseline-versus-candidate comparison of the exact render path; this
audit does not attribute the omission to tile storage or authorize a patch.

## What the 08:30 evidence establishes

Root directly inspected the ordered captures:

- frame 32: status `scene 2`, Lumbridge at `3222,3222`;
- frames 33 and 34: status `scene 1`, destination `3213,3423`, while retaining
  the Lumbridge 3D view, minimap, chat, and panel chrome; neither shows loading
  text, and their PNG hashes are identical;
- frame 35: status `scene 2`, Varrock at `3213,3423`;
- after-varrock: changed Varrock positions, consistent with resumed updates.

The OCR selector independently reports `scene 2`, `scene 1`, `scene 1`, `scene
2` for frames 32–35 in
`diagnostics/windows-visual-proof-nullraster-20260908-0830/root-ocr-selection.jsonl`.
The capture JSON's `sceneState: "not inferred from capture"` is only a helper
field; it does not contradict the scene status visible in the PNGs. This run
therefore establishes the ordered scene2 → scene1 → scene2 transition and a
qualitative held-scene observation, but not full G1/G4, pixel equality, cadence,
or modal/IF coverage.

The read-only fixture check at
`diagnostics/windows-0830-title-provenance/receipt.json` found the exact local
cache target used by the frozen binary: `title` is 59,855 bytes with SHA-256
`b7a47dead79c241beb9c4ffbb4cb289bb3099eaf088f51e364cde40f3aa70b55`;
`p12_full.dat` is 8,956 bytes, its index is 7,521 bytes, all 256 glyphs parse,
and font height is 12. `p12AvailabilityChecksPassed` is true. The receipt also
explicitly says `runtimeMediaP12Observed: false`: this proves fixture
availability, not that the running process retained `Media.p12` at capture time.

Consequently, absent splash text in known scene1 frames is a narrowed rendering
concern, not yet an established source bug. The fixture is valid, but runtime
font state and the exact draw/composite path remain unobserved. No redundant
native repeat is justified merely to recover scene state already visible in the
PNG/status evidence.

## Bounded next step

Perform read-only/source investigation of the frozen GPU
`scene_loading_splash` → `composite_scene` → frame draw/finish path, including
the remaining runtime `Media.p12` availability gap and the draw/coverage ordering
identified above. Trace whether the splash writes to the same `area_game` pixels
that `GpuBackend::composite_scene` blits during the freeze, whether GPU finish
re-runs or records it, and whether atlas coverage can discard it. Do not patch
from this audit alone. If runtime p12 is proven present while a scene1 freeze
still omits the text, open a narrowly targeted render-path fix; otherwise
classify the result as inconclusive.

## Scope limits

This report does not claim G1 or G4 passed, does not claim modal/IF freeze proof,
and does not claim performance acceptance. Raw 08:30 archives remain preserved.
