# Step 3 panel/TUI/host-play launch wiring

Bounded demanding frontend implementation. Work on the campaign host branch
codex/rs2b0t-multirevision. Read AGENTS.md and docs/execution.md, the plan step 3,
and reviews/session-profile-design-grok46.md. Root owns host/profile/backend
files concurrently; another Sol owns the client submodule. Do not edit those
areas. Allowed product files: crates/panel/**, crates/tui/** and
crates/host-play/src/main.rs. Add focused frontend tests alongside those files.
Write the task report to docs/compat/01-session-profile-frontends.md and concise
raw evidence under docs/compat/evidence/session-profile/frontends/.

## Backend API root is implementing (already declared in working source)

`host_play::profile::{ProfileOptions, ProfileSelection, ProfileEnvironment,
ServerProfile, ServerSelection, parse_profile_args, parse_revision}`.
- `parse_profile_args(args) -> Result<(ProfileOptions, Vec<String>), String>`
  consumes --profile, --revision, --prod, --host, --port, --asset-host,
  --http-port, --engine, --cache, --unpack, --nav-pack, --nav-flags, --content,
  --vault, --catalog, --cache-manifest. Remaining args belong to the frontend.
  It has no global mutation. Use it before frontend-specific parsing.
- `ProfileOptions::resolve(saved_revision: Option<u16>) -> Result<ProfileSelection,String>`
  resolves CLI > captured environment > saved panel revision > defaults. Testable
  `resolve_with_env(saved_revision, &ProfileEnvironment)` avoids env mutation.
- ProfileSelection getters: selection(), revision(), target(), game_host(),
  game_port(), asset_host(), asset_port(), cache_dir(), unpack_dir(), nav_pack(),
  nav_flags(), content_dir(), vault_path(), catalog_root(), label().
- `ProfileSelection::bind() -> Result<Arc<ServerProfile>,String>` validates and
  freezes actual cache/nav/catalog/RSA identity, no sockets or filesystem writes.
  Do not mutate/open/create a vault before a successful bind and asset load.
- `SharedClientTemplate::load(Arc<ServerProfile>) -> Result<Arc<SharedClientTemplate>,String>`
  validates identity and decodes shared cache/interface/nav once, no game login.
  `.profile()` returns &Arc<ServerProfile>; `.world()` clones the nav Arc.
  `.prepare_client(uid, lowmem)` invokes actual host::prepare_client_with_profile
  and Client::from_shared_with_profile. Use this same path in construction tests.
- `run_with_template(template: Arc<SharedClientTemplate>, mainland: bool,
  profiles, per_slot, per_frame) -> Result<Play,String>` starts a checked Play.
  The empty-profile form can install your existing per-frame hook before slots.
  `Play::try_spawn_slot(...) -> Result<(),String>` reports refusal before mutation.
  Legacy run_with_io/PlayOptions are retained for old tests/callers only.
- ServerProfile getters: client(), revision(), target(), selection(), nav_pack(),
  nav_flags(), nav_availability(), content_dir(), vault_path(), catalog(), label().
  Catalog identity exposes root:PathBuf and sha256:String. validate_catalog()
  refuses changed bound inputs. prepare template before vault mutation and reuse
  it across lock/unlock; do not resolve a new active profile silently.
- Selection and profile both have `require_bot_operation() -> Result<(),String>`.
  274 passes; 289 returns host-boundary-not-qualified until step 4. Keep bound
  289 construction testable; normal login/script/guardian operation must refuse
  visibly before raw 274 sends or automatic policy can run. Do not disable or
  dim catalog cards. A constructor test is not 289 gameplay acceptance.

## Behavior requirements

1. Consistent --revision 274|289 and --profile local-274|local-289|public-274
   in panel, TUI and host-play. --prod aliases known public 274. Parse all flags
   before resolution so --prod cannot erase explicit overrides based on order.
   Conflicting named profile/revision, unsupported revisions and invalid ports
   must fail before mutation or connection. CLI help documents the new inputs.
2. Panel startup exposes revision before sessions start; persist only one new
   revision field in PanelUiState, default/migration 274. Existing UI prefs stay.
   CLI/environment override saved revision. Changing a bound process revision
   requires restart even after vault lock. An explicit CLI revision/profile
   cannot be silently replaced by a saved or clicked setting. No last-script
   persistence. Show selected server and active revision in panel and TUI.
3. Use checked template/profile on actual production startup, slot creation,
   login/reconnect, and the live/memory entry paths. Preserve legacy empty-cache
   test constructors by an explicit legacy path if needed, not a production
   bypass. Tests should traverse frontend selection -> template -> real client.
4. Freeze target-sensitive vault/passphrase/minted-password/fixture UI decisions
   using *_for_target helpers or profile.target(), not later bot_target(). Keep
   vault contents/account settings separate from server profile. Distinct289
   vault path must be used for create/unlock/reset/existence checks.
5. Catalog fill uses the bound/selected root; no later RS2B0T env reread choosing
   another source. Keep Load custom script support, existing dim/import-blocked
   behavior, card set, settings storage and source lifecycle. If catalog input
   changes after binding, require restart rather than pretend identity matches.
6. Bind panel picker navflags to selected profile's nav_flags path. Existing
   static flags loader re-reads NAV_FLAGS/default_pack_path: production must use
   the bound path. Keep Arc world sharing and paint behavior. Scatter preparation
   must consume the selected Play world rather than silently loading default274
   nav; root can add a shared scatter helper if required (report exact seam).
7. Keep current Start/Pause/Resume/Stop/focus/render ownership and timeouts.
   Native macOS smoke/visual verification is root's final integrated work; do not
   launch live gameplay or wait for Linux/Windows. Operator explicitly says
   Linux/Windows 289 engines need setup later, after macOS step 3 validation.

## Verification and handoff

Use meaningful existing/added parser and startup tests for default/explicit274,
289, conflicts, saved-preference precedence, identity/immutability and before-
mutation refusal. Test actual host/client construction with pinned local assets
or a scoped fixture, not only struct getters. Root will supply a reusable fixture
helper if needed; do not depend on absent external assets and report a skip as pass.
Run affected frontend tests (with memory-profile when changed) and fmt/check.
Compiler cache may reuse /Users/acfrazier/experiments/274bot/target. Client API
is being implemented concurrently; missing client::session initially is a known
build dependency. Do useful code/test work, then wait for those files if needed.
Do not duplicate backend/profile implementations to bypass it.

Commit ONLY allowed host files and your report/evidence; root owns shared host
files, STATE and the client gitlink. Call kanban_request_review on THIS SAME CARD
with profile reviewer and evidence, then STOP. No extra delegate review or
subagents, no main, no remote push/merge. Root monitors actual review completion.
