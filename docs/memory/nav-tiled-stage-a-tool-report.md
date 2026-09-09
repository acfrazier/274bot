# Stage A tooling report

Status: prepared tooling only; no real pack, native host, SSH, account, game,
server, cache, frontend, or performance run was executed.

Files are confined to `docs/memory/nav-tiled-stage-a/`. The Rust probe admits
only the two explicit synthetic modes (`all-uniform` and `all-dense`), rejects
malformed or unsupported ten-field selectors, and records state provenance for
presets 0..6. Presets 4 (`563x50,556x150,554x50; level 6=25`), 5 (carried
1712x1 only), and 6 (995x5000 only) are retained as distinct accepted selector
values; they are not remapped to the original bitmask.

The proposed manifest uses nine fresh processes per arm/cell, alternating dense
and tiled order, separate targets, System allocator, release mode, and fixed
wall/CPU/RSS/output limits. The predeclared Stage A thresholds are peak increase
<=8 MiB, cold-load median increase <=10%, route CPU increase <=5%, and route p99
increase <=2 ms. Repeat noise can classify a comparison inconclusive.

Accounting is deliberately milestone-based. The input byte buffer is retained
through decode/conversion and dropped after world drop; route results are reduced
to counts and a tick sum. The probe reports logical cell and blocked-word
counts, not an invented candidate allocation estimate. Collision-specific
allocation cannot be isolated from route workspace allocations here, so no
zero-read-allocation claim is made. The counting-allocator variant remains
separate and is not mixed into clean timing/RSS.

Qualification evidence is limited to source compilation and Python validation;
local synthetic output, when run, is tooling qualification only and cannot
establish memory savings or Stage A acceptance. Darwin has no hard address-space
qualification in this tooling. A future real-input launcher must add explicit
root authorization binding path/hash/source/lock/tool/binary and an owned output
directory; this card does not provide that release.
