# Revision preparation audit: 274 → 289 → later 377

**Operator/source update (2026-09-07):** The initial bounded inventory below missed an existing `vendor/client-java-289` worktree (6834c7255f559db5f1702b8b0e5e7286a7d61244), found by the root RAG query. The vault has now been renamed to `/Users/acfrazier/experiments/FR-vault`, with compatibility aliases and repaired Git links. Operator-supplied RuneWiki branch289 is installed separately at `research/deob/289`, pinned at0c00ef249546fada67b1f6eb8bbe01ea7c250c95. See the vault's `docs/research/deob-289-source-inventory.md` and complete provenance JSON. Source acquisition is complete; the next research step is comparison, not another download. The original missing-source statements below describe the superseded audit and must not be treated as current next actions.


Audit scope: read-only preparation on 2026-09-07. The active host branch is
`codex/memory-diagnostics`. No code, cache, index, memory, Git, network,
server, live, build, or test action was performed for this audit.

## Decision summary

A 274 → 289 hop is sensible only as an isolated compatibility/research lane,
not as a revision switch in the current host. The host currently has a
revision-shaped client API, but its packet table, cache layout, rendering
assumptions, login handshake, and lifecycle behavior are all coupled to the
274 client. The available local deob evidence is later-revision material; no
289 deob tree or 289 client archive was found in the bounded local inventory.

The later 377 work is a separate research horizon. Existing Fairy Ring notes
record 289 as a useful content-script ladder for some 377 quest trees, but
explicitly say not to treat 289 as behavioral truth for the 377 client. That is
content evidence, not a client-port prerequisite.

## Current 274 architecture (observed facts)

| Surface | Current fact | Primary anchor |
|---|---|---|
| Client boundary | `vendor/fr-client-rust` is the `acfrazier/FR-client-bothost` submodule, branch `r274-bh-modular`; the workspace depends on its `client` library by path. | `.gitmodules:1-4`; root `Cargo.toml:1-6`; `vendor/fr-client-rust/Cargo.toml:1-3` |
| Simulation state | `Client` owns connection/config, packet state, world/collision, entities, interfaces, mutable overlays, lifecycle flags, and packet-family generations. | `vendor/fr-client-rust/crates/client/src/client/client.rs:291-379,820-843` |
| Protocol | `handle_packet` dispatches the 274 `ServerProt` table; each applied packet bumps a family generation. `REBUILD_NORMAL` invalidates all families. | `.../client.rs:3617-3697` |
| Host tick edge | The host synthesizes the tick callback from a `PLAYER_INFO` generation edge; this is a current 274 host contract, not an invented cross-revision opcode. | `crates/host/src/slot.rs:20-24,60-74`; `vendor/.../client.rs:272-289` |
| Cache | `Client::new` reads the local cache and may unpack it; host construction uses `Client::from_shared` so immutable `Cache` and interface tables are unpacked once and shared by `Arc`. | `vendor/.../client.rs:300-320,851-895`; `crates/host/src/lib.rs:113-125` |
| Cache fetch | The client expects the 274 jag names/checksum flow and loads from `cache_dir`; `maininit` is separate from login. | `vendor/.../client.rs:194-205,1527-1535`; `vendor/.../client-play/src/main.rs:202-207` |
| Rendering | The applet is 765×503. Render state is held in a separate `Renderer`; the backend seam is CPU Pix3D/Pix2D with optional wgpu scene rendering and CPU fallback. | `vendor/.../client.rs:266-270`; `vendor/.../render/renderer.rs:47-68,193-280`; `vendor/.../render/backend/mod.rs:15-23,141-188` |
| Freeze behavior | GPU deliberately preserves the last 3D scene while `scene_state == 1`; this is a 274-compatible Java loading/freeze behavior that must be re-proven, not silently carried to 289. | `vendor/.../render/backend/mod.rs:171-175`; `vendor/.../render/backend/gpu.rs:594-595,1605-1613` |
| Account/login | A vault profile contains username, password, uid, and settings. The login queue models production address/device limits; login opens a fresh stream per attempt. | `crates/vault/src/lib.rs:101-108`; `docs/api/login.md:8-35,90-95` |
| RSA/world | Local login uses the engine key/default pair; target selection changes host/port/cache and secure transport behavior. | `vendor/.../bot_target.rs:41-48,74-101,130-143`; `docs/api/login.md:67-88` |
| Lifecycle | `maininit` precedes login; `logout` closes the stream, drops the on-demand socket, clears session/world/render-adjacent state, and bumps every generation. | `vendor/.../client-play/src/main.rs:202-207`; `vendor/.../client.rs:7979-8037` |
| Host scheduling | Each slot owns a client thread; active/render work is 20 ms, idle/stalled slots use event-driven parking and bounded wakeups. | `crates/host/src/lib.rs:128-205` |

