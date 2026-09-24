# Login: FIFO throttle numbers

`crates/host/src/login_queue.rs` stays under Lost City's **production**
login rate limits. `LoginQueue::request_permit(uid, now)` returns
`Permit::Grant` or `Permit::Wait(duration)` — retry after `duration`. Only
the FIFO head may be granted.

A process binds one **server profile** (`local-274`, `local-289`,
`public-289`) before sockets open. Profile defaults (ports, vault path,
engine root) are documented in [README.md](../../README.md). This page is
the FIFO and handshake policy shared by every profile.

## Where the server stores this

In the engine checkout (`$ENGINE_DIR`, default `$HOME/experiments/Server/engine`):

| File | What |
| --- | --- |
| `src/util/WorldConfig.ts` | defaults `rateLimitAddressLogin: 30`, `rateLimitDeviceLogin: 5`; `NODE_RATELIMIT_ADDRESS_LOGIN` / `NODE_RATELIMIT_DEVICE_LOGIN` override |
| `src/engine/World.ts` | `loginAddressAttempts` TTL **60 s**, `loginDeviceAttempts` TTL **15 s**. Counters increment on opcode 14 (address) and 16/18 (device = `uid@ip`). **`>=` the cap sends response 16.** Both run **only** when `node.production` is true. |

Local default is `production: false` (`NODE_PRODUCTION`). A loopback engine
does **not** apply these counters. The host still stays under the production
numbers so flipping production on does not 16 the wall.

There is **no** inter-grant spacing in the engine. A previous 2.5 s host
gap was invented (rs2b0t used 1 s); it is not a server default.

## Constants (the numbers agents must respect)

| Rule | Value | Meaning |
| --- | --- | --- |
| spacing | **0** | engine has none; not-head polls every **20 ms** |
| per-IP window | **30 grants / 60 s** | production `rateLimitAddressLogin` + address TTL |
| per-uid cap | **4 grants, then remaining of 15 s** | production device cap is 5 (`>= 5` rejects); stay under with 4, cooldown = device TTL from the latest grant |
| backoff (response 16, world full) | **20 s + 45 s per prior hit** | `LoginBackoff::delay()` escalates; `reset()` clears |

Defaults are `LoginQueue::default()`; `new(spacing, ip_cap, ip_window)`
exists for tests. A blocked requester waits the longest unmet constraint:
the per-IP window roll-off or the per-uid cooldown.

## Backoff

`LoginBackoff` delays retries after a response-16 (world full) rejection:
first retry 20 s, then 65 s, 110 s, … (`20 + 45·hits`). Call `reset()` on
any successful login.

## Queue position and leaving

While a slot waits it sits on the FIFO. `LoginQueue::status(uid)` returns
its place as `Option<QueuePos { position: u32, total: u32 }>` — the **k of n**
snapshot (1-based; a granted uid is popped and no longer present). host-play
mirrors that onto `SlotStatus.queue_position` / `queue_total` while the slot
waits; the panel renders it as **"k of n"** in the status row and as the
queue card over the focused slot. `LoginQueue::leave(uid)` drops a queued
uid (no-op if absent) — the panel's rail ✕ and `stop_slot` call it so a
removed slot does not sit in the FIFO.

## Mainland hop (tutorial skip)

New accounts spawn on Tutorial Island. Two different local-engine paths:

- **host-play** `--mainland` / `BOT_MAINLAND=1`: `api::interact::mainland_hop` after `ingame && scene_state == 2` — cheat body `tele 0,50,50,20,20` then `setvar tutorial 1000` (not a `~` debugproc).
- **panel TutSkip** (loopback only): hidden until `getvar tutorial` says
  the tutorial is still open; press sends `setvar tutorial 1000` and
  caches `tutorial_skipped = Some(true)`. No courtyard tele.

`tele` / `setvar` / `setstat` have **no** tilde. Engine debugprocs are cheat bodies that **start with `~`** (`~home` from the panel Lumbridge button; type `::~name` in chat). Panel capture must pass colon, tilde, and comma.

This does **not** relog. Side icons stay tutorial-locked. A clean logout is already wired: `api::interact::logout` presses the `CC_LOGOUT` iface (client code 205) through the doAction path, so client-code logout vetoes still apply ([interact.md](interact.md)). Local engine grants staff cheats when not `production`.

## RSA (local engine)

No compile-time key bake. Stock Lost City Server uses the **Java default**
public pair; that is the usual local-dev login and needs no env.

If you rotated the engine key, login reads the public half from
`$ENGINE_DIR/data/config/private.pem` (rs2b0t `deploy-local-key.sh`
layout), or from `LOGIN_RSAN` / `LOGIN_RSAE`.

## Public worlds (`public-289`)

`BOT_TARGET=prod` (alias `live`), `host-play --prod`, or
`--profile public-289` uses the ordered endpoints in `~/.274bot/worlds.json`
(created with w1 and w2 on port 443 when absent). An invalid file or a
public endpoint outside that list fails closed. The shared cache is fetched
from the first reachable configured asset world. Each vault account stores
an optional world number: auto rotates on response 7, waits after all
worlds report full, and pinned accounts stay on their chosen world.
The client uses the selected world's node id and fetches its login RSA
modulus from `/client/client.js` (successful fetches cached per host, refreshed
on response 6); a failed fetch uses the baked public modulus without caching
the fallback. The local engine key path above is unchanged. Login and
cache transport remain WSS/HTTPS for public worlds; Cargo `TARGET` remains the
rustc triple.

`$ENGINE_DIR` defaults depend on revision (274:
`$HOME/experiments/Server/engine`; 289:
`$HOME/experiments/lostcity-289/engine`). Cache and nav-pack paths follow
the resolved profile. Prefer an explicit `--profile`. Cargo `TARGET` is
the rustc triple, not a world switch.

## Wiring

`api::interact::login` routes the handshake through the driver
(`Client::login`), which opens a fresh stream per attempt and blocks until
the server responds. The FIFO sits ahead of that handshake: request a permit,
wait the returned `Duration` when throttled, then send.


## Panel: Login all vs auto-login

The panel arms logins through `SlotArm` flags (host-play), not
`api::interact::login` directly. Two intents differ:

- **Login all / Log in** is a **one-shot**: it clears the member's logout
  latch, cancels any pending logout, and arms the handshake. Once the grant
  lands the arm disarms, so an unexpected DC leaves the slot on the title
  until the next explicit arm.
- **Auto-login** (General config → **slot**, **auto-login on title**, backed
  by `ProfileSettings.auto_login`, default **off**) keeps the arm armed
  after a successful handshake, so a DC re-handshakes. An explicit
  **Logout / Logout all** latches the member, which blocks even an
  auto-login slot until the next **Login all** clears the latch.
