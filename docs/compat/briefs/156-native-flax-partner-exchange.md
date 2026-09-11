# Complete the native Flax partner exchange seam

Use Sol defaults after native126 review. Read AGENTS/execution and the
fail-closed-dispatch skill, 05w-paired-runner-design.md sections1/3/5 and
06as paired-seam list. The required missing operation is
driveActivePartnerTrade as actually called by frozen FlaxRunner in both
catalogs. The approved boundary is a Rust host-owned exchange over native
trade operations; do not copy drivePartnerTrade.ts or PartnerTrade policy.

Inspect the exact caller options and existing reviewed trade126 capabilities.
Implement only the required giver/receiver exchange for an already-open trade,
with the script retaining callback decisions. Rust owns fresh posted trade
identity, operation sequencing, settlement and cancellation. JavaScript may
invoke caller callbacks and project their decisions/metrics into native
arguments; it must not recreate the foreign trade controller. Preserve existing
callback arity, refusal/error behavior and applicable frozen deadlines.

Require exact allowed counterpart where requested, the selected unnoted product
slots/counts, distinct offer/confirm phases and actual inventory change after
close. Decline, stranger, missing header and no-progress cannot invoke successful
completion. Receiver safety must reject an unintended own offer. Do not bypass
the caller's accept/missing-partner hooks. Keep the callback metric meaningful;
raw close is not transfer. No market/mule scheduler or alternative JS router.

Use native126 sends and compact Rust observations, not a second packet table,
whole-world clone or V8-to-JSON-to-Rust polling. Pause freezes active work;
Stop/logout/restart invalidates pending state and cannot send late actions or
callbacks. Generation/partner/modal changes invalidate prior decisions. Missing
host capability is an explicit refusal, never guessed success. Unused ancillary
call shapes need not be implemented by copying foreign policy; identify limits.

Own narrow script/api/host-play native files, meaningful composed tests,
report09c-native-flax-exchange.md and evidence/native-flax-exchange. No scenario,
shared CoreWitness, panel, client, STATE, ledger, LIVE or remote changes. Native
lane is serialized behind126; fixture155 may run in its own files. Exact source
exports and one exclusive reusable cache. Test real isolate callback dispatch
plus native sequence settlement, wrong counterpart, stale phase, no progress,
refusal and cancellation. Run affected tests and strict Clippy, record identities
and unresolved gaps, commit scoped changes, request SAME-card reviewer and stop.
