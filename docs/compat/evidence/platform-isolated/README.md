# Isolated platform candidate 6c7075ce

Exact host 6c7075ce/client 56d80272, with both immutable catalogs and nav inputs:
archive SHA-256 84e955cccd1cbe821d8aff7bc7b911af0b82c2bfa1012a5774056a059fee4e0a,
5,933 files verified on both platforms. Each platform used a new empty target.
Windows built headless catalog_boundary_live and native catalog_watch. Linux
built headless catalog_boundary_live and tui-play. Downloaded build receipts,
compiler logs and source verification receipts are retained with their hashes.

Linux ran out of space while copying the already compiled TUI binary, after the
compiler reported success. Root verified the old campaign target was a compiler
cache with no open references, removed that old cache only, preserved the new
target and source exports, then recovered the copy. The partial-copy size/hash
and recovery receipt are retained. All source files and final binary hash were
reverified. Recovered TUI build duration is unavailable, not zero.

These are builds, not platform gameplay acceptance. Source includes native
canStep 5612b265 (review gate pending), reviewed DirectNavigator 5ce85959 and
Superheater Attack-30 fixture correction. The independent ChickenKiller fixture
review found an already-satisfied final XP arm; its separate correction remains
pending. Do not qualify that old scenario on this candidate.

The existing selected 289 engines on both remote machines were freshly verified
at 04:27–04:28 UTC: all 1,213 runtime files matched the retained fixture manifest,
client cache was unchanged and HTTP/CRC endpoints replied. That is readiness
only; no new platform actor has been started from this candidate yet.
