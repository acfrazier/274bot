# Observation and process clock correlation

The launcher's `started_unix` and the Rust harness's `Run.started` have different
origins. Adding harness elapsed seconds to launcher start is not an exact wall
timestamp. New successful qualification boundaries carry `elapsed_wall_bracket`
with `before_unix_s` and `after_unix_s`: two SystemTime reads surrounding the
existing harness Instant read, after slot collection. A clock before the epoch
is null; readers must reject unavailable, inverted, or inconsistent brackets.
These two boundary-only clock reads do not change actions or scheduling policy.

The process collector adds `acquisition_before_utc` / `acquisition_after_utc`
surrounding the full process sweep. Its historical `utc` is written later, after
pressure acquisition, so it is not used as the process acquisition endpoint.
Monotonic per-role brackets and CPU intervals retain their existing meanings.
The sweep brackets conservatively enclose every role read; they do not assert
simultaneous acquisition or clock synchrony with another machine.

Readers must verify wall/monotonic consistency and coverage of the native
observation envelope independently. Old artifacts cannot gain this evidence
retroactively. No live run, release binary, overhead proof, or performance claim
is part of this change.

Validation: existing 42 collector/native tests passed, plus a new deterministic
test that delays process acquisition and then pressure acquisition separately
and proves the brackets enclose only the former. Full host-play library suite
with `memory-profile-no-alloc`: 175 passed.
