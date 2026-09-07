# Development status

Updated 2026-09-07. Public development branch: `codex/memory-diagnostics`.

This branch makes ongoing work available for inspection and deliberate testing.
Public `main` and existing release tags are unchanged by this publication.
The alpha release overview in the README describes the released surface; it is
not a complete inventory of this development branch.

## Progress

The memory campaign adds allocation and resident-memory diagnostics, workload
qualification, artifact-bound comparisons, scheduling and responsiveness
measurements, and explicit renderer observations. Reviewed changes include
shared navigation ownership, smaller appearance-packet storage, and private
animation-base sharing. Allocation removals and observed differences are kept
separate from accepted resident-memory savings.

Native Windows work includes process measurements, operator-home paths and
ConPTY terminal support. Native Windows and Linux TUI diagnostics have run real
local-server gameplay. These are functional proofs; native evidence binding,
helper coverage and performance validation remain incomplete.

## Before promotion to main

- Complete the agreed memory, CPU and responsiveness gates, or obtain an explicit
  decision on a measured blocker. Current results do not establish those targets.
- Finish panel rendering, visual, lifecycle and scaling validation, including
  the required target-platform evidence.
- Resolve candidate acceptance and complete the required whole-branch Grok 4.6
  review. Per-task reviews are not campaign approval.
- Verify the proposed integration builds and runs with its exact client
  submodule revision, and record remaining limitations in release notes.

The current acceptance plan is [performance-finish-plan.md](docs/memory/performance-finish-plan.md).
[STATE.md](docs/memory/STATE.md) records the campaign timeline; later entries
supersede earlier next actions. Reports distinguish failures, diagnostics and
accepted evidence. Some linked raw captures and local fixtures are intentionally
not distributed; a report link alone is not a self-contained reproduction.

## Trying this branch

Use a separate checkout and test profiles. Do not switch an operating fleet to
this branch simply to follow development.

```sh
git clone --branch codex/memory-diagnostics --recurse-submodules https://github.com/acfrazier/274bot.git 274bot-development
```

The host pins its client by commit. Use `git submodule update --init --recursive`
when updating the host; do not use `git submodule update --remote` to select an
independent client tip. Local server/cache prerequisites still apply. No game
assets, operator credentials or production account database are supplied.

Development branches may advance before a release is ready. Promotion and
release tagging are separate deliberate steps; publication here does not change
the default checkout or advertise validated fleet capacity.
