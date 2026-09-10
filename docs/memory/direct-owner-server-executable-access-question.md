# Direct-owner server executable access — concrete native blocker

Actual controller6f2b410 on Concord cannot read /proc/726/exe. Root reproduced
linux_executable_basename(726) raising CellError with PermissionError errno13.
Fresh /proc/stat start identity is linux_proc_start_ticks:595, euid1000 matches
server uid1000, systemd MainPID726 matches, and the service declares
AmbientCapabilities=cap_net_bind_service. /proc/stat resource sampling succeeds.
Capability mismatch appears consistent with the access restriction; this is an
inference, not proof of the complete kernel policy. Read-only sudo -n readlink
also failed because a password is required. No sudoers/sysctl/service change,
restart, privileged frontend or live run was performed.

Evidence: diagnostics/direct-owner-managed-extension/server-executable-access/
actual-controller-failure.json. Existing controller's direct preflight and
recheck require actual executable basename from /proc/PID/exe. Service ExecStart,
comm MainThread or a declared filename are not substituted as live image proof.

Review requested: smallest legitimate read-only identity adapter, potentially
an explicit optional direct-mode helper with a fixed absolute sudo/readlink
command and exact validated PID, bounded timeout/output and strict parsing.
No arbitrary shell/helper command, credential collection, root frontend,
policy relaxation, process mutation or server restart. Normal readable proc
behavior stays unchanged; unavailable helper remains a failed preflight.

Before any operator approval request for host permissions, finish a concrete
adapter design and implementation/review/native qualification as applicable.
Then provide the exact narrowly scoped permission action for approval; the
current plan explicitly prohibits permission changes without further authority.
Do not assume that this question authorizes any such host action.
