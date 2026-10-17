# Login: FIFO, pacing and throttle numbers

`crates/host/src/login_queue.rs` admits **one login handshake at a time**,
spaces login starts, and stays under Lost City's **production** login rate
limits. FIFO identity is a process-unique slot owner, while device rate
accounting remains keyed by UID. Only a slot thread creates membership when
it reaches Queueing. Workers enter in arrival order; the focused/preferred
owner is the sole exception, entering at the front or moving there if already
queued. Focus never bypasses the in-flight cap, the spacing or the rate limits.
Login all, Log in, and auto-login changes only arm intent and wake workers.
They create no membership, record no order hints, and no absent owner can gate
the head.

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
| `src/engine/World.ts` | `processLogins` admits new players once per 600 ms world tick (`TICKRATE`, `:120`; called from the tick at `:384`; sends response 2 at `:809-957`). |
| `src/server/tcp/TcpServer.ts` | `s.setTimeout(30000)` (`:19`): a socket silent for 30 s is destroyed (`:61-67`). The original client's read timeout is also 30 s (`javaclient/.../jagex2/io/ClientStream.java:47`). |

Local default is `production: false` (`NODE_PRODUCTION`). A loopback engine
does **not** apply these counters. The host still stays under the production
numbers so flipping production on does not 16 the wall.

The engine has **no** inter-login spacing rule. The host's 1.2 s spacing is
its own policy, not a protocol requirement. Spacing alone would not keep two
attempts in different admission ticks (network delay can bunch or split
them). The one-in-flight rule does that: a successful handshake returns only
after `processLogins` answered it, so the next attempt starts after that tick.
The spacing only keeps fast handshakes (about 0.4 s on a local engine) from
following each other back to back.

## Constants (the numbers agents must respect)

| Rule | Value | Meaning |
| --- | --- | --- |
| handshakes in flight | **1**, for at most **30 s + 1 s** | the next login starts only after the previous `client.login` returned (or its unused grant was given back); scene loading and the response-21 countdown do not hold it. The client stops the connect, every read (also of data already buffered) and every write of the handshake 30 s (`HANDSHAKE_HOLD`) after the attempt started, however much WebSocket control traffic arrives; handshake writes go to the socket on the caller's thread, each armed with the time left. The 30 s is host policy matching the 30 s read timeouts above, which bound single reads, not a whole handshake. The queue frees the slot 1 s (`LEASE_SLACK`, host policy: time for the client to report its own deadline failure) later even if the call never returns: it first shuts that attempt's socket both ways (`LoginCancel`), so the server reads end of stream before the next login can be granted, and counts the attempt as returned then |
| start spacing | **1.2 s** (host policy) | measured between attempt starts (the moment each connect begins), not grants; not-head polls every **20 ms** |
| per-IP window | **29 attempts / 60 s idle** | production threshold is 30 and rejects the attempt that reaches it; pending reservations count |
| per-uid cap | **4 attempts, then remaining of 15 s idle** | production device threshold is 5 (`>= 5` rejects); partial counts expire after the same idle TTL |
| backoff (response 16, attempts exceeded) | **20 s + 45 s per prior hit** | shared across the wall, published in the same queue update that frees the in-flight slot; `LoginBackoff::reset()` clears per-slot escalation |
| backoff (connection failure, response 7) | **5, 10, 20, 40, then 60 s**, each ×0.8–1.2 jitter, never above 60 s | per slot; 60 s is the engine's address-attempt TTL |

Defaults are `LoginQueue::default()`; `new(spacing, ip_cap, ip_window)`
exists for tests and rejects a zero address cap. A clean exit keeps its owner's
FIFO place but defers the first relog by the normal 1.2 s spacing. If the
server still replies code 5 (account in use), the owner retries every 5 s
within a 30 s budget measured from the observed clean exit; after that, normal
code-5 backoff resumes. Other eligible owners may proceed during this release
wait.

An owner blocked only by its own per-UID device cooldown also keeps its place
and yields to owners on other UIDs; owners sharing that UID wait for the
cooldown. Otherwise, a requester waits behind the first eligible owner or for
the longest unmet in-flight, spacing, shared-throttle, or per-IP constraint.
There is no separate hint wait.