The checked-out client submodule currently reports commit
`3456edc8dabf7b25ada78110ffa56327af9f67a4` on local branch
`codex/windows-native-parking`; the host branch remains the required
`codex/memory-diagnostics`. These are local checkout facts, not a proposed
submodule update.

## What separates a real 274 → 289 hop

These are prerequisites, not implementation claims:

1. **Pin primary 289 client evidence.** Obtain a reproducible 289 client
   archive/deob and record archive, loader, cache, and source hashes. The
   current bounded inventory found no `research/deob/289` and no 289 jar.
2. **Build a revision delta ledger before editing Rust.** Compare the 274 and
   289 login handshake (response codes, RSA/key selection, initial packet
   sequence), inbound/outbound opcode IDs and packet sizes, ISA/bit order,
   checksum/jag index names, archive/group conventions, and map/loc/config
   formats. Any item without 289 primary evidence stays UNKNOWN.
3. **Separate cache identity from revision identity.** A 289 cache must have
   its own root, manifest, unpack output, and content/config provenance; it
   must not reuse the 274 `Cache`, `IfType`, model/animation assumptions, or
   the current shared-`Arc` key without proving binary compatibility.
4. **Reconcile rendering at the seam.** Compare applet dimensions, title/game
   frame layout, scene build/height/visibility rules, texture/model/animation
   formats, and loading-state compositing. Re-prove the last-FBO freeze and
   CPU fallback behavior for 289 instead of assuming the 274 `scene_state`
   contract.
5. **Keep the host account model revision-neutral where possible, but verify
   boundaries.** Vault profiles, UID identity, FIFO/backoff, auto-login,
   logout latches, and snapshot generation plumbing can be retained as host
   concepts; handshake fields, response semantics, RSA source, and tutorial
   behavior require 289 evidence.
6. **Define lifecycle acceptance separately.** Prove title → init → login →
   scene-ready → disconnect/retry → logout for one 289 client before adding
   multi-slot sharing or renderer policies. The current 274 contract includes
   explicit stream/on-demand teardown and all-family invalidation that may not
   match 289.

### Later 377 is not the same hop

The local Fairy Ring corpus gives verified folder-level evidence: 274 has 65
quest folders, 289 has 69, and local 377 has 74. `quest_mm` and
`quest_routequest` are absent on 274 and are 289-source candidates, while
377-only stubs are not 289 ports. Those inventories are useful for content
planning, but do not establish 289 client protocol, cache, renderer, account,
or lifecycle behavior. Primary anchors:
`$LC377_ROOT/docs/research/corpus/inventory-274-vs-377-quest-delta.md:49-66,92-104`
and `$LC377_ROOT/docs/research/corpus/inventory-289-vs-377-quest-delta.md:35-43,47-71,75-86`.
The 377 stance explicitly says the Java 377 client is the behavioral oracle and
Client-TS 289 is not behavioral truth:
the vault `AGENTS.md:74-79` and `$LC377_ROOT/docs/research/client-strategy-377.md`.

