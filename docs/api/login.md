# Login: FIFO throttle numbers

`crates/host/src/login_queue.rs` stays under Lost City's **production**
login rate limits. FIFO identity is a process-unique slot owner, while device
rate accounting remains keyed by UID. Only a slot thread creates membership
when it reaches Queueing. Workers enter in arrival order; the focused/preferred
owner is the sole exception, entering at the front or moving there if already
queued. Login all, Log in, and auto-login changes only arm intent and wake
workers. They create no membership, record no order hints, and no absent owner
can gate the head.

A process binds one entry from `~/.274bot/servers.json` before sockets open.
The built-ins are `local-274`, `local-289`, and `rs2b2t`; `public-289` is an
alias. Profile defaults are documented in [README.md](../../README.md). This
page is the FIFO and handshake policy shared by every profile.

## Where the server stores this

In the engine checkout selected for a local profile (`--engine`, `ENGINE_DIR`,
or `login_key.engine_dir` in `~/.274bot/servers.json`):

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
exists for tests and rejects a zero address cap. A requester waits when another
owner is ahead of it or for the longest unmet spacing, shared-throttle, per-IP,
or per-uid constraint. There is no separate hint wait.

## Backoff

`LoginBackoff` delays generic retries after response 16 (“Login attempts
exceeded”): first retry 20 s, then 65 s, 110 s, … (`20 + 45·hits`). A
response-16 hold is also published to the shared queue so sibling slots pause.
These generic retry waits run to their deadline and exit early only for Stop,
login withdrawal, an intentional-logout latch, or a changed world selection.
Generic focus, render, script, and panel wakes do not shorten them. Any
successful login resets the slot's escalation.

## Response 21 transfer cooldown

Response 21 carries a countdown truncated to a single unsigned seconds byte.
For a byte `N`, Java-compatible behavior displays `N, N-1, …, 0` and waits one
second after every display, so the next attempt begins after **N + 1 seconds**;
even a zero byte waits one second. The retry uses the same endpoint and bypasses
generic world-error switching, key refresh, and escalating backoff.

The completed handshake attempt is acknowledged before this wait. The worker
holds neither a FIFO place nor a pending reservation during the countdown, so a
later worker may be granted. Stop, login withdrawal, or an intentional-logout
latch interrupts the countdown. World-selection edits and their wakeups do not;
the selected world is reconsidered only after the server-owned delay expires.

## Queue position and leaving

While a slot waits it sits on the FIFO. `LoginQueue::status_owner(owner)`
returns its place as `Option<QueuePos { position: u32, total: u32 }>` — the
**k of n** snapshot (1-based; a granted owner is popped and no longer
present). Two slots with the same UID therefore keep independent places while
sharing conservative device-attempt accounting. host-play publishes both
fields atomically with owner membership; an absent owner always clears its
own row. Each visible Game image renders only its displayed slot's valid
`1 <= position <= total` tuple, so connected and neighboring previews never
inherit another slot's card. Rail tiles do not draw queue cards. Withdrawal,
terminal startup failure, worker unwind, rail removal, and Stop clear both
owner membership and status.

## Mainland hop (tutorial skip)

New accounts spawn on Tutorial Island. Two different local-engine paths:

- **host-play** `--mainland` / `BOT_MAINLAND=1`: `api::interact::mainland_hop` after `ingame && scene_state == 2` — cheat body `tele 0,50,50,20,20` then `setvar tutorial 1000` (not a `~` debugproc).
- **panel TutSkip** (loopback only): hidden until `getvar tutorial` says
  the tutorial is still open; press sends `setvar tutorial 1000` and
  caches `tutorial_skipped = Some(true)`. No courtyard tele.

`tele` / `setvar` / `setstat` have **no** tilde. Engine debugprocs are cheat bodies that **start with `~`** (`~home` from the panel Lumbridge button; type `::~name` in chat). Panel capture must pass colon, tilde, and comma.

