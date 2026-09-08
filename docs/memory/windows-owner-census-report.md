# Native N16 renderer owner census

## Scope and evidence

This report analyzes only `native-render-owner-census-focused-plus-background-20260908-0150`, the qualified Intel/Vulkan N16 native diagnostic. The immutable archive SHA-256 is `a01f129182a4ff25a3f72057d26a96cec9955045978f22b5ccbfdf0f41cddee2`; the archive manifest contains 22 verified files (the tar listing has 29 entries including directories). The binding is `binding_ok=true`, `qualified=true`, `frontend=panel`, `n=16`, `workload=active`, `render_policy=focused-plus-background`, with native unchanged reader binding. The binary is the frozen diagnostic owner-census build (host source digest `a7e618...b108`, client source digest `f746e9...356f`, host build commit `66ba7c1`, client commit `9f72c2f`).

The qualification file contains exactly two owner snapshots: `observe-start` at elapsed 112.0412201 s and `observe-end` at 232.0603645 s. Owner data is only `slots[].renderer.owner_census`; periodic `samples.jsonl` renderer profiles do not carry owner rows. Private-commit periodic samples are available separately. `sampled_ms` is retained separately from renderer `updated_ms`; `build_generation` and `phase` are retained. Missing fields are not interpreted as zero.

Qualification is diagnostic-only, not performance acceptance: observation 119.0890417 s, median resident 2,369,544,192 B, CPU 0.67543998 core, client 48.7319187 ticks/slot/s, steals 5–13. The changed binary has profiles enabled, so this is not a savings comparison.

## All 16 renderer owners

Values are bytes except `shared_arc_pointees` and `overlay_materialized` (counts). Each row reports both qualification boundaries; zero deltas mean only that the two sampled boundaries matched, not that no intermediate changes occurred.

| slot | start sampled_ms | end sampled_ms | tile | nested model | unique Arc model | shared Arc | world scratch | Pix3D scratch | pixmap | minimap | overlays | mask |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 `live3b0c0_0` | 1788832139964 | 1788832259968 | 49,387,600 | 16,764,772 | 15,870,584 | 3,364 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 1 `live3b0c0_1` | 1788832139885 | 1788832260107 | 49,387,600 | 16,764,772 | 16,046,488 | 3,396 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 2 `live3b0c0_2` | 1788832139925 | 1788832259984 | 49,387,600 | 16,764,772 | 16,238,404 | 3,439 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 3 `live3b0c0_3` | 1788832139614 | 1788832259741 | 49,387,600 | 16,764,772 | 16,184,044 | 3,426 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 4 `live3b0c0_4` | 1788832139613 | 1788832259741 | 49,387,600 | 16,764,772 | 16,212,684 | 3,433 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 5 `live3b0c0_5` | 1788832139922 | 1788832260026 | 49,387,600 | 16,764,772 | 16,213,824 | 3,430 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 6 `live3b0c0_6` | 1788832139614 | 1788832259759 | 49,387,600 | 16,764,772 | 16,143,568 | 3,416 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 7 `live3b0c0_7` | 1788832139947 | 1788832260046 | 49,387,600 | 16,764,772 | 16,198,480 | 3,432 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 8 `live3b0c0_8` | 1788832139864 | 1788832259903 | 49,387,600 | 16,764,772 | 16,222,552 | 3,433 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 9 `live3b0c0_9` | 1788832139516 | 1788832259585 | 49,387,600 | 16,764,772 | 15,977,560 | 3,384 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 10 `live3b0c0_10` | 1788832139475 | 1788832259524 | 49,387,600 | 16,764,772 | 15,993,080 | 3,389 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 11 `live3b0c0_11` | 1788832139615 | 1788832259741 | 49,387,600 | 16,764,772 | 16,197,256 | 3,427 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 12 `live3b0c0_12` | 1788832140086 | 1788832260127 | 49,387,600 | 16,764,772 | 15,870,584 | 3,364 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 13 `live3b0c0_13` | 1788832139614 | 1788832259777 | 49,387,600 | 16,764,772 | 16,210,660 | 3,429 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 14 `live3b0c0_14` | 1788832139925 | 1788832259985 | 49,387,600 | 16,764,772 | 16,230,468 | 3,435 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |
| 15 `live3b0c0_15` | 1788832139882 | 1788832260005 | 49,387,600 | 16,764,772 | 16,243,948 | 3,436 | 678,080 | 3,296,712 | 2,896,668 | 1,048,576 | 866 | 31 |

The full machine-readable table retains every measured field, both boundary generations, all deltas, and per-boundary sums/maxima in `windows-owner-census-table.json`. The analysis independently checks all 22 manifest file hashes and lengths against the tar contents; all checks pass.

## Field ownership and lifetime interpretation

