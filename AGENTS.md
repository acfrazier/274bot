# Agent rules — 274bot host

Read this file once. Do **not** search the disk for another `AGENTS.md`.

**What this is:** Rust bot host for the 274 client. GitHub: `acfrazier/274bot` (public). Client is a **submodule** at `vendor/fr-client-rust` (`acfrazier/FR-client-bothost` `r274-bothost`). Attribution: `NOTICE.md`. Specs/plans: gitignored `docs/superpowers/` on the FR clone, not this repo.

**Client fork:** Patch `FR-client-bothost` `r274-bothost` (instrumentation, skip-paint). **Do not** push `Fairy-Ring/FR-client-rust`. Do not add a bot action API inside `client`. Wiring `client` compiles the **lib**; `cargo test` here does not run FR integration tests. Do not put 274bot crates in the client repo.

**Layout:** crates under `crates/{host,vault,api,host-play,panel,nav,e2e}` (`panel` is this campaign's native UI).

**Scope:** the **wall** is **in-scope** this campaign — MultiBox sidecar
rail, grid mode, profile chooser (rail/grid/chooser). **Nav** remains
in-scope (pack bake, router/traveller, WalkTo picker, live door harness).
Still no dummy tick-end opcode, no scripts, no GPU 3D in this repo
(CpuPix3D holds for the wall; wgpu if ever is last on bothost — 50
full-rate GPU clients as a tech demo — not Fairy-Ring).

**SDD models (operator):** task implementer `deepseek-v4-flash`, per-task reviewer `deepseek-v4-pro` (or flash if it behaves), **whole-branch review: grok**. Do not skip the final grok pass. Repo hygiene (remotes, force-push, submodules) is **orch inline**, not subagent-driven.

**Do:** TDD as the task brief. One task only. `cargo test -p <crate>`. Commit on `main`. Write the report file the orch named.

**Do not:** invent a tick-end opcode; deep-copy the world every read; skip the brief; wander into scripts; hunt for a longer AGENTS.md.

**Live:** automated harnesses in `crates/e2e` (`LIVE=1 cargo test -p e2e -- --ignored`). FAIL + exit 1. Wait `ingame && scene_state==2`. Not Playwright. Verbose only if `BOT_DEBUG=1`. Do not skip the live task.
