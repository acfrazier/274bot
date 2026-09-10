# Step-4 integration review

Use configured `grok46` defaults. This is the coherent integration review before
controlled live/native qualification, not another reviewer trial. Host branch
`codex/rs2b0t-multirevision`; client branch `codex/bothost-274-289`.

Review frozen host base `41d896b3007e476c76a9c01352b64d5afd5ed901` to head
`b9cacc6e5b3b5101cd2baf6b2a91e9ecf13f84f8`, including client base
`2be1697060e4d2b8b709ad4d5e54d12513b38333` to
`6cb5a0b17aeef74da6b57b205e916681daee4f76`.
The parent snapshot card must have completed its actual Grok 4.5 review.
Other prerequisite same-card approvals: public profile `t_96dc78da`, outbound
`t_b22abf57`, original live harness `t_079d478d`. Their receipts are under
`docs/compat/evidence/{public-profile-correction,host-boundary}`.

Read the applicable AGENTS/execution once, step 4 of the active plan, current
`13-host-snapshot-reset.md` top, `02a-host-outbound.md`,
`02b-host-snapshot-reset.md`, and the source/evidence for script-loading policy.
The operator explicitly clarified that script loading/starting must be revision
agnostic. End users decide suitability for revision/platform; host/client
operations refuse unsupported capabilities. Earlier profile/catalog hash gates
are superseded. Server/cache/nav identities remain process-bound. Catalog proof
is not a loading allowlist. Temporary 289 slot gating awaits world qualification.

Assess the combined public-289/local revision mapping, actual named outbound
writes and payloads, snapshot packet publication, reconnect watermark, all-family
invalidation accounting, producer/consumer queue order, isolate late replies,
script keyframe lifecycle, and shared live-harness path. Verify existing 274
behavior/ownership/timeouts and region/scene-ready transitions. No deep-world
copy or invented tick packet. Inspect surrounding production consumers for
missed actions/state surviving reset; do not infer safety from test names.

This is source approval for bounded live qualification, not live acceptance.
The required controlled loop is login/scene2, script/host-caused walk, NPC and
loc interaction, and actual IF logout for 274 and 289 separately. Existing
harness predicates/timeouts may not be weakened. No performance claims.

Do not implement, commit, stash, reset, checkout, restore, alter the index or
move any concurrent root/worker WIP. Inspect exact commits with git show or use
a separate temporary source export for a necessary test. Report actionable
findings or approval to `docs/compat/reviews/host-boundary-integration-grok46.md`;
record exact reviewed hashes and source/evidence limits. Complete this review
card only after that independent verdict, or block with concrete findings for
root. Root owns all integration and Git hygiene.
