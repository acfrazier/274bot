# Excerpts — Ardy cake/pickpocket first owner (frozen 4cc6cc27)

Read-only. Paths are under `.superpowers/review-exports/catalog-root-4cc6cc27`
unless noted as catalog inputs.

## Shim stealCakes (unfiltered nearest, no options)

`crates/script/src/shim/cake_stall.js`

```
export async function stealCakes() {
    const loc = Locs.query().nearest();
    if (!loc || String(loc.name).toLowerCase() !== STALL_NAME.toLowerCase()) return false;
    return loc.interact(STALL_OP);
}
```

## Shim tiles (all 2661,3301) and classifySteal stub

`crates/script/src/shim/cake_stall_data.js`

```
export const STALL_TILE = new Tile(2661, 3301, 0);
export const STAND = new Tile(2661, 3301, 0);
export const STAND_ALT = new Tile(2661, 3301, 0);
export const FLEE_TILE = new Tile(2661, 3301, 0);
export const STALL_NAME = "Baker's stall";
export const STALL_OP = 'Steal from';
export function classifySteal() {
    throw notImpl('cakeStallData.classifySteal');
}
```

## TaskBot validate-first

`crates/script/src/shim/mod.rs` prelude

```
globalThis.TaskBot = class TaskBot extends globalThis.LoopingBot {
    async loop() {
        for (const task of this._tasks) {
            if (task.validate()) {
                await task.execute();
                return;
            }
        }
    }
};
```

## Catalog stealCakes call (Cakes)

`.superpowers/inputs/rs2b0t-100adccc…/src/bot/scripts/ArdyCakes/ArdyCakes.ts`

```
class StealCakes {
    validate(): boolean {
        return nearMarket() && !Game.inCombat() && !Inventory.isFull() && !this.bot.died;
    }
    async execute(): Promise<void> {
        const result = await stealCakes({ fillTo: 28, abort, shouldEat, lockedOutUntil, setStatus, log, onSteal, onReset });
        if (result === 'combat') { … }
        else if (result === 'no-progress') { … }
    }
}
```

## Catalog restock (Thiever) before Pickpocket

`ArdyThiever.ts`

```
class RestockCakes {
    validate(): boolean {
        return nearMarket() && !this.bot.inRealCombat() && !Inventory.isFull() && foodCount() <= RESTOCK_AT;
    }
    async execute(): Promise<void> {
        this.bot.log(`restocking cake (have ${foodCount()})`);
        await stealCakes({ fillTo: FOOD_TARGET, abort, shouldEat, setStatus, log, onSteal });
    }
}
```

## Catalog cakeStallData (not the shim)

```
STALL_TILE (2667,3310,0)
STAND      (2668,3312,0)
STAND_ALT  (2669,3310,0)
FLEE_TILE  (2655,3298,0)
```

## LIVE start / FAIL (old catalog, 4cc6cc27)

Cakes both revs: `ArdyCakes starting — stand (2661, 3301, 0)` then no steal log.
FAIL tile `[2668,3312,0]`, empty inv, `stat_xp_gain(17)>=1` 150 ticks.

Thiever both revs / both targets: `restocking cake (have 0)` 148–149 times.
FAIL tile `[2661,3306,0]`, empty inv, same XP watch.

## Existing native loc op (do not reimplement)

`locs.js` queues `{ op: 'loc', x, z, level, action, id }`.
`host-play/src/lib.rs` `InteractReq::Loc` matches tile + optional id.

## Engine stall (274 and 289)

`[bakers_stall_stealing] name=Baker's stall op2=Steal from`
