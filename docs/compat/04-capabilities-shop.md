# Observed shop buy and sell transfers

## Candidate

Task `t_1591d140` implements brief 38 on `codex/rs2b0t-multirevision`. Client
gitlink is unchanged by this card (`52c37f9ce50d1f184656d5b4469c007ec8a5791a`,
the pin inherited from the parent's approved brief 37). No live client was
launched. Root owns LIVE, frontend/scenario/ledger, gitlink and repository
integration.

## Frozen semantics this card implements

The compatibility target is the imported script's own module
(`RS2B0T 100adccc` `src/bot/api/shop/Shop.ts`), read from the frozen input at
`.superpowers/inputs/rs2b0t-100adccc.../`:

| Frozen call | Frozen behavior | Host implementation |
|---|---|---|
| `Shop.open(npcName)` | already open → `true`; otherwise 3 attempts, one `Trade` press per attempt, `delayUntil(isOpen, 3000)` each; absent keeper → `false` with no press | Rust `script::shop` owns the 3 attempts and the 3000 ms window per attempt; JS presses nothing itself |
| `Shop.buy(name, n)` | `shopOpBatch(ops, 'buy', n)` = the 10/5/1 decomposition capped at 5 user-event packets, `delayUntil(count moved, 3000)` then `delayTicks(1)`, returns the observed count | Rust plans the batch, bounds it to `MAX_PACKETS_PER_TICK = 5`, waits `SETTLE_MS = 3000`, settles exactly one posted tick and reports the observed delta |
| `Shop.sell(name, n)` | same batching over `shop_template_side:inv` (3823) and counts the inventory loss | Rust resolves the row in the posted player pack only; losing that container stops the transfer (`player-missing`) |
| `Shop.close()` | `closeModal` then `delayUntil(!isOpen, 3000)`, resolves void | Rust sends one close verb, waits `CLOSE_WAIT_MS = 3000`; the shim resolves `undefined` for close |

A queued batch is never a transfer: the count only moves on the observed
container counts, and the frozen one-tick settlement after the 3000 ms window
is preserved (`Phase::SettleTick`).

## Facts and provenance

Both selected caches decode the same shop interfaces and their fixed op slots.
Evidence: `docs/compat/evidence/shop-capabilities/iface-274.txt` and
`iface-289.txt` (raw decode output, engine dirs named in the files), plus the
committed test `crates/api/tests/snapshot.rs::packed_shop_interfaces_post_main_stock_and_player_pack`
which reads the local client jag through `client::cache_dir()`.

| Interface | 274 cache | 289 cache |
|---|---|---|
| `shop_template` 3824 | TYPE_LAYER, layer 3824, children 3825–3916 | identical |
| `shop_template:inv` 3900 | TYPE_INV, layer 3824, `iop[1..4]` = Buy 1 / Buy 5 / Buy 10 | identical |
| `shop_template_side` 3822 | TYPE_LAYER, layer 3822, children `[3823]` | identical |
| `shop_template_side:inv` 3823 | TYPE_INV, layer 3822, `iop[1..4]` = Sell 1 / Sell 5 / Sell 10 | identical |
| jag component count | 10984 | 11942 |

Because both caches expose exactly Buy/Sell 1/5/10 at `iop[1..4]`, the frozen
`shopOpBatch` op-label filter reduces to those three steps on the enabled path;
the fixed 10/5/1 plan plus the per-tick bound is therefore the whole policy, and
the host never sends a generic if-button at a stock component nor answers a
count dialog (the count dialog branch was removed from `Interactions::shop_buy`).

## Ownership

- Rust owns all sequencing and policy: `crates/script/src/shop.rs` (new)
  holds the phases, both deadlines, the batch plan, the settled-count
  accounting, the open/close attempts and the Pause/Guardian-hold/Stop state.
- JavaScript stays the thin facade `crates/script/src/shim/shop.js`: it marshals
  the caller's arguments once (`begin`), dispatches the verbs the runtime
  returns (`npc`, `close-modal`, `ops`), awaits `next` and reports the observed
  completion. There is no `shopOpBatch` copy and no row selection in JS.
- The queued op is `shop-button { kind, name, id, slot, component, chunk }`: the
  exact posted row identity, which the host re-resolves in the posted container
  and refuses (`StaleTarget`) if it is gone. Same-name fallback does not exist.
- Sell reads `ItemContainer::ShopPlayer` (3823) and Buy `ShopStock` (3900);
  neither reads the backpack. `shop_player_available` separates "not decoded
  this rebuild" (Sell fails closed) from "decoded and empty".
- Bulk snapshot roundtrips are unchanged: the runtime keeps only a compact
  projection of changed snapshot fields (`NativeObservation`) and iterates the
  posted rows once.

## Callers

`ShopBuyout` (this card's enabled branch) calls `Shop.open(keeper)`,
`Shop.buy(want.name, want.units)` and `Shop.close()`. `Shop.sell` has no
ShopBuyout caller but is completed here for the shop-owned paths; `Shop.buyById`
keeps its explicit `not impl` because no enabled branch calls it.

## Hotspot

`crates/script/src/load.rs` — composition only: `__rs2b0t_shop` register,
`shop_player` materialization, and the snapshot/hold/pause/resume/reset hooks.
`crates/script/src/isolate_fb.rs` — snapshot field readers/writers/fingerprint/
delta mask plus the `shop-button` interact row encode/decode.
`crates/script/src/host_js.rs` + `crates/script/host-js/index.d.ts` — declared
`InteractReq` union regenerated for the new op.

## Verification

- `cargo test -p script --lib shop`: 13 passed (plan/10-5-1 bound, exact row
  lookup, Trade_target, open attempts and window, open success, queued batch is
  not a transfer, settle-tick continuation, sell on the player pack, losing the
  pack, depleted stock partial count, Pause/hold freeze, abort token).
- `cargo test -p script --test shop`: 9 passed (composed through the shim:
  closed-shop refusal, 10/5/1 batch then settled 25, sell on 3823 not the
  same-name stock row, unpublished pack fails closed, pick unavailable, open
  press then opened shop, absent keeper false, shop closed mid-batch, Pause/hold
  and session reset).
- `cargo test -p script --test load_isolate shop`: 6 passed (including the two
  updated shim mappings: the exact posted shop row and the 5+1+1 batch).
- `cargo test -p api --test interact shop`: 4 passed (Buy 1/5/10 slots at 3900
  rows, Sell slots at 3823, refusals for non-fixed chunks and stale rows, no
  sends without an open shop or a posted pack).
- `cargo test -p api --test snapshot shop`: 4 passed (packed both-cache
  interfaces, side-modal pack posting and its fail-closed absence).
- `cargo test -p script --test host_js`: golden `index.d.ts` regenerated and not
  stale.
- `cargo clippy`, `rustfmt --check` and `git diff --check` on the owned paths.
- Not LIVE: root owns the headed/live cells.

## Limits

- Not LIVE. No client, frontend, profile, engine or fixture edits.
- `Shop.buyById` stays unavailable (no enabled caller in the current audit).
- `Shop.sell`'s optional `pick` callback stays an explicit `not impl`: the
  caller's row selector cannot be evaluated without moving row selection into
  JavaScript. The only frozen caller that passes one is the Nature island branch
  (`NatureCrafter.ts`), which is not this card's enabled branch; the default
  (first matching row in the shop's player pack) is implemented.
- Trade boundaries are untouched: this card adds no Trade controller and the
  existing trade tests still pass.