`LoginQueue::head_blocker(owner, now)` names the eligible head's admission
blocker; for an owner passed because its own device cooldown is the selected
blocker, it reports that cooldown instead. `AdmitBlocker` covers another login
in flight, start spacing, address cooldown with its attempt count, device
limit, or a response-16 hold, with its deadline when known. host-play publishes
it on the slot row next to the place, and `frontend-core` turns it into the one
line both the panel (status and Game image queue card) and the TUI show, e.g.
`waiting in login queue (1/21): next in line, login rate cooldown (29
attempts), next try in 41 s`.

## Backoff

`LoginBackoff` delays generic retries after response 16 (“Login attempts
exceeded”): first retry 20 s, then 65 s, 110 s, … (`20 + 45·hits`). A
response-16 hold is also published to the shared queue, in the same update
that acknowledges the refused login, so no sibling starts in between.
A failed connection (the client's code -1, see below) and a full world
(response 7) use the slot's escalating jittered delay instead of a fixed 5 s.
These generic retry waits run to their deadline and exit early only for Stop,
login withdrawal, an intentional-logout latch, or a changed world selection.
Generic focus, render, script, and panel wakes do not shorten them. Any
successful login resets the slot's escalation.

## Failed connections

A login that fails before the server answers keeps its cause: the handshake
stage (`connect`, `send_hello`, `read_hello`, `read_seed`, `send_login`,
`read_response`, `read_transfer_delay`, `read_accept`), the I/O error kind, the
OS error number when there is one, and the bounded error text
(`LoginError::transport`). A server that hangs up instead of answering is an
`UnexpectedEof` at the stage it hung up in, and one that stops answering is a
`TimedOut` (`login handshake deadline passed`) at the stage it reached when
the handshake deadline passed. The slot row shows
`Error connecting to server. (connect: Connection refused (os error 111))`
rather than `code -1`, and the session log adds `stage=… kind=… errno=…`.

## Response 7 (world full)

In the engine, response 7 means the world has no room: more than
`maxConnected` players (`World.ts:2227-2229`) or no free player slot
(`World.ts:883-888`). The login server's own refusals (including its "safety"
and IP-ban replies) reach the client as 11, not 7 (`World.ts:1873-1877`).
Each refusal logs one line with the world, its endpoint, Auto or pinned, and
the next attempt, e.g. `world full (code 7) on w2 w2.rs2b2t.com:443 (pinned
world); next attempt w2 w2.rs2b2t.com:443 in 9.6 s`. An Auto profile moves on
to the next world at once (back through the paced queue) and waits only after
every world refused it in one round; a pinned profile keeps its world and
waits; both waits escalate as above.

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
rs2b2t roster (w1, w2 and w3 on port 443 by default). `public-289` is a profile
name alias; `--prod` and `BOT_TARGET` were removed.
The shared cache is fetched from the first reachable world. If 274bot cannot
reach a world's cache server, or cannot look up the name of a world's
server, while it prepares the cache, it moves to the next world as a whole,
and auto accounts start on that world. It skips a world whose server name
already failed to look up. Each vault account stores
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
Java-compatible response-1 retry. The worker owns the grant from the polling
call that granted it (`GrantedReservation`): an unused grant is given back on
any return or unwind before the socket call, and a started attempt is
acknowledged on success, error, or unwind. A grant whose lease ran out before
its attempt started cannot start; the slot queues again. Host-play keeps
transient response 1 in Connecting rather than publishing phase Error.


## Panel: Login all vs auto-login

The panel arms logins through `SlotArm` flags (host-play), not
`api::interact::login` directly. Two intents differ:

- **Login all / Log in** is a **one-shot**: it clears the member's logout
  latch and repeat-logout history, cancels any pending logout, and arms the
  handshake. Once the grant lands the one-shot disarms. A running or paused
  script still relogs after an unexpected disconnect; a slot without a script
  then follows its saved auto-login setting.
- **Auto-login** (the Profiles edit form's **Log in automatically**, backed
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
