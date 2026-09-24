# 274bot

Rust bot host for the 274/289 client. Catalog scripts written for another runtime should still run here so that writing effort is not wasted. This host keeps its own verbs; it does not become that runtime.

## Language

### Host and revisions

**Host verb**:
A snapshot field or `Interactions` op this product already owns (scene op, IF, held/use-on, walk, nav, continue). Gaps are filled here first.
_Avoid_: packet, their adapter, isolate primitive

**Production revision**:
289, the revision the product is built, tested and claimed for. 274 is best-effort: it may run, but it is neither tested nor claimed.
_Avoid_: calling 274 supported, "both revisions" as a release claim

**Selected facts**:
Game facts extracted from the selected revision's content by the one generator, with provenance. What content cannot supply stays unknown.
_Avoid_: frozen tables copied from the catalog, invented coordinates or names, unknown reported as zero or empty

**Coordinate table**:
Host-owned tiles/names in Rust for a fact selected content cannot supply. Small, and a last resort after **Selected facts**.
_Avoid_: empty `[]` forever; copying `api/ai/quests/defs`; gathering camp tables that exist only to run GatheringBot

### Script layers

**Rust-native script**:
A script compiled into the host that drives host verbs directly, with no isolate. Sherlock is the first.
_Avoid_: compiled card as a JS wrapper, "native" meaning JS API v2

**JS API v1**:
The isolate surface that remaps `@rs2b0t/api` onto host verbs so catalog scripts run unmodified. Also called Compat.
_Avoid_: calling this the native API; filling catalog planners in JS

**JS API v2**:
The typed JS surface of this host's own verbs, for scripts written against 274bot rather than rs2b0t. It mirrors the host; it is not a layer over v1.
_Avoid_: Host JS (old name); cloning `rs2b0t-api/index.d.ts` names; a translation layer on top of v1

**Name map**:
A few JS lines that bind a catalog name onto a host verb, a step machine or `not impl`. Compatibility without their control flow. Anything the script needs that touches the client or nav is a host verb, not a cloned `api/*.ts`.
_Avoid_: shim port, facade clone, copying the reference module “for shape”; putting walk, bank, combat, clue or acquire policy in JS

**Isolate**:
The V8 runtime that loads catalog and v2 JS, one per bot. A means to run scripts. Not the product, not the architecture, not a place to reimplement their bot.
_Avoid_: kernel, their runtime, EventBus

**Isolate wire**:
The FlatBuffer that carries snapshots and actions between an isolate and the host. The only such wire.
_Avoid_: JSON RPC, a second host transport

**Typed local helper**:
A bounded synchronous Rust calculation called from the isolate with typed values. It computes; it never sends a game action or talks to the host.
_Avoid_: helper that echoes the snapshot back to Rust, helper as an action path

**Step machine**:
A per-isolate Rust state machine that owns a multi-tick behavior (a clue trail, a hunt, a bank trip). JS starts it and awaits its result; Rust owns the loop, the sequencing and the retries.
_Avoid_: JS driving begin/next loops with its own policy, a generic task framework

### Catalog

**Catalog script**:
A LoopingBot (or TaskBot) written against the frozen rs2b0t names. The compatibility goal is that it still does the skill here.
_Avoid_: honest script port, overlay, rewrite

**Declared ABI**:
The runtime names in the published rs2b0t-api `index.d.ts` (exports and members, not `@internal`). Catch-up maps those names onto host verbs or `not impl`.
_Avoid_: their `src/bot/api` bodies, `abi.ts`, cloning the reference tree

**Rewrite** vs **port**:
A rewrite does the operator-facing job on this host's verbs. A port transplants the catalog's method (control flow, routers, quest `defs/`). Whales (GatheringBot, AIOQuester, ClueSolver) and quest-def cards (ArravSupplier, Barcrawl, RoguesPurse) are later rewrites or stay dim. A coordinate table in Rust is not a port.
_Avoid_: honest script port, cloning QuestEngine / `defs/` / WalkExecutor bodies / ToolAcquire JS / gathering god-class

**World-port**:
Their runtime we will not become: quest `defs/`, GatheringBot, WalkExecutor (Traveller is nav), ToolAcquire JS, MarketMaker ledger, webwalk A*.
_Avoid_: calling a coordinate table a world-port; calling `lightFire` (tinderbox use-on logs) a world-port

**Unloadable**:
A catalog card the picker will not Start: missing remap, or a locked dim name (GatheringBot rows Woodcutter/Miner/Fisher, AIOQuester, ClueSolver, ArravSupplier, Barcrawl, RoguesPurse). Catalog WalkTo is reserved (host nav), never a card. Dimmed.
_Avoid_: dimming because a mapped member throws `not impl`; keeping CookBot dim for lack of a `Firemaking.js` URL