This does **not** relog. Side icons stay tutorial-locked. A clean logout is already wired: `api::interact::logout` presses the `CC_LOGOUT` iface (client code 205) through the doAction path, so client-code logout vetoes still apply ([interact.md](interact.md)). Cheat sends are admitted on local (loopback) profiles; remote profiles refuse them.

## RSA (local engine)

No compile-time key bake. Stock Lost City Server uses the **Java default**
public pair; that is the usual local-dev login and needs no env.

If you rotated the engine key, login reads the public half from
`$ENGINE_DIR/data/config/private.pem` (rs2b0t `deploy-local-key.sh`
layout), or from `LOGIN_RSAN` / `LOGIN_RSAE`.

## Public worlds (`rs2b2t`)

`--rs2b2t`, `--profile rs2b2t`, or `BOT_SERVER_PROFILE=rs2b2t` selects the
rs2b2t roster (w1 and w2 on port 443 by default). `public-289` is a profile
name alias; `--prod` and `BOT_TARGET` were removed.
The shared cache is fetched from the first reachable asset world. Each vault account stores
an optional world number: auto rotates on response 7, waits after all
worlds report full, and pinned accounts stay on their chosen world.
The client uses the selected world's node id and fetches its login RSA
modulus from `/client/client.js` (successful fetches cached per host, refreshed
on response 6); a failed fetch uses the baked public modulus without caching
the fallback. The local engine key path above is unchanged. Login and
cache transport remain WSS/HTTPS for public worlds; Cargo `TARGET` remains the
rustc triple.

A peer WSS Close, TLS failure, or WebSocket protocol error is treated as an
immediate transport loss rather than waiting for the dead-server deadline.
Standalone clients enter the Java-style `lostCon` reconnect path; the same
failure during login reports the ordinary connection error.

Local profiles require an engine root from `--engine`, `ENGINE_DIR`, or the
selected profile's `login_key.engine_dir` in `~/.274bot/servers.json`, in that
order. There is no HOME-relative engine default. Cache and nav-pack paths
follow the resolved profile. Prefer an explicit `--profile`. Cargo `TARGET` is
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
  latch and repeat-logout history, cancels any pending logout, and arms the
  handshake. Once the grant lands the one-shot disarms. A running or paused
  script still relogs after an unexpected disconnect; a slot without a script
  then follows its saved auto-login setting.
- **Auto-login** (General config → **slot**, **auto-login on title**, backed
  by `ProfileSettings.auto_login`, default **off**) records the intent's
  provenance. Turning it on arms an unlatched parked slot only when no
  explicit intent is already active; turning it off withdraws only
  auto-derived intent, including during preparation/backoff. An explicit
  **Log in** survives an auto on→off toggle. An explicit **Logout / Logout
  all** latches the member until the next explicit login clears it.

## Host-owned session liveness

The host owns inactivity policy for every connected slot. The embedded client
does not send its automatic `IDLE_TIMER` request while externally owned; this
does not synthesize mouse or keyboard input and does not add an anti-idle game
packet. `NO_TIMEOUT` is appended after about one wall-clock second without a
successful outbound flush, even when another packet is already queued or an
idle slot is pumped slowly. The successful combined flush starts the next
interval, so fast pumping cannot flood keepalives.

An unexpected server logout, EOF, or transport failure relogs with the normal
queue and backoff while a script is running or paused, independently of the
saved auto-login setting, and preserves that script's in-flight work.
A slot without an active script keeps the saved auto-login behavior. Three
unexpected session exits inside ten minutes trip a repeat guard to avoid a
reconnect storm; the slot status names the guard and the session log records
its count and window. The window uses Rust's monotonic `Instant`; on macOS it
counts awake time, so system sleep does not advance the ten minutes. The
script's work remains held, and **Log in** clears the guard and resumes it.
Operator **Logout**, Stop, and removal remain terminal and are never
automatically resurrected.

## Relog now (memory mode)

Switching lowmem/highmem queues the whole mode for the next login after a
clean logout ([panel.md](panel.md), [tui.md](tui.md)). **Relog now** performs
that logout and login through the ordinary queue above — it is not a shortcut
around it — and warns first when a running script would be interrupted.
