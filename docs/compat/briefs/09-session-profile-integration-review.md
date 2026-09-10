# Step 3 integrated profile milestone review

Use the configured `grok46` profile defaults, with no model/provider overrides.
This is the coherent step-3 integration review required by docs/execution.md;
the first Astra/Grok trial completed at step 2 and is not repeated here. Do not
substitute this milestone for the final whole-campaign branchreviewer pass.

Read AGENTS.md and docs/execution.md once, the plan architecture plus step 3,
01-session-profile.md, 01-session-profile-frontends.md, and the frozen review
manifest `reviews/session-profile-integration-inputs.json`. That manifest names
exact host base/head and client base/head, prerequisite actual-model receipts,
source/evidence hashes, and checks. Verify those refs before reviewing. Dated
reports for later steps are not current instructions.

Review the combined client binding, host profile/template/Play ownership and
panel/TUI/host-play production paths. In particular check:

- CLI > environment > saved > default selection; negative named/revision/target
  combinations; unchanged 274 defaults and explicit paths; distinct 289 paths.
- Binding and checked assets before vault mutation, slot construction, OnDemand,
  HTTP/maininit/login/reconnect. One immutable process profile and shared Arcs,
  including across panel lock/unlock; no second effective endpoint config.
- Panel revision selection before the session, accurate effective revision and
  server labels, CLI/environment precedence and restart-required refusal later.
  Invalid selection must never reset a fallback default vault. TUI title uses
  the same selected profile; live/memory entry points use checked construction.
- Cache, CRC, RSA, nav/flags and catalog identity are retained and validated;
  wrong/missing 289 nav never falls back to 274. Catalog custom script loading
  remains available within the bound contract. No deep-copy regressions.
- All production 289 bot operations refuse before queue/status/vault mutations
  until step 4 qualifies direct host writers and guardians. Bound client 289
  construction remains available for that qualification. Public cheats refuse.
- Client retains sole protocol/world/render ownership and no host dependency.
  Legacy APIs/fixtures retain their behavior; no completed old stubs, changed
  policy/timing, or invented packet handling. No 377 gameplay claim.

Review cold against the exact manifest inputs. Existing full client/GPU/backend
and final frontend suites have raw receipts; use focused independent checks or
reproductions for unresolved risk. Do not repeat every suite merely for a new
review. Do not launch live/native work, change product code, create subagents,
merge, push, or move the client gitlink. Root performs native macOS startup and
visual validation after a valid integrated review. Linux/Windows 289 engines
are prepared only for their later checks; those platforms remain unqualified.

Write findings and an explicit approve/request-changes verdict to
`docs/compat/reviews/session-profile-integration-grok46.md`, with exact reviewed
refs and commands. Distinguish source approval from native/evidence acceptance.
Complete this independent milestone card with actual review outcome and report;
do not request the routine reviewer profile on this review-only card. Root owns
committing the report and exporting actual model/session receipts.
