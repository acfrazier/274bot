# Agent rules — 274bot host

Read this file once. Do **not** search the disk for another `AGENTS.md`.

**What this is:** Rust bot host for the 274 client. GitHub: `acfrazier/274bot` (public). Client is a **submodule** at `vendor/fr-client-rust` (`acfrazier/FR-client-bothost` `r274-bh-modular`). Attribution: `NOTICE.md`. Specs/plans: gitignored `docs/superpowers/` **in this checkout** (not Fairy-Ring, not GitHub).

**Client fork:** Patch `FR-client-bothost` `r274-bh-modular` (instrumentation, skip-paint, wgpu). `r274-modular` is the same refactor without bot-host hooks; `r274-bothost` is the pre-modular fork — do not push there. **Do not** push `Fairy-Ring/FR-client-rust`. Do not add a bot action API inside `client`. Wiring `client` compiles the **lib**; `cargo test` here does not run FR integration tests. Do not put 274bot crates in the client repo.

**Layout:** crates under `crates/{host,vault,api,host-play,panel,nav,e2e}` (`panel` is this campaign's native UI).

**Scope:** the **wall** is **in-scope** this campaign — MultiBox sidecar
rail, grid mode, profile chooser (rail/grid/chooser). **Nav** remains
in-scope (pack bake, router/traveller, WalkTo picker, live door harness).
Still no dummy tick-end opcode. GPU 3D (wgpu) lives in the client
submodule (headed default; `BOT_CPU=1` is CpuPix3D). Compiled scripts
and WalkTo are in-tree; do not invent a tick-end opcode or put 274bot
crates in the client repo.

**SDD models (operator):** task implementer `deepseek-v4-flash` (live smoke that must read screenshots: `deepseek-v4-flash-vision-exp`), per-task reviewer `grok-4.5`, **whole-branch review: grok-4.6**. Do not skip the final grok pass. Repo hygiene (remotes, force-push, submodules) is **orch inline**, not subagent-driven.

**Do:** TDD as the task brief. One task only. `cargo test -p <crate>`. Work on a **branch** (or git worktree). Do **not** commit campaign work on `main`. Write the report file the orch named.

**Do not:** invent a tick-end opcode; deep-copy the world every read; skip the brief; wander into scripts; hunt for a longer AGENTS.md.

**Live:** automated harnesses in `crates/e2e` (`LIVE=1 cargo test -p e2e -- --ignored`). FAIL + exit 1. Wait `ingame && scene_state==2`. Not Playwright. Verbose only if `BOT_DEBUG=1`. Do not skip the live task.