## FR-vault rename audit

The requested source directory is a normal Git work tree, not a linked
worktree: `git-dir` and `git-common-dir` both report `.git`, and its current
branch is `main`. Its recorded remote is the private
`https://github.com/Fairy-Ring/FR-vault.git`; no remote action was taken.

Bounded findings:

- `research/deob/` already exists under the project. The bounded directory
  inventory contains `274`, `410`, `412`, `413`, `414`, `418`, `419`, and
  `468`, plus deob tooling; it does not contain `research/deob/289`. Placing a future 289 deob at
  `research/deob/289/` is consistent with the existing layout.
- `research/jars/` exists. There is no named `research/indices/` directory.
  The proposed `/research/deob` is therefore ambiguous and should not mean
  filesystem root `/research`; use the project-relative path
  `$RS2_R377_ROOT/research/deob` (or `$LC377_ROOT/research/deob`).
- The RAG implementation resolves its root from the script location and
  stores FTS paths relative to the project root. Its SQLite and local Chroma
  data are under `.rag/data`; the Chroma volume is relative to
  `.rag/docker-compose.yml`. A directory rename should not require rewriting
  indexed relative paths, but the running RAG process/container must be
  stopped and reopened after the move. Do not mutate or reindex as part of
  this audit. Anchors: `.rag/lib.mjs:7-12,25-36,51-72,123-151` and
  `.rag/docker-compose.yml:1-13`.
- The primary project contains machine-specific absolute path references in
  `PLAN.md` and the full `AGENTS.md`; operational docs otherwise prefer
  `$RS2_R377_ROOT` / `$LC377_ROOT`. The bounded 274bot search found no visible
  caller containing the exact FR-vault directory path or the proposed
  project path. This is not an exhaustive machine-wide safety claim.
- Git metadata, the `FR-vault` remote name, the RAG collection name
  `fr-vault`, and the repository's relative internal links are not themselves
  reasons to rewrite. Update only scripts/configs/launchers that hardcode the
  old absolute directory, plus operator shell environment and any external
  scheduler/index configuration found in a separate approved audit.

### Reversible migration plan (proposal only)

1. Freeze writers and RAG/index consumers; record `git status`, active
   processes, and the old/new path mapping without changing either tree.
2. Search only approved caller roots for the exact old path and environment
   variables; exclude credentials, SQLite/WAL, Chroma binaries, and unrelated
   worktrees. Classify each hit as executable/config, documentation, or
   historical evidence.
3. Prefer the new canonical name `FR-vault` and project-relative/env-based
   paths. Keep a temporary compatibility symlink from the old name to the new
   name only if an operator approves it; do not create it automatically.
4. Rename the directory with the operator's explicit approval, update the
   small classified set of actual callers, then reopen RAG from the new root
   and verify read-only FTS path resolution. Preserve the old→new mapping in
   the migration note.
5. Roll back by stopping consumers, restoring the original directory name and
   caller values, and reopening consumers. Do not delete or rebuild indexes as
   part of rollback.

## Recommended next 289 step

Acquire or locate one primary 289 client archive/deob and create a hash-pinned
`research/deob/289/` inventory containing only loader/client/cache provenance,
class names, and a first-pass protocol/cache/rendering/account/lifecycle delta
ledger. Do not begin compatibility implementation or copy foreign runtime
policy. If no 289 artifact can be sourced, record that as the blocking evidence
gap and use 274 plus 377 primary sources only for a hypotheses list.

## What can wait until memory work is complete

All compatibility implementation, client submodule changes, 289 cache
materialization, protocol adapters, renderer changes, host API expansion,
account/login behavior changes, and lifecycle rewrites can wait. The current
memory campaign still has open performance, target, latency, renderer,
Linux/panel, lifecycle, scaling, and required whole-branch review gates (see
`docs/memory/STATE.md:432-475` and its current later entries). This audit is
preparation only and does not alter that priority.
