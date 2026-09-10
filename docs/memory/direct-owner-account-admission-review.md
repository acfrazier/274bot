# Direct-owner account identity timing review

Task `t_d6f23813`. Independent grok46 architecture/fidelity review of the
root question at `90c716d`. Not implementation, not same-card controller
review, not live authorization.

Writes are this report and
`diagnostics/direct-owner-managed-extension/account-admission-review/`.
No source, controller, test, plan, or STATE edits. No native, SSH, live,
private-account, or cache reads. No capture or new harness.

## Verdict

No faithful prelaunch admission of an actual individual account identity
exists under the current plan, 6f2b410 controller, and frozen diagnostic
source.

The unchanged N1 account-selection recipe can be known before `Popen`.
The actual minted username cannot. Typed pre-`Popen` account evidence
accepts an opaque SHA plus slot counts and cannot establish a future
name, a preexisting save, or a logged-in session. An opaque SHA is not
proof of either identity.

Root must choose one behavior-preserving clarification before any live
release. This card does not implement or authorize a redesign.

## What was inspected

- `docs/execution.md`
- Question `docs/memory/direct-owner-account-admission-question.md` @ `90c716d`
- Plan `docs/memory/direct-per-bot-owner-capture-plan.md` §5 item 3
- Controller blobs at `6f2b410` (identical on this branch HEAD):
  `run_current_tui_calibration.py` `4533856a`,
  `run_managed_cell.py` `2c00e549`,
  `run_diagnostic.py` `8750f57c`
- Admission correction contract
  `docs/memory/direct-owner-admission-correction-report.md`
- Design `docs/memory/direct-owner-managed-extension-design.md` §6–7
- Frozen production diagnostic archive
  `diagnostics/direct-owner-native-preparation/root-runtime-preparation-01/frozen-direct-owner-source.tar.gz`
  SHA-256 `2c36d254c37c5ed56a55f78728357296d62be602f8653ed9ec041cd37d099d5d`
  (matches root-runtime-manifest-02 source lineage)
- Local source-only probe
  `diagnostics/direct-owner-managed-extension/account-admission-review/probe-result.json`

Git object `dcdbeebf` is absent here, as warned. Moving
`crates/host-play/src/{lib,memory}.rs` hashes
`096753fc…` / `74b7a181…` do not equal the frozen members
`f997f22f…` / `76e633ac…`. This review uses the tar, not checkout
host-play.

## Two identities

Fixture recipe identity is the unchanged N1 selection/seed contract:

- `BOT_MEMORY_N=1`, `BOT_MEMORY_WORKLOAD=active`, `BOT_MEMORY_SUSTAIN=1`
- `tui-play` launched as `Popen([binary])` with no `--user` / `--vault`
- `memory::Run::prepare_unseeded` calls `mint_live_names(n)`
- throwaway vault under a temp dir named from `names[0]`
- profile `uid = 274_900_000 + i`, `password = name`, `auto_login = true`
- Thiever card + seed runners after Play exists
- local `BOT_TARGET=local`, mainland true on the TUI memory boot

That recipe is established as soon as the frozen binary and 6f2b410
child env/argv are admitted. It does not name a person.

Actual individual account identity is the concrete minted username
(`live<token>_0` for N=1) and the save that auto-register creates for
it. That value is a function of the frontend process id and a
process-local serial (`host-play` `lib.rs` 64–82). It does not exist
before the child runs `prepare_unseeded`.

Plan §5.3 asks root to reuse that N1 contract and also to bind an
actual existing disposable account/population/settings at release.
Those two sentences do not currently coincide in time.

## Current diagnostic path (frozen tar)

`run_diagnostic.spawn_direct_unix` execs only the admitted binary
(excerpt `excerpt-popen-argv.py.txt`). `build_child_env` sets
`BOT_MEMORY_N` and related flags; it does not pass a username
(`excerpt-child-env.py.txt`). Calibration `clean_environment` does the
same (`excerpt-clean-environment.py.txt`).

Frozen `tui/src/bin.rs` 1178–1196: when `memory::Config::from_env()`
is `Some`, TUI prepares the memory run, unlocks that throwaway vault,
binds seeds from `Play::world`, spawns those minted names, and
returns. The interactive `--user` / operator-vault path at 1208–1240
is not reached on this diagnostic.

Frozen `memory.rs` 735–763: names come only from `mint_live_names`.
Vault is `Vault::create` of a new temp file, not an existing operator
vault. Probe found no `BOT_MEMORY_NAMES`, `BOT_ACCOUNT`,
`BOT_LIVE_NAMES`, or `BOT_USER` override in frozen lib/memory/tui.

`--user` exists on interactive TUI and host-play CLI. Using it for
this cell would require dropping or bypassing `BOT_MEMORY_N` memory
setup. That is a selection-policy change, not a supported diagnostic
route.

