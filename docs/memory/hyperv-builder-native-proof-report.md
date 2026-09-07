# Native Hyper-V builder proof

2026-09-07. Root provisioning completed; project build qualification remains
pending. This is builder readiness, not campaign performance acceptance.

Reviewed generic image receipt: hyperv-generic-image-preparation-report.md,
3ca3e66, Grok4.5 session 20260907_144508_78a2e1, task t_82c720f3.
Root verified the transferred 3,766,484,992-byte VHDX against
`a77e2980c3d35ebc27f84a892b005ea7500b5d4eb68b6c0afdbb93303dc72e2f`
and the 921,600-byte CIDATA ISO against
`90e15120d5277a08d6f5acab21fd4ce940ec2decf4d779e8acdcb2a0725bef7f`
before creation. The base was made read-only and copied to a writable VM disk.
All Windows files are isolated under C:\ProgramData\274bot\hyperv-builder.

VM 274bot-builder, ID 34954ade-f213-45ef-8957-dfafdbaa53fc: Generation2,
4 vCPU, 8 GiB static memory, 64 GiB virtual disk, existing Default Switch.
Secure Boot remains On with MicrosoftUEFICertificateAuthority. Automatic
start is Nothing; automatic stop is ShutDown. Initial automatic checkpoint and
explicit pre-campaign-toolchain checkpoint are retained; later automatic
checkpoints disabled. Current active disk is a checkpoint AVHDX child of the
writable disk, not the immutable source. Seed DVD detached after successful
cloud-init. No host restart, viewer change, new external switch or adapter
reconfiguration occurred.

COM1 named pipe was added for first-boot observation. Its initial DebuggerMode
was On by default; root explicitly set Off before boot, preserving Secure Boot.
A bounded SYSTEM scheduled task captured serial bytes; that task is now stopped
and disabled. A PowerShell boolean escaping error while disabling automatic
checkpoints made no change; an encoded-command correction succeeded, confirmed
by current native state. No claim relies on the failed command's exit status.

## Guest and access proof

First boot completed cloud-init in about33s, datasource NoCloud on /dev/sr0,
errors [] and recoverable_errors {}. Ubuntu24.04 amd64, kernel6.8.0-138-generic,
`systemd-detect-virt=microsoft`, 4 CPUs. Root filesystem expanded to about61GiB,
about58GiB free at verification; guest has about7.75GiB visible RAM and no swap.

Guest DHCP address172.30.253.116 was read from the authenticated Windows serial
console. Hyper-V adapter IPAddresses currently returns empty; do not treat that
as network failure or invent a stable DHCP reservation. Ed25519 host key was
read from the same console and pinned before the first SSH connection. Host
fingerprint SHA256:dLqxNtRrlcZ7lfuBwGOY1mHx0sjp8pYWBzXjGO0EkIg.

Mac alias `ssh 274bot-builder` uses the dedicated login key and known-hosts file,
with the existing verified Windows SSH connection as ProxyCommand. No private
key was copied to Windows, the guest, Git or these artifacts. Authorized login
fingerprint SHA256:hzae4+XeHkXCux9DBNI+sa0/+uGFJT0FrcVXvU0/0fk.
Login is builder(uid1000), sudo allowed. sshd is active, password authentication
No, public key Yes, keyboard-interactive No; root login policy is
without-password, with no root key provisioned for this setup. An explicit
no-authentication probe failed255 and advertised publickey only.

Cloud-init installed the declared C/X11/Wayland/ALSA/SSL/clang/cmake/nasm build
dependencies. Root then downloaded official rustup-init plus its SHA256 manifest,
verified the hash, and installed Rust/Cargo1.98.0 with rustfmt/clippy. Native C
and Rust compile-and-run smoke checks passed. Use ~/.cargo/bin explicitly in
noninteractive builds; rustup used --no-modify-path. These smoke checks do not
replace affected project tests or matched build/source provenance. Existing
Mac matched Linux freeze remains active and is not replaced by this VM proof.

## Network observations and operator constraint

The image transfer completed; no sneakernet needed. Both Mac route en0 and
Windows adapter were Wi-Fi. Both reported2.4GHz channel6. Mac reported20MHz,
229Mb/s link rate and -30dBm/-81dBm signal/noise; Windows reported688.2Mb/s,
99% signal/-30dBm. These are link metrics, not application throughput.

Disk-free SSH probes, compression disabled: direct Windows16MiB download
8.60MB/s; guest via Windows proxy32MiB upload6.84MB/s/download8.70MB/s. Timing
includes SSH startup and the proxy has two SSH sessions. Tiny management reads
and initial VM activity may overlap; this is bounded troubleshooting, not a
controlled wireless benchmark. Two Windows stdin-upload probes timed out
(45s EOF-based, then25s exact-count) and have no throughput result; those are
probe failures, not evidence of network throughput or packet loss. ICMP got no
replies while SSH worked; do not infer network packet loss from that.

Disk I/O is not needed to reproduce the general low transfer rate. The shared
2.4GHz path is a leading hypothesis, but SSH overhead and other factors remain
unseparated. Operator clarified that others use the network: no shared-network interruption. Throughput tests are authorized; changes
that reset anyone else's connection are prohibited. No network settings were changed. A later comparison changing only our band/wired connection would be needed
for causal attribution; shared router changes are not authorized.

## Evidence

All local raw proof is under docs/memory/diagnostics/hyperv-builder-native-20260907:
artifact-sha256.json lists the exact retained files, including native state and
text enum labels, create transcript/scripts, first boot serial, cloud-init JSON,
guest/access verification, no-key rejection, seed verification/provenance,
toolchain install command/log, and transfer-probe results. User-data itself and
private keys are excluded. Native root remains running and ready for bounded
build work. Future clean Windows measurements must keep builder activity quiet
and separately record the VM state.

## Subsequent direct TCP probe

Operator clarified throughput tests are fine; nothing may reset other users'
connections. A single32MiB each-direction direct TCP test (no SSH encryption,
compression or disk I/O) measured upload11.124MB/s and download9.892MB/s,
excluding connection startup. This narrows the bottleneck: SSH is not the sole
low-throughput limit, though it adds overhead in the earlier tests. Link
conditions/other traffic varied, so this is not a matched cipher benchmark or
proof of the router's exact bottleneck.

The Windows listener bound10.0.0.205:50274 with a temporary private-profile
firewall rule restricted to Mac10.0.0.176; the handler also checked the peer IP.
It used a256KiB buffer,32MiB per direction and bounded waits. Successful task
exit0 and native cleanup prove rule_count0/listener_count0; its no-trigger
scheduled task is disabled. No Wi-Fi/router settings or connections changed.
Raw client timings, scripts and native cleanup are in the same hashed evidence
directory. No additional throughput or reconnect experiment is underway.
