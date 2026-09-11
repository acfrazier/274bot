# First-run Browse picker check

Root fixed the operator-reported first-run picker at `1a2f2dcf`. Browse previously
submitted the catalog picker before submitting the Scripts window, leaving the
new picker underneath it. Pickers now follow Scripts. A short header explains
why the catalog picker appeared and what folder to choose; the file picker
also explains its purpose. Dialog close flags preserve Cancel, Not now and
successful selection instead of overwriting them with the titlebar open flag.

Three existing import/defer/first-Browse tests pass, with exact commands and
logs in `checks.json`. Formatting and diff checks pass. No new label-mirroring
tests were added. Root independently built the frozen host/client source plus
the included private `startup_watch.rs` entry, which isolates script stores
and invokes the production panel. `source-1a2f2dcf.json` and `binary.json` bind
the 1,453 files and executable; the dirty build label reflects that extra entry.

Native Mac CUA observations on 2026-09-11, process 69916:

- First Browse shows Import above Scripts, with the full explanation readable.
- Not now dismisses it and leaves Import catalog available in Scripts.
- Explicit Import reopens above Scripts with the appropriate shorter header.
- Cancel closes the catalog picker; Load opens its file picker above Scripts.
- Cancel closes the file picker. Closing/reopening Scripts honors the defer.

Root read all three internal F12 whole-window PNGs in `shots/`, retaining their
sidecars and SHA-256 list. The empty manual sidecars are UI capture metadata,
not gameplay proof. No vault was created, account added, or script started.
The production preparation used the real 289 resources; this does not claim
Windows coverage or gameplay acceptance. The isolated check window exited 0;
the operator's separate window remained untouched.

Source review is grouped with loading-progress card `t_42263bfd` and the final
integration review, rather than creating a standalone routine UI review card.
