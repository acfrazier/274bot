# Login: FIFO throttle numbers

`crates/host/src/login_queue.rs` stays under Lost City's **production**
login rate limits. FIFO identity is a process-unique slot owner, while device
rate accounting remains keyed by UID. Only the slot thread creates membership
at its Queueing transition, then polls its owner token for `Permit::Grant` or
`Permit::Wait(duration)`. Login-all records non-membership order hints before
it exposes each new login intent; a later worker cannot overtake an earlier
hinted owner, and an online or mid-handshake slot cannot leave a ghost place.

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
| per-IP window | **29 attempts / 60 s idle** | production threshold is 30 and rejects the attempt that reaches it; pending reservations count |
| per-uid cap | **4 attempts, then remaining of 15 s idle** | production device threshold is 5 (`>= 5` rejects); partial counts expire after the same idle TTL |
| backoff (response 16, attempts exceeded) | **20 s + 45 s per prior hit** | shared across the wall; `LoginBackoff::reset()` clears per-slot escalation |

Defaults are `LoginQueue::default()`; `new(spacing, ip_cap, ip_window)`
exists for tests and rejects a zero address cap. A blocked requester waits
the longest unmet spacing, shared-throttle, per-IP, or per-uid constraint.

## Backoff

`LoginBackoff` delays retries after response 16 (“Login attempts exceeded”):
first retry 20 s, then 65 s, 110 s, … (`20 + 45·hits`). A response-16 hold
is also published to the shared queue so sibling slots pause. Retry waits run
to their deadline and exit early only for Stop, login withdrawal, an
intentional-logout latch, or a changed world selection. Generic focus, render,
script, and panel wakes do not shorten them. Any successful login resets the
slot's escalation.

## Queue position and leaving

While a slot waits it sits on the FIFO. `LoginQueue::status_owner(owner)`
returns its place as `Option<QueuePos { position: u32, total: u32 }>` — the
**k of n** snapshot (1-based; a granted owner is popped and no longer
present). Two slots with the same UID therefore keep independent places while
sharing conservative device-attempt accounting. host-play publishes both
fields atomically with owner membership; an absent owner always clears its
own row. The panel renders only the focused slot's valid
`1 <= position <= total` tuple, so a connected slot never inherits another
slot's card. Withdrawal, terminal startup failure, rail removal, and Stop
clear both owner membership and status.

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

Every host-owned socket attempt is preceded by a shared permit, including
opcode-18 reconnects and a retry after response 1. In external-ownership mode
the embedded client returns those intents to host-play instead of reconnecting
or recursively retrying internally; standalone clients retain their
Java-compatible response-1 retry. A granted attempt is acknowledged on
success, error, or unwind; an unused grant is abandoned before any socket
call. Host-play keeps transient response 1 in Connecting rather than
publishing phase Error.


## Panel: Login all vs auto-login

The panel arms logins through `SlotArm` flags (host-play), not
`api::interact::login` directly. Two intents differ:

- **Login all / Log in** is a **one-shot**: it clears the member's logout
  latch, cancels any pending logout, and arms the handshake. Once the grant
  lands the arm disarms, so an unexpected DC leaves the slot on the title
  until the next explicit arm.
- **Auto-login** (General config → **slot**, **auto-login on title**, backed
  by `ProfileSettings.auto_login`, default **off**) records the intent's
  provenance. Turning it on arms an unlatched parked slot only when no
  explicit intent is already active; turning it off withdraws only
  auto-derived intent, including during preparation/backoff. An explicit
  **Log in** survives an auto on→off toggle. An explicit **Logout / Logout
  all** latches the member until the next **Login all** clears it.
