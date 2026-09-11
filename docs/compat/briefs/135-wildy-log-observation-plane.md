# Correct Wildy log destination observation plane

Active campaign codex/rs2b0t-multirevision; read AGENTS.md and docs/execution.md.
Own only scenario/lib.rs Wildy fixture and host-play/tests/catalog_boundary_live.rs
Wildy oracle/constants/tests. Serialize after131 before132 in the fixture lane.
No runtime/API/client/nav changes, no LIVE. Same-card reviewer after scoped commit.

Exact15e779cad/clientaef four new Wildy runs are retained in
catalog-harness/live/*-wildy15e.{json,log}. Three reach all five script obstacle
messages and lap1, two reach lap2, but the fixture remains at step16 requiring
arrived_near(2994,3945,1,3). Current scene control uses Loc2297 level0. The scenario
constant WILDY_LOG_DEST and harness duplicate both expect1 based on selected
map loc level. Inspect BOTH selected server maps/scripts and the client's
bridge/effective level semantics, distinguish scenery raw plane from observed
player plane, and correct this one destination to the actual player plane when
supported. Native wilderness_course.rs2 performs x+1,-4,-3,-1 (net-7), no level
change. Do not replace the entire full-lap oracle with script log messages.

Retain all ordered ridge/pipe/swing/stone/log/rocks XP/destination witnesses and
next-pipe continuation. Keep Agility52, HP/food/settings, all150 tick watches and
180s global budget. One274 newer run falls at ridge/dies before XP; preserve that
failure and do not try to qualify it by weakening the fixture. No retries here;
root does fresh controlled four-cell qualification after reviewed correction.

Focused meaningful fixture/oracle tests must distinguish actual observed plane
from the incorrect plane; no tests merely mirroring an unverified literal.
Formatting + affected existing checks suffice. Report05zb-wildy-log-plane.md,
evidence wildy-log-plane. Explain source refs and effective-plane evidence, then
request same-card reviewer and stop.
