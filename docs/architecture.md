# Architecture

274bot is a Rust bot host over a vendored client library. Ownership is crate
and runtime boundaries, not a line-count budget and not a copy of a TypeScript
lint stack.

The Cargo workspace graph is **enforced**. Runtime semantics that a manifest
cannot prove stay **review requirements**. Changing a crate edge is a
deliberate policy edit plus documentation and review; do not sneak a reverse
or convenience dependency through an alias, target table, or optional rename.

## Local command

Python **3.11+** (stdlib `tomllib`). No Rust rebuild.

```bash
python3 tools/architecture/check.py
python3 tools/architecture/check.py --self-test
```

If `python3` is older than 3.11, use `python3.11` or newer. The checker reads
workspace and member `Cargo.toml` files, including `[dependencies]`,
`[dev-dependencies]`, `[build-dependencies]`, `[target.*.dependencies]`,
`optional`, `workspace = true` aliases, `path`, and `package` renames. It
does not parse `.rs` with regex and does not compile.

GitHub Actions runs the same two commands in the `architecture` job. That job
does **not** replace `fmt`, `clippy`, or `test`. Those gates stay as they are.

The explicit allowlist is [`tools/architecture/policy.toml`](../tools/architecture/policy.toml).
Every workspace member must appear there. Unknown members fail closed. crates.io
and other non-path crates are ignored. `client` is a workspace *alias* to
`vendor/fr-client-rust/crates/client`, not a 274bot workspace member; it is
listed as an architecture external.

## What this is adapted from

rs2b0t's strict CI on `origin/main` makes **test, typecheck, lint, prose, and
prose extras** mandatory. ESLint `no-restricted-imports` / restricted globals
encode layer ownership (adapter-only client internals, api must not import
scripts/UI, data/geometry stay leaves). Scripts such as `audit:exports`,
`check:api-contract`, and `audit:e2e-split` exist but are **manual** — they
are not CI gates.

This repo adapts the **mandatory ownership lint** idea to the actual Rust
crate graph. It does not copy ESLint, Vale, TypeScript commands, export
audits, API-contract dumps, e2e-split auditors, or comment/line-budget
punishment.

## Production crate ownership

| Crate | Owns | Must not become |
| --- | --- | --- |
| `vault` | Encrypted profile store | A dependant of api/host/ui |
| `api` | Host-facing read/act/settle types mapped onto `client` | A dependant of host, nav, script, or UI |
| `host` | Native slot/tick APIs, login FIFO, guardian | A dependant of nav, script, host-play, or UI |
| `nav` | Packed world, router, Traveller | Script isolate or operator UI |
| `script` | Compiled cards, Load isolate, thin JS shim | A production dependant of `client` or `nav` |
| `host-play` | Shared `Play` lifecycle over host/script/nav/vault | A second panel or client renderer |
| `scenario` | Shared headed/headless live scenario runner | Panel/TUI chrome |
| `panel` | Native ImGui UI, winit/wgpu window, game blit | The client 3D renderer or isolate runtime |
| `tui` | Headless operator view of the same `Play` | A second kernel or GPU loop |
| `e2e` | Wide **test orchestration** (library + suite binaries) | Production UI or script-kernel ownership |
| `client` (external) | 274/289 client lib, GPU/CPU raster, last-FBO | Bot action API or 274bot crates in its repo |

Authoritative edges and reasons live in the policy file. The current
intentional graph, derived from the manifests:

- `api` → `client`
- `host` → `api`, `client`, `vault`
- `nav` → `api`, `client`
- `script` → `api`, `vault` (runtime); `client`, `nav` **dev-only**
- `host-play` → `api`, `client`, `host`, `nav`, `script`, `vault`; `scenario` **optional**; `nav` also **build**
- `scenario` → `api`, `client`, `nav`
- `panel` → `api`, `client`, `host`, `host-play`, `nav`, `scenario`, `script`, `vault`
- `tui` → `api`, `client`, `host-play`, `nav`, `scenario`, `script`, `vault`; `host` **dev-only**
- `e2e` → `api`, `client`, `host`, `host-play`, `nav`, `scenario`, `vault`

