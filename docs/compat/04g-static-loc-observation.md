# Refresh native static-loc observations without invalidating render caches

Task `t_192feefa` on host `codex/rs2b0t-multirevision` and client
`codex/bothost-274-289`. Isolated checks used root's Git-blob export
`.superpowers/review-exports/static-loc-root-7d1d4386` (2474 files,
manifest sibling `static-loc-root-7d1d4386-manifest.json`) at host
`7d1d43869fc4d9a1ca00043345ac219b0c7a8111` and client
`9d090ed04957e4efc254f073cda97bc5510ca72b`. `CARGO_TARGET_DIR` was the
previously absent
`.superpowers/review-exports/static-loc-root-7d1d4386-target`. Not LIVE.
Do not claim the Gnome radius-8 pipe/log failure is fixed.

## Diagnosis

`LOC_DEL` / `LOC_ADD_CHANGE` packet reception bumps `gens.scene`. The
deferred `loc_change_do_queue` apply does not. Consuming that packet gen
before native `del_loc` / `add_scenery` left `Snapshot::rebuild_loc`
clean: static scenery does not bump tile `model_stamp` (doing so
recreated unlit walls on moving actors). Cached `LocView.distance` was
also only written on loc rebuild, so a local-player tile change published
the old Chebyshev scalar.

## Seam

Client `World` exposes `static_loc_generation`. It wrapping-adds on
successful `add_scenery`, on `del_loc` that actually removes a ground
sprite, and on `reset_map`. Failed/out-of-range/no-op add, missing-tile
delete, and dynamic sprite add/release do not touch it. Render
`model_stamp` is unchanged.

API `rebuild_loc` dirties on scene gen, aggregated tile stamp, or that
static generation. Player-family rebuild rewrites only `LocView.distance`
when `local_world_tile` moves; loc vec allocation, names, and actions
stay put. Same-tile player ticks are a no-op for those scalars.

Walls/decor keep stamp-based invalidation. No JS cache, no per-read
scenery sweep, no world clone.

## Proof (isolated empty target)

Raw logs: `docs/compat/evidence/static-loc-observation/`. Owned export
bytes matched git blobs `f2f25adf1` / `b43c4f101` / `e94739fa9` and the
manifest sha256s. Restrictions that blocked tar extract, `python3 -c`,
and heredoc are in `restrictions.txt`; those paths were not retried.

- Client lib `static_loc_observation`: add/remove bump generation without
  tile stamps; no-op/failed add do not bump; multi-tile add/del bump
  once; dynamic churn leaves generation and stamps; `reset_map` bumps.
  5 passed.
- Client lib `sprite_reuse`: dynamic slots stay bounded. 1 passed.
- API `--test static_loc_observation`: after consuming packet scene gen,
  native delete removes flax and native add restores it; missing-tile
  delete and dynamic churn do not rebuild or clone names; player tile
  change refreshes distance 13→0 without a loc resweep. 3 passed.
- API `--test snapshot loc_view`: existing wall/stamp/four-layer/default
  loc tests. 4 passed.
- `rustfmt --edition 2021 --check` on the three owned sources: empty
  output, exit 0.
- `cargo clippy --locked -p client --lib --tests -- -D warnings` and
  `-p api --all-targets -- -D warnings`: finish with no diagnostics.

## Out of scope

LIVE requalification, host gitlink/remotes, DeathRecovery, Gnome
radius-8 / foreign pipe resync, render ownership, and last-FBO freeze
remain root or other cards. Reviewer must inspect client `9d090ed`
directly; root updates the host gitlink after approval.
