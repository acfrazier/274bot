# Correct the public server profile to revision 289

Operator correction on 2026-09-10: rs2b2t moved to revision 289. Public 289
requires the known `w1.rs2b2t.com:443` game/asset pairing; public 274 is unavailable.
This supersedes the opposite assumption in the step-3 design and receipts.
Those old receipts remain historical evidence. The local 274/289 proof stands.

Use configured `sol` defaults. Read AGENTS and docs/execution once, the current
profile implementation, and this brief. Work on the existing host branch
`codex/rs2b0t-multirevision` in this campaign checkout. Base is `db9b741a`.

Implement the correction in `crates/host-play/src/profile.rs` and appropriate
existing profile/frontend tests and CLI help strings. Update the current
`docs/compat/01-session-profile.md` support description with an explicit dated
correction. Root owns STATE, machine inventory, step-4 work, client gitlink,
native/live runs and public integration. Do not edit frozen review reports,
evidence/manifests or old task briefs to pretend they used the corrected rule.

Contract:

- `public-289` resolves to Prod/R289 and exactly the known game+asset host/443
  pairing. `public-274` and explicit public revision 274 requests fail before
  assets, vault or network. Local defaults stay 274; local 289 stays isolated.
- `--prod` without an explicit CLI/environment revision uses the known public
  revision 289. A saved local 274 preference must not make this default fail.
  Explicit CLI/environment/named revisions retain precedence and a public 274
  conflict must fail clearly. An explicit named CLI `public-289` overrides lower
  priority environment/saved selections as before. Add meaningful cases for
  named flags, --prod, environment target, saved 274, explicit 274 conflicts,
  game/asset host and port drift, and unchanged local defaults.
- All 289 resource defaults (unpack/cache/nav/content) derive from revision,
  including public 289. Keep the existing `vault-prod` public account path;
  preserve explicit paths. Do not silently accept an old 274 cache or nav pack.
- Retain the existing checked RSA/cache/CRC identity requirements and shared
  client bindings. Do not invent or download a new public RSA/cache identity.
  Missing public assets stay an explicit setup requirement; no public login or
  gameplay qualification is authorized by this correction.
- Keep the 289 host-operation gate in place until step 4 qualifies the direct
  host writers. Tests for public fixture-cheat refusal must still exercise the
  target guard (not merely pass because the 289 gate rejected everything).

Run focused existing profile/frontend tests that cover the change, affected
compile checks and formatting. Avoid repeating full client/GPU suites. Use
`CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target`; store raw receipts
under `docs/compat/evidence/public-profile-correction/`. Commit only your scoped
changes, hand this SAME card to profile `reviewer` via `kanban_request_review`,
then stop. No client source edits, remotes, main, live engines or subagents.
