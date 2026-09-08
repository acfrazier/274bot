# Windows 08:30 loading-splash audit

Status: bounded source/evidence audit. This report does not claim full G1 or G4.

## Frozen subject and lineage

The captured native subject is the frozen candidate recorded by
`docs/memory/windows-tile-boxed-native-freeze.json`:

- host `fb3589ac28583242b999ac864ea69c4ef8fa5923`
- client `fd956c91bf09e059359c8e182a33583e2c626cd3`
- staged executable `C:/ProgramData/274bot-Test/tile-boxed-fb3589a/panel-play.exe`
- executable SHA-256
  `a9b581bab780d2868035941d799d416f0ef9ec68d5eb5a4fe0a62fa29670667f`

The 08:30 capture JSON files repeat that executable path and hash. The archive
manifest reports the burst PNG/JSON files and the initial/post captures, but the
capture metadata explicitly says `sceneState: "not inferred from capture"`.

This checkout is not the frozen source tree: the current root is host
`0c1e4388daa536d0d1f7d0b0fd5cf2b496eeb80e`, and its client gitlink is the
`3456edc` lineage, whereas the frozen candidate is the sibling host commit
`fb3589a` with client `fd956c9`. The frozen client commit is available in the
`windows-render-owner-census` checkout and is descended from `3456edc`; source
behavior below is therefore cited from the exact frozen client commit rather
than inferred from the current root.

## Source-grounded expected behavior

At frozen client `fd956c9`:

- `crates/client/src/client/client.rs:5756-5783` handles
  `ServerProt::REBUILD_NORMAL`, sets `scene_state = 1`, records the load start,
  and leaves the loading splash to the renderer.
- `crates/client/src/client/client.rs:9197-9217` is the simulation half of
  `check_minimap`. While `scene_state == 1`, it continues checking scene data;
  on success `check_scene` sets `scene_state = 2` at `:9269-9272`.
- `crates/client/src/render/draw.rs:3930-3955` implements
  `scene_loading_splash`. It does not clear `area_game`; it draws
  `Loading - please wait.` with `media.p12` when that font exists, then blits
  the retained `area_game` at `(4, 4)` when `client.draw` is true.
- `crates/client/src/render/draw.rs:3965-3968` invokes that splash on every
  draw-only `check_minimap` pass while `scene_state == 1`.
- `crates/client/src/render/media.rs:194-210` loads `title` from the fixture's
  cache directory and depacks `p12_full`. A missing/unreadable title jag or
  missing `p12_full` leaves `media.p12` as `None`; the source intentionally
  skips the text in that case while still blitting the retained game pixels.
- `crates/client/src/render/backend/gpu.rs:1191-1203` avoids rebuilding the
  GPU scene mesh during Game `scene_state == 1`, retains the last scene texture,
  and still draws scene overlays. At `:1329-1335`, the GPU path composites the
  `area_game` pixels over that retained scene during the freeze. The predicate
  at `:1612-1619` is exactly Game plus `scene_state == 1`.

The source expectation for a valid GPU freeze is therefore: last 3D scene held,
loading text present only if `p12` is available, minimap/other retained layers
not blanked, and eligible overlays still composited. It is not an expectation
that every scene-1 frame must contain visible text when font availability is
unknown.

## What the 08:30 evidence establishes

The directly inspected root captures were:

- frame 32: scene2 Lumbridge baseline;
- frames 33 and 34: labeled synchronized rebuild burst frames, visually still
  retaining the Lumbridge 3D view, minimap, and chat/chrome;
- frame 35: scene2 Varrock result;
- after-varrock: post-action capture.

No loading splash text was visible in frames 33 or 34. The paired frame JSON
records the exact capture timestamps and executable hash, but does not record
`scene_state`, renderer state, font inventory, or a scene-transition event.
Frames 33 and 34 are identical by PNG hash, which confirms retained pixels for
those two samples but does not identify whether they were captured during the
`scene_state == 1` interval. The root observations therefore support a
qualitative held-scene/retention observation only; they do not prove the G1
ordered `scene2 -> scene1 -> freeze capture -> scene2` sequence.

The 08:30 archive contains no p12/font/cache-provenance field. Its capture
records and archive manifest do not establish that `title`/`p12_full` was
present in the frozen fixture, nor that it was absent. Earlier visual reports
also record scene2 modal/minimap/chrome observations but do not record a p12
availability check. Thus the absent text is not, on this evidence, a confirmed
splash defect.

The retained Lumbridge view is consistent with the frozen GPU source path, but
because capture metadata says scene state was not inferred, this run cannot
separate a true scene1 FBO freeze from a scene2 frame captured before or after
the rebuild. It is missing evidence, not an established implementation bug.

## Bounded next step

Do not patch the splash from this result. Run one diagnostic-only repeat on the
same frozen binary with an observable scene transition and readable capture:

1. prove the staged hash and record the fixture/cache provenance, including
   whether `title` contains `p12_full` (or explicitly record it unavailable);
2. arm capture before the rebuild and correlate the first capture to a logged
   `scene_state == 1` transition, then capture after `scene_state == 2`;
3. inspect the ordered live/freeze/post PNGs for held 3D, minimap continuity,
   and splash text under the recorded p12 condition.

If p12 is proven available and a scene1-correlated freeze PNG still lacks
`Loading - please wait.`, then the next fix/audit target is the frozen GPU
`scene_loading_splash`/`composite_scene` path, not the tile-boxing change. If
scene1 ordering or p12 provenance remains unavailable, classify the result as
`inconclusive`/`blocked-missing-capability` rather than changing source.

## Scope limits

This report does not claim G1 or G4 passed, does not claim a modal/IF freeze
proof, and does not claim performance acceptance. It records only the observed
qualitative retention and the source conditions needed for a defensible splash
verdict.
