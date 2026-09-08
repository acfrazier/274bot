# Making 274bot lean: what we have changed, and why

Written for the operator, 2026-09-08. This is a reading guide to the memory and
performance campaign, not a release note or a claim that its targets are met.
For the latest execution status, use [STATE.md](STATE.md).

The central problem is that a bot host contains several applications' worth of
state: a game client, a world model, navigation, a script runtime, observation
messages, and sometimes a renderer. Running more bots multiplies some of those
costs. Other costs are shared, or appear only during startup and scene changes.
Making this lean means understanding who owns each large object, why it exists,
how long it stays alive, and which other components really need their own copy.

That is difficult work, but it is not evidence that a modern application has to
be wasteful. We have found concrete, avoidable costs. The difficulty is removing
them while keeping the application useful: the same game behavior, script
observations, command order, graphics, responsiveness, and Stop/restart behavior.

## What we are aiming for

The campaign targets a useful small deployment first. It covers a Linux terminal
interface and two GPU panel modes: one renderer with other bots simulating, and
a focused renderer with background views painting at one frame per second.
Background bots must continue their normal simulation in both modes.

| Steady host-process memory target | Terminal interface | GPU panel |
| --- | ---: | ---: |
| One active bot | 256 MiB | 384 MiB |
| Sixteen active bots | 512 MiB | 768 MiB |

Additional targets cover startup peaks, CPU, the cost of adding each bot,
simulation and frame cadence, input latency, and repeated start/Stop cycles.
The game server is accounted for separately. Large-fleet ambitions remain
important, but a powerful development machine running many clients does not
prove that one or sixteen clients run well on modest hardware.

The complete requirements are in the
[approved finish-line plan](performance-finish-plan.md).

## What has changed

**First, we made costs and useful work observable.** The harness records current
resident memory, CPU time, allocation and ownership diagnostics, script-runtime
state, simulation progress, rendering activity, and timing. Runs also record
which source and executable were used, along with their settings and environment.
Workload checks establish that every requested bot is actually ready and doing
its job. A low-memory run in which half the bots never start is not an improvement.

This measurement infrastructure is a substantial part of the work. An allocation
counter tells us what the program requested; resident memory tells us what is
currently in RAM. Neither alone explains GPU memory, shared mappings, allocator
retention, or whether Stop released the last live owner. We keep those questions
separate rather than subtracting unlike numbers into an attractive total.

**We removed duplicate navigation ownership in the test setup.** Production
navigation was already shared, but preparation and scenario runners could retain
additional decoded navigation worlds. The harness now reuses the existing
shared world where available. Each bot keeps its independent execution state;
sharing the map does not share its route or decisions. This also prevents a
benchmark's setup overhead from being mistaken for an unavoidable product cost.
See the [navigation ownership change](shared-play-nav-world-report.md) and its
[resource screen](shared-nav-resource-live-report.md).

**We reduced copying between the host and scripts.** An observation contains
lists, strings, widgets, inventory, and other state. Rebuilding an owned copy
just to discover that nothing changed creates unnecessary allocation traffic.
The borrowed fingerprint path compares against existing state first and updates
only changed fields, while preserving the bytes and ordering the script receives.
This matters for CPU and allocation churn as well as memory. It is a concrete
implementation improvement, not by itself proof of a particular RSS reduction.
See [borrowed fingerprint comparison](borrowed-fingerprint-report.md).

**We made sparse storage cheaper.** Some client tables reserved room for a large
value in every position, even when most positions were empty. Appearance-packet
storage is a clear example: on the measured layout, an empty entry occupied
2,112 bytes; a pointer-based optional entry occupies 8. Across 2,048 positions,
the empty-table structural difference is about 4.1 MiB per client. Occupied
entries still retain their full packet, and now also pay allocation overhead.
The game protocol and appearance-cache behavior remain intact.

Similar work investigated sparse scene-tile model storage. These changes are
about paying for data that exists, rather than reserving its largest form
everywhere. The tradeoff is extra indirection and potentially different allocator
behavior, so actual scene transitions and resource measurements still matter.
See the [appearance implementation](appearance-packet-box-report.md) and the
[Windows tile screen](windows-tile-boxed-background-screen-report.md).

