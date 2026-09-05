# Agent rules — 274bot host

Read applicable instructions once. Do not search unrelated worktrees or archives for alternative instructions.

**What this is:** Rust bot host for the 274 client. GitHub: `acfrazier/274bot` (public). Client is a **submodule** at `vendor/fr-client-rust` (`acfrazier/FR-client-bothost` `r274-bh-modular`). Attribution: `NOTICE.md`. Specs/plans: gitignored `docs/superpowers/` **in this checkout** (not Fairy-Ring, not GitHub).

**Resume:** for campaign work, read `docs/superpowers/STATE.md` once and follow the named current plan. Stop browsing the archive, then continue the task. Treat dated status as a snapshot; current code and operator instructions take precedence. Give implementers the relevant plan, not the entire state file.

**Client fork:** Patch `FR-client-bothost` `r274-bh-modular` (instrumentation, skip-paint, wgpu). `r274-modular` is the same refactor without bot-host hooks; `r274-bothost` is the pre-modular fork — do not push there. **Do not** push `Fairy-Ring/FR-client-rust`. Do not add a bot action API inside `client`. Wiring `client` compiles the **lib**; `cargo test` here does not run FR integration tests. Do not put 274bot crates in the client repo.

**Layout:** crates under `crates/{host,vault,api,host-play,panel,nav,script,scenario,e2e,tui}` (`panel` is the native UI; `tui` is `tui-play`).

**Scope:** finish the memory-efficiency campaign first, then resume the remaining JavaScript API compatibility plan. Compatibility maps the JavaScript surface onto Rust host APIs; do not recreate the foreign JavaScript runtime or its policy/routers. Missing host capabilities must be identified explicitly and implemented in Rust only within authorized scope. Preserve current behavior during memory work, including existing incomplete features and errors. No invented tick-end opcode.

**Rendering:** GPU 3D lives in the client submodule; `BOT_CPU=1` selects CpuPix3D. Preserve the last-FBO freeze while `scene_state==1`.

**SDD models (operator):** task implementer `deepseek-v4-flash` (live smoke that must read screenshots: `deepseek-v4-flash-vision-exp`), per-task reviewer `grok-4.5`, **whole-branch review: grok-4.6**. Do not skip the final grok pass. Repo hygiene (remotes, force-push, submodules) is **orch inline**, not subagent-driven.

**Git (this is the only copy of the rule — plans must not restate it):**
- **Flash / SDD implementer / spawned subagent:** forbidden on `main`. Run `git branch --show-current`. If it is `main`, **stop** and tell the orch. Commit only on the orch-named branch or a worktree (`isolation: worktree`). Never merge, never push remotes, never `checkout main`.
- **Orch / grok with the human on this checkout:** `main` only when the human said **inline**. Campaign SDD still uses a branch; orch merges.
- **origin:** GitHub `main` is this checkout’s commit history. Tag `0.1.0` is the squash that first went public; from `0.1.1` publishes are ordinary pushes + annotated tags. Do not squash-publish. Do not `git pull` the `0.1.0` squash onto local `main`. Force-push only when the human said **inline**.
Copied “commit on main” in a plan or a stale session snapshot is not consent.

**Verification:** follow the task’s regression requirements and run affected crate tests with required features. Run client integration tests separately when affected. Delegated implementers stay within their named task and write the requested report; the orchestrator may complete all authorized cross-crate work.

**Memory:** do not deep-copy the world on every read. Preserve ownership boundaries and verify savings with measurements.

**Live:** automated harnesses in `crates/e2e` and `crates/host-play` (`LIVE=1 cargo test -p e2e -- --ignored`; same for `-p host-play`). FAIL + exit 1. Wait `ingame && scene_state==2`. Not Playwright. Verbose only if `BOT_DEBUG=1`. Do not skip the live task.