| field | category / purpose | lifetime / accounting |
|---|---|---|
| `tile_container_bytes` | tile containers | renderer world tile/container ownership; lifetime follows the renderer world. |
| `linked_container_bytes` | linked containers | linked container backing storage; lifetime follows linked world/container state. |
| `sprite_container_bytes` | sprite containers | sprite/cache container backing storage; lifetime follows sprite cache state. |
| `nested_model_bytes` | nested model bytes | model bytes counted through nested ownership; nested occurrence, not an extra unique allocation. Do not add to unique Arc bytes. |
| `unique_arc_model_bytes` | unique Arc model bytes | unique Arc-owned model pointees in this renderer census; aggregate identity is renderer-local. Renderer-local identity; do not infer fleet-wide deduplication. |
| `shared_arc_pointees` | shared Arc pointees | count of shared Arc pointees represented by the unique-model accounting. |
| `render_world_scratch_bytes` | render-world scratch | temporary renderer-world scratch storage; lifetime follows render/update work. |
| `pix3d_scratch_bytes` | Pix3D scratch | Pix3D temporary scratch storage; lifetime follows 3D raster work. |
| `pix3d_texel_active_bytes` | Pix3D active texels | active Pix3D texel backing storage; lifetime follows active texture use. |
| `pix3d_texel_free_bytes` | Pix3D free texels | free/reusable Pix3D texel backing storage; retained until allocator/cache release. |
| `pixmap_bytes` | pixmaps | software pixmap backing storage; lifetime follows pixmap owners. |
| `minimap_bytes` | minimap | minimap backing storage; lifetime follows minimap state. |
| `overlay_materialized` | materialized overlays | count of materialized overlay objects, not bytes. |
| `transient_mesh_bytes` | transient meshes | currently-live transient mesh bytes; short-lived render work. |
| `transient_mesh_high_water_bytes` | transient mesh high-water | observed transient mesh peak, not current residency. High-water diagnostic, not current residency. |

The field names are the frozen source instrumentation contract for host render-profile owner census at host commit `66ba7c1` / client `9f72c2f`; the archived binding records the exact source digests and the client source correction `f746e9...356f`. This checkout does not contain those frozen source objects, so no current-checkout source line is substituted for the frozen-source attribution.

## Largest owners and accounting limits

Tile containers are the largest fixed per-renderer category: 49,387,600 B per renderer and 790,201,600 B across 16. Nested model bytes are 16,764,772 B per renderer (268,236,352 B fleet); unique Arc model bytes are 15,870,584 B per renderer (258,054,184 B fleet). They are different accounting views: nested occurrences can include repeated references, while unique Arc bytes count pointees once within the renderer-local census. Adding them double-counts. The first slot is the requested check: tile 49,387,600 B, unique Arc 15,870,584 B, nested 16,764,772 B.

Other fixed fleet sums at each boundary are linked containers 2,097,152 B, sprite containers 21,889,024 B, render-world scratch 10,849,280 B, Pix3D scratch 52,747,392 B, free texels 20,971,520 B, pixmaps 46,346,688 B, minimap 16,777,216 B, and materialized overlays 13,856. Active texels and current/high-water transient meshes are zero in both boundary snapshots. These are fleet aggregates of per-renderer fields, not RSS corrections; do not subtract logical, commit, or GPU gauges from RSS.

The aggregate is fixed-versus-fleet accounting within this one N16 capture. Cross-renderer Arc identity is not established: equal per-renderer unique totals do not prove shared pointees between renderers, and shared-pointee counts are renderer-local. `unmeasured_mask=31` is explicit and means the census is incomplete; unmeasured categories cannot be treated as zero.

## Boundary freshness and periodic context

At observe-start, owner `sampled_ms` values span 1788832139885–1788832139964; at observe-end they span 1788832259889–1788832259968 (about 120 s later). All 16 owner rows carry `build_generation=5`, `phase=2` (`steady_resolved`), and renderer generations are unchanged per ordinal across the two boundaries. That is a boundary comparison only: it does not establish no intermediate owner changes or stationary residency.

The periodic raw renderer profile stream has no owner census rows. It does provide private-commit samples throughout seed/warmup/observe (for example, 2,771,111,936 B at elapsed 151.5663 s and 2,773,958,656 B at 201.9340 s) and the qualification root reports the 119.089 s diagnostic interval. Private commit, resident, GPU, logical and commit-related gauges have different scopes and must not be combined with owner bytes as one total.

## Conclusion

This census identifies large, fixed per-renderer owner categories and supplies a bounded attribution lead for one later ownership design. It does not explain total RSS, establish a leak, claim savings, or accept diagnostic overhead. No source, native operation, raw archive, control, STATE, or unrelated analysis file was changed by this report.
