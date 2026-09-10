# Native panel startup beachball

The operator reported a startup beachball and requested tracing if reproducible.
Root reproduced a blank, unresponsive startup window using the unchanged frozen
f2b04198 panel on local Mac 289. The process started at 22:10:46 UTC; its first
slot-thread log arrived at +37.471 seconds, and the first login handshake at
+39.224 seconds. The startup screenshot was read. The later Alcher run retains
its known banking failure; it was repeated here specifically to trace startup.

The 30-second `/usr/bin/sample` capture, sampled at 10 ms, places the main
thread in `boot_execute -> Session::bind_profile_with_env` and in
`live_prepare_script -> unlock_at -> start_play -> run_with_template`.
Both lead to `ServerProfile::validate_resources -> nav::manifest::hash_file`
and SHA-256 computation. The first window exists, but boot runs synchronously
inside its next render callback and prevents event processing during that work.

Source inspection identifies three hashing passes over the same resources:

1. `ProfileSelection` resolution's `bind -> validate_nav` establishes identities.
2. `SharedClientTemplate::load` calls `validate_resources` before decoding assets.
3. `run_with_template` calls `validate_resources` before spawning the slot.

The selected navigation pack is 73,441,977 bytes and its flags are 260,571,161
bytes: 334,013,138 bytes per pass, 1,002,039,414 bytes over three passes, plus
cache checks. `hash_file` reads each whole file and hashes it. The sampled
binary is an unoptimized debug build and the captured stack uses software
SHA-256. The trace establishes CPU work on the UI thread before login, rather
than a login timeout. It is not a matched performance benchmark or a release
build timing claim; review/build activity overlapped this diagnostic.

The bounded fix direction is to prepare the profile/template away from the UI
event thread, then install the completed immutable result on that thread.
Any removal of repeated validation must preserve the existing changed-resource
rejection contract. No validation checks or startup behavior were changed as
part of this tracing request.

Raw sample, window capture, line timeline, exact launch command, binary/source
identity, process outcome and SHA-256 inventory are in
`evidence/catalog-headed/startup-trace-f2b04198.json` and its referenced files.
The earlier Thiever covered-window captures were a different observation:
its UI thread was running and the final raised-window render updated normally.
The operator saw a macOS debug-log permission prompt around root's earlier
sampling call. That call completed; no permission-caused stall was established.
