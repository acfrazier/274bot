# Capture-corrected native input follow-up

Predeclared 2026-09-08 after the bounded Grok 4.6 accept in
[the integration review](cohort-integration-grok46-review.md).

Run one N=1 active focused-one diagnostic with candidate host
`e3a2cbfbdfdab37a4f35ee330e91a7e6264c14d7`, client
`fd956c91bf09e059359c8e182a33583e2c626cd3`, and native binary SHA256
`94b55e21e22be3c45aaa2c50b878965e4e730abd4e9dd69ab0533af7b850d330`.
It includes the reviewed capture transition correction and stable-frame wake
gating. The source justification is the reproduced missing-channel failure in
[the preceding diagnostic](native-cohort-input-trace-report.md).

Keep N, fixture, policies, environment flags, 120s warmup, 180s observe,
5s cohort tail, normal teardown, and exact reviewed f64b81d input helper unchanged.
Keep `BOT_DEBUG=1` for stage observations. Do not overlap native builds/tests.
Use new stage `cohort-candidate-e3a2cbf` and new run directory
`latency-diagnostic-input-trace-20260908-b`. Capture helper bindings must name
this binary and hash from the start. Inspect a fresh scene2/configuration capture
and derive current rectangle/Game Image point before sending one 20-pulse sequence.

Success for this functional proof requires channel attachment through input
pulses, actual stream/drain/metric admission observations for the correct live
slot/generation, and published input starts/bind/present completion accounting.
Report raw cohort records and incomplete members exactly; do not infer a pass
from SendInput return values. Preserve all rows, receipts, sidecars, captures,
errors and terminal status, then verify the complete archive.

This diagnostic remains outside matched performance acceptance. Its simple
supervisor does not generate canonical matched metadata. Any direct reader call
using qualifier-mapped metadata is structural verification only. The Grok 4.6
finding about uncached host debug environment lookup is being corrected separately
and must pass review and be applied identically to both native roles before a
performance comparison. It does not invalidate this authorized BOT_DEBUG=1
functional localization run. The reference role will be rebuilt after that
correction, avoiding an unused intermediate reference build.
