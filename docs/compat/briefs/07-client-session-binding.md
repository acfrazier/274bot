# Step 3 client connection/resource binding

Bounded implementation assignment, client only. Host root handles the host
profile and frontend wiring concurrently. Read host AGENTS.md, docs/execution.md,
06-session-profile-design.md and its accepted Grok 4.6 design review. Accepted
client base: 58120f28ee5208553ca07f41cb364f2cf98ea280. Work only in campaign
vendor/fr-client-rust on codex/bothost-274-289; root owns the host gitlink.

## Public API contract shared with root's host implementation

Add `client::session::{ClientSessionProfile, ClientSessionConfig}` (reexport at
crate root is welcome). Config is an owned builder input with public fields:

- revision: client::io::ClientRevision
- target: client::BotTarget
- game_host: String, game_port: u16
- asset_host: String, asset_port: u16
- cache_dir: PathBuf, unpack_dir: PathBuf
- rsa_modulus: String, rsa_exponent: String (public decimal RSA values only)
- expected_crc: Option<[i32; 9]>
- content_id: String (opaque cache-content identity, host computes it)

`ClientSessionProfile::new(config) -> Result<Self, String>` validates inputs,
parses the public RSA once, and stores immutable private fields. Reject bad
RSA rather than the legacy fallback on this explicit path. Provide read-only
getters named after the fields; revision/target/ports/expected_crc return Copy
values, string getters return &str, paths return &Path. Provide
`client_config(&self, members: bool, lowmem: bool) -> ClientConfig`.
A method returning the public RSA pair by reference is fine for login internals.

`Client::from_shared_with_profile(config, cache, ifaces, ifaces_mut, profile:
Arc<ClientSessionProfile>) -> Result<Client, String>` is the real shared
constructor. Validate redundant config host/port/cache_dir match the profile
BEFORE OnDemand or other effects. Keep per-slot members/lowmem in ClientConfig.
Profile must be attached BEFORE construct initializes OnDemand. Add
`Client::session_profile() -> Option<&Arc<ClientSessionProfile>>` and
`Client::session_target() -> BotTarget` (legacy delegates to current target).
Root will add host::prepare_client_with_profile as the same thin wrapper plus
login_uid. Do not change this API contract silently; record a required API
adjustment in the report/card if unavoidable.

Expose reusable bounded-transport helpers so host does not duplicate HTTP:
`Client::get_jag_checksums_for(target: BotTarget, host: &str, port: u16)
 -> Result<[i32;9], &'static str>` and
`Client::get_jag_file_for(target: BotTarget, cache_dir: &str, host: &str,
 port: u16, filename: &str, index: usize, checksums: &[i32;9])
 -> Option<Vec<u8>>`.
The existing wrappers keep current ambient behavior. Explicit HTTPS/WSS must
honor the supplied port; existing standalone wrappers still choose their
existing defaults. Maintain existing connection/read/retry timeouts.

## Actual behavior requirements

- Bound login and reconnect use frozen revision, target, game endpoint, public
  RSA, CRC and cache/unpack input. Changing environment or public ClientConfig
  connection fields later must not redirect them. Preserve the step-2 frame
  limit correction and all existing login error/timeouts/adoption behavior for
  legacy clients.
- Bound HTTP must use the asset endpoint/transport during actual maininit,
  never the game endpoint or ambient target. If expected_crc is Some, enforce
  the frozen checksum identity; a changed server CRC must not overwrite the
  shared cache/interface contract. Explicit error/loading state is required.
- OnDemand shares one worker for matching bound resources and rejects
  mismatched target/revision/cache/version-table identities before joining an
  occupied endpoint (including a bound/legacy conflict). Propagate an explicit
  constructor error; do not add a mixed-world worker scheduler or silently
  turn a refused OnDemand into absent optional data. Compare actual version/CRC
  tables as well as declared content identity. It must reconnect with its
  captured transport. Preserve subscriber accounting, last-subscriber shutdown
  and existing legacy behavior. Avoid a mixed-world global-resource scheduler.
- Successful bound socket adoption requires compatible complete identities;
  reject a bound/legacy or mismatched-bound handoff BEFORE taking any stream
  or changing revision/buffers. Matching adoption and all legacy adoption keep
  their prior lifecycle semantics.
- Snapshots/OnDemand/JagFX/texture reads must use frozen roots on bound clients.
  Audit mutable ClientConfig accesses in the actual client lifecycle. Do not
  recreate a cache, deep-copy world data, or add any host/script dependency.

## Proof / completion

Use real HTTP/socket tests for bound endpoint/transport/RSA/revision and
reconnect, not only getter assertions. Cover incompatible adoption leaving
both clients intact and matching-profile Arc sharing, OnDemand mismatches and
last-subscriber behavior, and bad explicit RSA. Preserve standalone tests.
Run focused affected client suites plus format/strict all-target Clippy; root
will run the coherent final workspace gate after integration. Compiler cache:
CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/.worktrees/rs2b0t-multirevision/target/client.
Do not set SKIP_GPU globally. Record exact commands, results and commit in
`docs/revision-289/session-profile-client.md`, with concise raw receipts in
`docs/revision-289/evidence/session-profile-client/`. No live gameplay or
performance claims. Do not rewrite historical evidence.

Commit only scoped client files. Request same-card review with
kanban_request_review(reviewer="reviewer"), include the commit and evidence,
then STOP. No extra delegate review, no further subagents, no host edits,
no merge or remote push. Root monitors through actual Grok 4.5 completion.