**We added an opt-in way for snapshot owners to share identical bodies.** The
host, navigation consumers, and panel can simultaneously own equivalent snapshot
data. The snapshot-dedup feature shares exactly equal, completed bodies within
one slot's lifetime. It keeps changed data independent and uses weak references
so the registry does not become a permanent cache. Default builds leave this
feature off; its implementation and behavior checks must not be confused with
an enabled, measured deployment win. See the
[snapshot landing report](snapshot-dedup-production-landing-report.md) and
[frame-equivalence tests](snapshot-frame-equivalence-report.md).

**We separated simulation, rendering, and waiting costs.** A bot that does not
need a resident renderer should not automatically pay for one. Conversely, a
background renderer at one frame per second is still a real renderer with
memory and lifecycle costs. Socket/control waiting was also made portable to
Windows without replacing event-driven waits with busy polling. Native Linux
and Windows work exposed assumptions that local tests alone could not check.
See the [socket portability report](windows-socket-portability-report.md).

Behavior validation has included real gameplay, focus changes, renderer changes,
and scene transitions. A synchronized Windows capture showed the old scene and
minimap remaining visible during a rebuild, followed by the new scene. That is
useful qualitative evidence; it does not prove every overlay, frame interval,
or display-presentation requirement. The
[capture report](windows-nullraster-synchronized-0830-report.md) states that scope.

## Why measurement and review take so much effort

A representation change can remove allocations without immediately lowering
resident memory. Allocators may retain freed pages. Startup memory can decay
through an observation period. Different scenes or renderer counts can dominate
the change we intended to measure. Instrumentation itself can alter timing.
These effects make a single before/after number easy to misread.

Two examples show why we keep implementation and acceptance separate:

- A longer appearance-storage pair observed about **58.4 MiB lower median RSS
  at sixteen bots**. Earlier short runs were variable, so the implementation
  was retained provisionally for its concrete allocation removal and behavior
  checks; an accepted RSS/CPU improvement was not claimed.
  [Evidence and decision](appearance-confirmation-report.md).
- Boxed tile storage looked promising in short background-renderer screens,
  but startup age and resident-memory decay complicated attribution. The longer
  focused-renderer comparison did not demonstrate a sustained RSS improvement;
  that saving claim was parked.
  [Longer comparison](windows-tile-boxed-long-screen-report.md).

Those results cannot be added together into a cumulative saving. They come from
different builds, workloads, platforms, and experiment stages.

The current timing work addresses another subtle problem. If an operation starts
just before an observation ends and finishes just after it, simple counters at
the edges cannot establish its latency or whether anything was lost. The new
bounded cohort journal tracks operations that start inside one fixed window and
allows them a finite completion tail. The reader must account for the whole
declared population, preserve losses, and reject missing or contradictory evidence.
It must not search for a quieter interval that happens to pass.

As of this document, the producer and publisher have reviewed implementations.
The reader is undergoing corrections and review after independent probes found
cases that incorrectly reported success. Native focused-input coverage also
remains unresolved; a source tracer has been added to locate where input stops
progressing through the panel. These are active prerequisites for the next
comparison, not completed responsiveness results. See the
[current latency plan](current-latency-companion-plan.md).

## Where we stand, and where to look

There is real implementation progress, better attribution, and useful native
diagnostic evidence. There is not yet a completed low-end performance claim.
The dated [finish-line gap audit](current-finish-line-gap-audit.md) records both
encouraging observations and misses: for example, its Linux one-bot diagnostic
was around 167 MiB, while its sixteen-bot diagnostic was around 611 MiB against
a 512 MiB target. These are different diagnostic cells, not a final matched
result or a universal statement about current builds.

Remaining work includes complete latency accounting and native input proof,
remaining resource gaps, repeated final measurements across all three modes,
start/Stop and longer lifecycle checks, scaling qualification, and the required
whole-branch review. We will finish against those requirements, rather than
quietly replacing them with whichever tests already pass.

You do not need to follow every worktree. Use these three documents:

1. **[This overview](overview.md)** for the high-level explanation.
2. **[STATE.md](STATE.md)** for the current checkout, next action, and latest
   verified boundary. Its top entry is current; older entries are history.
3. **[The finish-line plan](performance-finish-plan.md)** for what “done” means.

The active campaign checkout is
`/Users/acfrazier/experiments/274bot/.worktrees/t_a1f4796f`, on
`codex/memory-diagnostics`. Other worktrees isolate experiments, implementation,
review, or frozen comparison builds. Some intentionally contain older host or
client revisions. A task being reviewed there does not mean its changes are
integrated into the campaign branch. Dated reports explain their own experiments;
their old “next step” sections do not override current STATE.
