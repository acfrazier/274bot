# Managed resource artifact binding

`managed_resource_binding.bind` runs after the existing receipt/build/workload
reader. It requires independently validated native qualification, verifies the
complete cache fingerprint and pre/post verification times, and matches its
canonical cache directory to the directory reported by the running client.

The process branch requires the prelaunch server/controller/ambient PID map plus
runtime launcher/collector identities, checks the server against its sidecar,
and verifies exact sampler module hashes (collector, system sampler, and native
sampler when selected). The collector output is SHA-bound to the completion
receipt and must enclose the native observation envelope under `process_evidence`.
No launcher-plus-elapsed approximation is accepted for this coverage.

The matched adapter exposes `managed_resources`. When available, actual cache,
renderer, sampler backend, and module hashes replace descriptive metadata labels
in the comparison keys. Missing evidence remains explicitly unavailable; old
artifacts gain no proof. Helper identities are run-local and are never required
to equal another run's helper PIDs.

Instrumentation perturbation remains a separate OFF/ON/ON/OFF experiment.
This integration does not turn a series or caller label into measured overhead.
Host, server and helpers remain separate owners; pressure and waited-child data
retain the process reader's explicit unassessed status.

Validation: 70 combined adapter/resource/cache/receipt tests passed, including
four new binding tests with production collector schema fixtures, ten malformed
binding mutations, changed cache/module bytes, different native clock origins,
missing coverage, and changed server identity. No live process or build was used
for the new binding tests. Runner emission is a separate pending integration.
