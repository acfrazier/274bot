# Agent rules — 274bot host

Read this file once. Do **not** search the disk for another `AGENTS.md`.

**What this is:** Rust bot host for the 274 client. Local: `/Users/acfrazier/experiments/274bot-host`. GitHub: `acfrazier/274bot` (private). Spec: `docs/superpowers/specs/2026-08-22-bot-host-kernel-design.md` on the FR clone (gitignored). Plan: `docs/superpowers/plans/2026-08-22-bot-host-kernel.md` there.

**Not this repo:** Fairy-Ring/FR-client-rust (`/Users/acfrazier/experiments/274bot`). Do not put bot crates there. Client is a **submodule** at `vendor/fr-client-rust` (path dep). Wiring `client` compiles the **lib**; `cargo test` here does not run FR integration tests.

**Layout:** crates under `crates/{host,vault,api,host-play}`.

**Do:** TDD as the task brief. One task only. `cargo test -p <crate>`. Commit on `main`. Write the report file the orch named.

**Do not:** invent a tick-end opcode; deep-copy the world every read; skip the brief; wander into nav/panel/scripts; rewrite the FR clone; hunt for a longer AGENTS.md.

**Live:** automated harnesses in `crates/e2e` (`LIVE=1 cargo test -p e2e -- --ignored`). FAIL + exit 1. Wait `ingame && scene_state==2`. Not Playwright. Verbose only if `BOT_DEBUG=1`. Do not skip the live task.