`e2e` does **not** Cargo-depend on `script`, `panel`, or `tui`. The suite
launches product binaries as child processes. That is orchestration, not a
claim that `e2e` owns those crates' production behavior.

### Truthful exceptions (enforced as written)

- **`api` → `client`** is intentional. `api` maps host types onto the
  vendored client. It is not a reverse `client` → `api` edge.
- **`script` → `client` / `nav`** are `[dev-dependencies]` only. Promoting
  either to a normal/optional/target dependency fails the checker.
- **`tui` → `host`** is `[dev-dependencies]` only. Production TUI composes
  through `host-play`.
- **`host-play` → `scenario`** is optional (feature-gated harness), not a
  default required edge.
- **`host-play` → `nav`** is both a runtime dependency and a build-dependency
  (bake/stage nav identity). Both kinds are listed.

If a future change needs a new edge or kind, edit the policy and this page
and get review. Do not add a second hidden graph.

## Runtime ownership

These are product boundaries. The crate checker does not prove them.

| Boundary | Owner | Not the owner |
| --- | --- | --- |
| Operator window, ImGui chrome, MultiBox, game blit, input into slots | `panel` | client applet UI, script isolate |
| Headless operator view (raster Off) | `tui` | GPU renderer, a second `Play` |
| Session lifecycle: unlock vault, spawn/park slots, login FIFO, tick pump, script start/pause/stop/load as one transaction | `host-play` (`Play`) | panel/tui chrome, `e2e` |
| Native per-slot host APIs, snapshot/think, random-event guardian | `host` | nav internals, JS |
| Script kernel, isolate thread, shim coerce/marshal only | `script` | JS policy/routers, a foreign runtime |
| GPU 3D / CpuPix3D (`BOT_CPU=1`), packet/doAction Java shape | `client` | host bot-action API, 274bot crates in the client repo |

The JS shim must not grow its own API or shared policy. Compatibility maps
the JavaScript surface onto Rust host APIs.

## Behavioral invariants (review unless noted)

The checker does **not** pretend to prove the following. They remain
independent-review and test requirements unless a later, robust check exists.

- **Isolate ↔ host and game-action wire:** FlatBuffers only. No extra JS↔Rust
  host transport. Vault/settings/serde JSON elsewhere is unrelated and is
  not banned by word.
- **Typed synchronous Rust calculation helpers inside the isolate** are an
  authorized exception (operator 2026-09-18): bounded calculations with typed
  value marshalling. They are not permission for in-isolate game actions,
  extra host RPC, or JS policy.
- **No JS policy growth** in the shim. Fail-closed helpers and configuration
  stay in Rust.
- **Lifecycle ordering** (login, `ingame && scene_state == 2` live gates,
  start/pause/stop/reload, pair/external exclusivity) is behavioral. Review
  and live/harness tests own it.
- **Last-FBO freeze** while `scene_state == 1` is a client rendering
  invariant. Review and existing client tests own it; this graph check does
  not inspect renderer code.

## Tests versus production

Large test suites in `panel`, `host-play`, and `scenario` are a **layout**
concern. Moving inline test modules into sibling files does not decompose
production ownership and is not an architecture change. This checker does
not require those sibling paths to exist and must not be used as a
path-presence gate while that layout work lands.

`e2e` and ignored live tests in `host-play` are wide orchestration. Linking
those crates for tests is not a claim that test code is the product owner of
nav, rendering, or the isolate.

## Changing a boundary

1. Edit [`tools/architecture/policy.toml`](../tools/architecture/policy.toml)
   with the new or removed `[[allow]]` and a reason.
2. Update the tables on this page so the public description matches.
3. Run the local commands above (including `--self-test`).
4. Get review. Do not land a graph change as a drive-by import.