**`not impl`**:
The honest miss prefix: `not impl: <throw reason>`. Unmapped: `not impl: Game.teleport`. Mapped close: `not impl: Game.setCombatStyle: combat_styles empty`. Better than a fake `true`. The throw list is the ledger for a later host-verb campaign.
_Avoid_: stub that returns success, schema default as live state, Chebyshev-as-nav; using `not v1` for a missing host verb

**`not v1`**:
Reserved for a catalog script not written against the v1 rs2b0t API (JS API v2 is that later surface). Not a missing-verb throw.
_Avoid_: unmapped name, mapped close, missing host verb

**Predicate**:
A boolean (or a posted number) a TaskBot `validate()` calls every tick — HP gate, loot-slot bank gate, food-count restock — with no item/spell/weapon table. Legal to name-map even when rs2b0t has the same one-liner (`shouldPanic`, `shouldBank`, `safeToSteal`).
_Avoid_: policy table, cloned `defs/`, hostility list, gather camp, calling this a second client API

**Policy table**:
Loot / hostility / weapon / spell / camp data that would decide *what* to do, not *whether* a posted fact holds. Stays `not impl` or empty (`COMMON_BANK_LOOT`, `HOSTILE_NAMES`).
_Avoid_: predicate, SETTINGS option keys (`FOOD_OPTIONS` / `SPELL_DB` stub names)

**WalkTo** vs **walkResilient**:
`Traversal.walkTo` is the scene packet (`Interactions::walk` / `try_move`). `Traversal.walkResilient` is packed nav (`Traveller` / `ScriptWalkArm`) with `FindOptions.allow_teleports` default off, force-on via `useTeleportCatalog` / `policy.useTeleports`. A Chebyshev walk packet for `walkResilient` is a compat break. A walk that never reaches Traveller is a **wire / name-map** miss. Traveller returning unreachable is **host nav**.
_Avoid_: mapping both to `op: walk`; mapping `walkTo` onto Traveller; calling every failed walk host nav

### Rewrites

**Clue machine**:
The one step machine that solves a held clue. Sherlock, the embedded SolveClue callers and JS API v2 all use it. A clue the catalog marks unreachable stays unreachable.
_Avoid_: a JS solver, thickening SolveClue in JS, a second clue implementation

**Sherlock**:
The Rust-native script that runs clue trails on the clue machine. It replaces catalog ClueSolver, which stays dim.
_Avoid_: ClueSolver, lighting ClueSolver

**Quester**:
The Rust-native script that will drive a Path on host verbs. Not the whale catalog card `AIOQuester`. A later rewrite.
_Avoid_: QuestEngine, AIOQuester, honest script port

**Gatherer**:
The Rust-native script that will replace catalog GatheringBot. Woodcutter / Miner / Fisher stay dim until then.
_Avoid_: GatheringBot, lighting dim gather cards, cloning camp tables

**Path**:
One authored quest document for the Quester (`id` is the engine stem; `display_name` is UI-only).
_Avoid_: QuestRoot, friendly name as a code key, `cooks_assistant` as the id

**Sequence**:
A linear bucket of Steps keyed by a named quest varp's value. There is no FFXIV journal Sequence byte.
_Avoid_: DAG, step graph, FFXIV Sequence

**Step**:
One tagged action on a Path, mapped onto a host verb (or composed from owned ops).
_Avoid_: ITask, QuestEngine step, named mill/cow/egg verbs

**Skip-if**:
A snapshot predicate that treats a Step as already done (inventory, tile, chat, varp). How a later Start resumes.
_Avoid_: SkipConditions as their type, varp-only progress

**Progress**:
The live cursor: current Path, varp bucket, local step index.
_Avoid_: QuestWork, their Sequence byte

**RequiredStats**:
Skill minima that must hold before Start (Path) or before a Sequence. A gate. The Quester does not train.
_Avoid_: TestedStats as a requirement

**TestedStats**:
Optional combat floor recorded from a live scenario PASS. Not a gate. Live below it is a UI warning, not Stop.
_Avoid_: treating it as RequiredStats, padding with setstat

**Acquire**:
How a Path obtains an item: inventory, then bank when that check exists, then gather/collect from server-content spawns (loc/obj/npc), then shop last.
_Avoid_: shop-first, hardcoded tiles when content has a spawn, mining/gather as a named mill-only verb

### Families

**Shop**:
The host verb for a shop modal's stock (fail-closed family + `buy(name, qty)`). Not player trade.
_Avoid_: TradeView as shop, faking stock, shop policy in JSON, catalog `Shop.sell` / `buyById` as host ops

**Trade**:
The host family for player-to-player trade (`TradeView`, baked `TRADEMAIN`). Catalog `Trade.*` name-maps here.
_Avoid_: using this for NPC shops