`--live` also mints names (`tui-bin.rs` 581). It is not the N1
controller path.

## What pre-Popen admission actually checks

`validate_direct_admissions` account evidence is exactly
`fixture_binding_sha256`, `slot_count=1`, `active_slot_count=1`
(`run_managed_cell.py` 885–895). Population is SHA plus
`requested_n=admitted_n=1` and `workload=active`. No username,
vault path, uid, settings body, or login flag.

The shared context copies the opaque SHA and independently measured
build/cache/server identities (`_direct_context` 707–726). The
controller requires SHA equality; it does not interpret the SHA.

Admission correction text states the SHA is a root-selected opaque
fixture identity repeated without exposing account name, password,
vault record, or cache payload. That is recipe-or-private-token
binding, not observation of an individual account.

Slot counts in that receipt are not `samples.jsonl` `ready`/`active`
and are not `ingame`. Treating them as logged-in would be exactly
the fabrication the question forbids.

## When each identity can be established

| Identity | Earliest honest moment | Supported public evidence |
|---|---|---|
| Recipe (N1 mint/throwaway/auto_login/Thiever/local) | Before `Popen`, once frozen source + 6f2b410 env/argv + build/cache/server receipts are admitted | Opaque SHA may privately denote this recipe; controller only checks SHA equality and counts |
| Frontend PID | Spawn handoff, after `Popen` returns | `frontend-handoff.json` `spawn.frontend_pid` / start identity. No username field |
| Minted local vault profile | Inside child `prepare_unseeded`, after mint | Not exported on handoff. Temp vault is not a public receipt |
| Actual username string | First qualification boundary that lists `slots[].name` | `samples.qualification.jsonl` observe-start/end. Name is the minted username used as status key |
| Logged-in / scene-ready | After handshake, when qualification `client.ingame` and `scene_state` are observed | Schema has those fields. Plan §5 item 5 requires `ingame && scene_state==2`. Default `cpu_screen` active path instead requires `Running` plus steal gain keyed by `slots[].name`; it does not itself prove login from slot counts |
| Owner-capture rows | After capture requests | `slot_token` / request/frame ids. No username. No supported join from SHA or handoff PID onto an account name |

Computing `live{pid_hex}{serial}_0` from the handoff PID is
reconstruction of `mint_live_names`, not a controller admission
path. Serial is a process-local atomic; assuming `0` is still a
guess. This review does not treat that reconstruction as
corroboration.

There is no supported linkage from `fixture_binding_sha256` to
`slots[].name`. Inventing one would be a protocol addition.

## Plan vs controller vs frozen source

Plan §5.3: reuse N1 account-selection/seed contract; bind account
identity privately; do not copy passwords/names into the owner
ledger; bind actual existing disposable account/population/settings
and server identity at release; old PID 726 is not a live identity.

Frozen N1 contract creates a fresh auto-registered save from a
PID-derived name after frontend start. A character that already
exists at release is not what this path selects. Reusing a previous
run's minted name would be the PID-726 class of stale identity.

Controller 6f2b410 can bind server/cache/build at release. It cannot
bind an actual account name at release without either guessing the
future PID/name or changing selection.

## Smallest required operator choice

Root must pick one. Neither option is implemented here.

1. Clarify plan §5.3 "actual existing disposable account at release"
   as recipe identity only. Prelaunch account/population receipts
   stay SHA + N=1 counts, meaning one minted auto-register fixture
   will be created under the frozen recipe, not that a named save
   already exists or is logged in. Actual username is first public
   in qualification `slots[].name`. Capture acceptance already
   depends on qualification; do not add a SHA-to-name join. Do not
   change minting, argv, vault, or seed policy. Live remains
   unissued until root records this reading.

2. Keep the literal "preexisting named disposable account at
   release" requirement. Then current frozen diagnostic + 6f2b410
   admission cannot honestly issue that receipt. Live stays blocked
   until a later authorized design changes selection. Using
   `--user`, the operator vault, or a predicted `live…` name is not
   a faithful current route.

Behavior-preserving boundaries for both choices: no mint/seed/env
change, no public password/name in owner ledger, no pretence that
slot count equals login, no SHA-as-account-proof, no capture retry,
no CF2 interaction.

## What this does not approve

- Live owner release, account minting, or private receipt issuance
- Changing `mint_live_names`, TUI argv, or vault policy
- Treating `fixture_binding_sha256` or slot counts as a logged-in
  account
- Equating moving checkout host-play with frozen diagnostic source
- Navigation CF2, performance acceptance, or workload qualification
  of a run that has not happened

## Verdict line

No faithful prelaunch actual-account route on the current
plan/controller/frozen source. Recipe identity is available now;
individual account identity is not. Root chooses clarification (1)
or keeps live blocked (2) before any release.
