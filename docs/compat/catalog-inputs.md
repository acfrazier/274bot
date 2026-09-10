# Frozen rs2b0t catalog inputs

This inventory freezes the two read-only rs2b0t source archives used by the
0.1.7 compatibility campaign. It does not modify or execute either archive.
`support-matrix.json` is the machine-readable contract. Imports and startup are
not functional support: every enabled source/revision row remains `PENDING` or
`BLOCKED` until the Rust-owned capability path and script-caused observation are
proved.

## Reproduce the loader inventory

Run from the campaign checkout on branch `codex/rs2b0t-multirevision`:

    CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target \
      cargo run -q -p script --example catalog_inventory -- \
      "$PWD/.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775" \
      > /tmp/catalog-100adccc.json

    CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target \
      cargo run -q -p script --example catalog_inventory -- \
      "$PWD/.superpowers/inputs/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a" \
      > /tmp/catalog-8e7d965b.json

The example calls `JsLibrary::register_rs2b0t`; classification therefore comes
from the real registry/import/settings loader, not a filename or name scan. It
also parses registry declarations through `parse_registry` only to account for
entries intentionally omitted by the loader (native `WalkTo` and dim
`Woodcutter`). It emits each loaded card's display name, relative source path,
source SHA-256, loader settings schema, direct imported API bindings, foreign
helper imports, loader refusal, and classification. Its scratch store/cache are
under the system temporary directory; the source roots remain untouched.

The target directory above is an existing functional build cache. It is not
performance evidence.

## Frozen identities and counts

Both registry files have SHA-256
`b614aedb3d5adfdfbac3760f082273f3cf5c33fffe4ced0f2b01490ae02d1bc2`
and 60 declarations. For each archive, the real loader returns 55 cards:

| Source identity | Read-only path | Enabled | Dim | Import-blocked | Shape-omitted | Native reserved |
|---|---|---:|---:|---:|---:|---:|
| `100adccc037d9f6898080e1cad58fcfc43364775` | `.superpowers/inputs/rs2b0t-100adccc037d9f6898080e1cad58fcfc43364775` | 45 | 9 | 2 | 3 | 1 |
| `8e7d965be2071d6ec65c3265e12af797082d720a` | `.superpowers/inputs/rs2b0t-8e7d965be2071d6ec65c3265e12af797082d720a` | 45 | 9 | 2 | 3 | 1 |

The 45-name enabled set is identical across the two sources and is frozen in
`support-matrix.json`. That file expands it to 180 explicit rows: 45 cards × 2
catalog sources × revisions 274 and 289. Every row carries its own source hash,
settings/branch contract, imported API members, required Rust capability
families, fixture identity reference, observable proof, known gaps and status.
No row is marked supported by this inventory.

Dim/deferred names are `AIOQuester`, `ArravSupplier`, `Barcrawl`, `Woodcutter`,
`Miner`, `Fisher`, `RoguesPurse`, `MarketMaker`, and `ClueSolver`. `Woodcutter`
is a real registry declaration but its entry module is not a loader bot shape;
it remains visible and dim rather than disappearing from the ledger. The native
`WalkTo` declaration is reserved by `JsLibrary`; host navigation owns the card.

Three additional registry declarations are not enabled because their entry
modules do not match a `JsLibrary` bot shape: `CowKiller`,
`EdgevilleMonkeyBars`, and `ShopRunner`. They are frozen separately as
`registry_omitted_entries`; this inventory does not silently promote or discard
them.

Import-blocked names are:

- `BrimhavenMossGiants` — `BLOCKED: missing native navigation mapping`; source
  imports `../../event/webwalk/Navigator.js`.
- `EssMiner` — `BLOCKED: missing host-owned tool acquisition mapping`; source
  imports `../../api/acquisition/ToolAcquire.js`.

Those entries are not counted in the enabled set, and the missing imports are
not replaced with foreign routing or acquisition planners.

## Source-version differences

Only four files under `src/bot/scripts` differ between the archives:
`BankFletcher.ts`, `BankFletcherLogic.ts`, `Superheater.ts`, and
`SuperheaterLogic.ts`.

`BankFletcher` entry hashes:

- `100adccc`: `194eb82dd3613cf2fea77a5cb04fba1dadbacad78efa6e0eee16c1f4a1c3f3ee`
- `8e7d965b`: `48a9f13b774931cd2ea5298cf625c480f071713d459ade293c1caccfb383aa2d`

The later source adds `mode=auto|cut|string|cut+string`, implements the combined
cut-then-string flow, changes bank opening to
`Banking.open({stand, boothName, boothOp})`, and removes `leashRadius`. The
baseline remains product-driven and uses direct booth/nearest open calls. Both
contracts remain separate; later implementation may not collapse them to a
shared reduced option set.

`Superheater` entry hashes:

- `100adccc`: `9188208c035121358cf6b72a176c34cbc50336509b8143282be2899d46d49089`
- `8e7d965b`: `4c4ea711e2f71dfaea8be05366bbbd0779d4be13cd6989702472e765d8d022ee`

The later source accepts and selects multiple fire-rune staff identities,
checks all alternatives before declaring the bank empty, and waits for a
`Wield` action after closing the bank. Qualification must exercise every
accepted alternative applicable to the selected content identity; supporting
only `Fire staff` is not sufficient.

## Idle-helper and deferred-behavior audit

The host shim currently preserves harmless construction but not behavior:

- `PeriodicBank` is constructed by `ArdyFighter`, `ChickenKiller`,
  `ArdyThiever`, and `RockCrab`. In
  `crates/script/src/shim/periodic_bank.js`, `validate()` always returns false
  and `execute()` is empty. Every strategy other than `Off` is therefore
  `BLOCKED: missing periodic bank execution`; startup is not proof.
- `DeathRecovery` is constructed by enabled `ArdyFighter`, `ArdyCakes`,
  `ArdyThiever`, `HillGiant`, `AutoFighter`, `MossGiant`, `ChickenKiller`,
  `WildyAgility`, `GreenDragon`, and `RockCrab`, plus import-blocked
  `BrimhavenMossGiants` and dim `RoguesPurse`. In
  `crates/script/src/shim/death_recovery.js`, `validate()` always returns false
  and `execute()` is empty. Recovery is `BLOCKED: missing death recovery
  execution` until Rust owns and proves death detection, reacquisition and
  return.
- Native quester/gatherer/clue/MarketMaker rewrites remain deferred. The dim
  cards above stay visible. Enabled `GreenDragon`, `ArdyFighter`, `AutoFighter`,
  `ArdyThiever`, `ArdyCakes`, and `RockCrab` construct `SolveClue`; the current
  shim always idles. Their clue-enabled branches are `BLOCKED: missing native
  clue solving` and must receive a clear pre-Start refusal rather than silently
  no-op. Core non-clue behavior remains pending independent qualification.

Foreign helpers listed in each row describe source-side implementation and are
not host capabilities. `required_rust_capabilities` lists the host-owned facts
and operations the row must obtain through Rust APIs. No foreign runtime,
router, navigation, banking, loadout, acquisition, recovery, quest, clue,
gatherer or market-making planner is authorized by this inventory.

## Fixtures and proof boundary

Rows reference stable identities at
`local-274` and `local-289`, resolved by JSON pointers into
`docs/compat/fixture-inputs.json`. Fixture inventory/readiness is
owned by the orchestrator; this catalog task does not fabricate availability.
Every row requires a post-seed, post-Start script-caused inventory, XP, world or
lifecycle observation. Loader registration, transpilation, startup, queued
sends, and fixture seeding do not satisfy a functional row.

The settings arrays are exactly what the current loader emits. Some source
schemas refer to computed/imported option constants that the static settings
parser does not expand; an empty emitted `options` array is not permission to
remove the source branch. The row's branch contract keeps every declared,
default and conditional source behavior in scope unless the matrix names an
explicit deferred branch.

## Step 1 verification receipt

Actual loader exports for both source versions and command receipts are retained
under `evidence/catalog-inputs/`. The parent orchestrator completed the saved
inventory after the Sol worker stalled during context compression. Its final
API-member scan needed one closure capture correction; the failed compilation
log is retained. After that correction, both exports and the 180-row source
hash, fixed-set and fixture-reference validation passed. This is support-tool
validation, not script startup or live acceptance. The member scan records
lexical `Binding.member` uses; source files remain the authority for dynamic
access and imported/computed option values.
