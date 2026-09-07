# Continuous process evidence reader

`process_evidence.validate` reads a caller-supplied, SHA256-bound schema-2
collector artifact and independently checks the explicit PID/start identities,
backend, sampling interval/mode, ordered grid, complete role sets, monotonic CPU
counters, acquisition brackets, wall/monotonic consistency, and completed
summary. It rechecks the file hash after reading. No live process is accessed.

The caller must supply the actual native observation wall envelope and expected
role identities from the managed launch, not launcher-start-plus-elapsed guesses.
Both fixed and explicitly stopped collectors need samples enclosing the entire
observation. Missing evidence stays unavailable. This module does not yet wire
itself into the managed runner or matched adapter.

RSS median uses samples wholly within the observation. Sampled peak uses the
whole recorded series and is explicitly a sampled peak, not continuous peak
capture. CPU uses the nearest acquisition wholly before the observation and
wholly after it; output records endpoint excess and the enclosing wall envelope.
The CPU-core interval divides cumulative CPU difference by the maximum/minimum
possible duration between the per-role acquisition brackets. It does not claim
exact observation-only CPU or interpolate counters between samples.

Process roles are separate; neither RSS nor footprint is summed here. Pressure
and waited-child rows remain unassessed raw evidence. Descendants are outside
explicit-PID coverage. These limits prevent this primitive alone from claiming
instrumentation overhead, host pressure health, or performance acceptance.

Five tests pass, using schema output from the production collector with injected
clock/process samples. They cover available enclosing evidence, twelve malformed
artifact mutations, missing boundary coverage/identity, controlled-stop completion,
and hash mismatch. No bot/server run or release build occurred.
