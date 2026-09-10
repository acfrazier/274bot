# Step 3 server profile design and review contract

The operator explicitly requested step 3 on 2026-09-10. Validate it on macOS.
Linux/Windows need their own 289 engine setup before later platform checks;
those checks follow macOS step 3. Current host base is
5bb500262fb4e7ab8507589c0784a36f9eac14ce and the accepted client is
58120f28ee5208553ca07f41cb364f2cf98ea280. Both campaign branches are clean.
Read the existing step-3 preparation brief and the plan's architecture/step 3.

## Proposed ownership and behavior

1. A general immutable client connection/resource value contains revision,
   target, game and asset endpoints, cache and unpack roots, public RSA values
   and optionally expected CRCs. Client has no host/nav/catalog dependency.
   A shared constructor accepts Arc of this value before OnDemand construction.
   Bound clients use it for initial HTTP, login/reconnect, snapshots and the
   update worker; later ambient environment changes cannot redirect a slot.
   Keep standalone legacy constructors and their current defaults/behavior.
   Public mutable ClientConfig fields must not change bound connection inputs.
   Socket adoption rejects incompatible bound identities before taking/mutating
   either socket; legacy adoption retains current behavior.

2. Existing OnDemand shares one worker by host/port. A bound-profile path must
   also distinguish target, revision and cache/content identity, or reject a
   mismatch before joining; it must not share the wrong version table. Preserve
   worker ownership, subscriber counts and socket lifetime. Keep existing
   memory sharing (Arc cache, interface tables and mutable-overlay template).

3. The Rust host resolves an Arc<ServerProfile> before assets, vault mutation
   or connections. It owns the client binding plus nav/content/catalog and
   vault identity. Built-in selections are local 274, local 289 and known
   public 274. Unsupported revisions and public 289 fail with a concrete reason.
   Local game/HTTP defaults: 274 43594/80 and 289 44594/1080. Distinct 289
   engine/cache/unpack/nav/vault defaults; existing 274 defaults and explicit
   path overrides remain. Public 274 retains its known server/asset/RSA pairing
   and local-only fixture restrictions. Resolve explicit CLI overrides after
   all flags, then environment, then saved panel revision, then defaults.
   A CLI revision conflicting with an explicit named profile fails early.

4. Resource validation must detect wrong cache/nav identity before Start, not
   silently substitute empty tables or 274 nav. Use the smallest manifest
   seam needed: identify cache/config/versionlist content and declared revision;
   allow matching explicit copies. Preserve the existing legacy 274 nav path;
   a 289 nav pack requires revision/content metadata. Actual nav baking and
   guardian/content adaptation are step 5. Missing nav must remain an explicit
   unavailable capability, not make a headless constructor pretend it loaded.
   Do not force all unrelated legacy test constructors to require the engine.
   An additive checked run/construction path is preferred to silently breaking
   the old empty-cache test/API contract; actual frontends use the checked path.

5. PlayOptions carries the resolved profile, all slots inherit it, and any
   redundant old endpoint fields must agree before mutation. All production
   construction flows through host prepare_client_with_profile before maininit.
   Resolve the profile once for the actual frontend process/session; account
   credentials/settings stay separate. Any new legacy wrapper freezes 274
   defaults at construction and must not become an unqualified 289 bypass.

6. Panel and TUI consistently parse --revision 274|289 and profile/resource
   overrides. Panel exposes revision before sessions start, persists only that
   new setting, and prevents revision changes after process/session binding.
   Display selected server and revision in both frontends. Changing an active
   revision requires process restart. Do not add last-script persistence.

7. Step 4 owns all direct host packet writers and their live qualification.
   Until then a configured 289 profile must refuse bot operation with an
   explicit host-boundary-not-qualified error before current raw 274 writers
   or automatic guardian/run actions can execute. Shared construction tests
   may exercise actual bound 289 client setup and mocked handshake directly.
   This intermediate refusal must not dim/remove catalog cards or claim 289
   gameplay. Keep actual profile construction independently callable so step 4
   can qualify it without bypassing identity validation.

## Bounded experiments / proof

- Pure precedence/negative tests: default/explicit 274, local 289, unsupported
  revision, named profile mismatch, wrong cache/nav, public pairing, overrides.
- Real shared constructor: Arc identity and actual HTTP mock endpoint used by
  maininit/CRC fetch. Socket handshakes prove selected revision/RSA, reconnect
  binding and rejection of incompatible adoption. Keep existing timeouts.
- Tests through panel and TUI parsing/session construction to the actual host
  constructor, plus persisted revision migration and post-bind change refusal.
- Affected host/frontend/client suites, separate client integration tests,
  format/appropriate Clippy. Keep failed artifacts. Mac native launch/visible
  revision check when coherent. No gameplay acceptance or performance claims.
- Same-card Grok 4.5 implementation review and a coherent integration review
  because this crosses client/host lifecycle; final campaign Grok remains later.

## Architecture review assignment

Profile grok46: inspect the proposal against the named source and source seams.
Look for missing ambient settings, ownership leaks, identity/caching hazards,
behavior regressions, fail-open paths and an unnecessarily broad API migration.
Review only. Do not implement or rerun broad suites. Write the concise verdict,
required changes and useful concrete API guidance to
`docs/compat/reviews/session-profile-design-grok46.md`, then complete your card.
Do not change other files, create subagents, commit, merge or push. Root will
incorporate findings before implementation dispatch. This design approval is
not runtime evidence. Check branch first per AGENTS.md.
