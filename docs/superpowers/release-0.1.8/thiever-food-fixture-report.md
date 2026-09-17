# Thiever food fixture

## Scope

The `thiever` scenario now selects the existing `Memory food` loadout and injects the supported ThievingBot settings:

- `loadout`: `Memory food` (the loadout carries `Lobster`)
- `banking`: `Auto`
- `foodWithdraw`: `22`
- `bankAtFood`: `3`
- `loot`: empty

The native offline `thiever` preset writes 10 Lobsters in inventory and 200 Lobsters in the bank. The initial 10-food prerequisite remains intentional: it forces the live script to eat before its first replenishment, rather than passing with setup alone. No catalog, JS shim, host API, operator save, or operator configuration is changed.

## Preparation

From the host checkout, with the isolated production engine root available:

    CARGO_TARGET_DIR=/Users/acfrazier/experiments/274bot/target cargo run --locked -p panel --bin panel-play -- --prepare-fixture thiever --profile local-289 --fixture-path /tmp/274bot-r018-thiever-food.receipt.json --server-root /tmp/274bot-r018-engine.aYl8li

Preparation must finish successfully and produce the receipt plus the account `.sav`; do not overwrite `/tmp/274bot-r018-thiever.receipt.json`.

## Production run

After preparation and the independent headed capture task are accepted, run the isolated production engine and then:

    BOT_PLAYERS_DIR=/tmp/274bot-r018-engine.aYl8li/data/players RS2B0T=/Users/acfrazier/experiments/274bot/.worktrees/release-0.1.8/.superpowers/release-0.1.8/reference/rs2b0t-beecd9126b BOT_NAV_SNAPSHOT_ROOT=$HOME/.274bot/unpack-289 BUDGET_S=1200 BOT_CPU=1 BOT_DEBUG=1 /Users/acfrazier/experiments/274bot/target/release/panel-play --profile local-289 --engine /tmp/274bot-r018-engine.aYl8li --host 127.0.0.1 --port 45594 --asset-host 127.0.0.1 --http-port 1180 --cache /tmp/274bot-r018-engine.aYl8li/data/pack/client --content /Users/acfrazier/experiments/lostcity-289/content --run-prepared --live script_thiever --fixture-path /tmp/274bot-r018-thiever-food.receipt.json

## Acceptance evidence

A setup-only pass is invalid. The run must show, in native readback/log evidence:

1. Lobster inventory count decreases due to a script `Eat` action.
2. Effective hitpoints increase after that same eating action.
3. Thieving XP continues after eating.
4. When food reaches the bank threshold, `Auto` banking deposits non-food items, withdraws Lobsters from the prepared bank stock, closes the bank, and returns to the Guard stand.
5. Genuine Guard pickpocket activity continues for 20 minutes, with a native final readback (ingame, scene state, inventory/HP, bank state where applicable, and Thieving XP).

The fixture configuration itself is not a PASS; the root task owns the matched LIVE run and final milestone claim.
